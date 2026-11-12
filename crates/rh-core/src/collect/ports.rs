//! Listening ports: which process holds `:5173`, which directory it runs in,
//! and whether it listens on every interface or only on this machine. All
//! from `/proc/net/{tcp,udp}{,6}` and each process's open sockets.
//!
//! Exposure is the point. A dev server bound to `0.0.0.0` answers anyone on
//! the café Wi-Fi; one bound to `127.0.0.1` answers only this machine.

use super::read;
use crate::model::Port;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

/// One socket from a `/proc/net` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Socket {
    pub addr: IpAddr,
    pub port: u16,
    pub uid: u32,
    pub inode: u64,
}

fn hex_addr(s: &str) -> Option<IpAddr> {
    match s.len() {
        8 => Some(IpAddr::V4(Ipv4Addr::from(
            u32::from_str_radix(s, 16).ok()?.swap_bytes(),
        ))),
        32 => {
            // Four 32-bit words, each in host (little-endian) order.
            let mut b = [0u8; 16];
            for w in 0..4 {
                let v = u32::from_str_radix(&s[w * 8..w * 8 + 8], 16).ok()?;
                b[w * 4..w * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            Some(IpAddr::V6(Ipv6Addr::from(b)))
        }
        _ => None,
    }
}

/// Sockets in `state` (`0A` listening for TCP, `07` for a bound UDP socket)
/// from one `/proc/net` table.
pub fn parse_table(text: &str, state: &str) -> Vec<Socket> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.get(3) != Some(&state) {
                return None;
            }
            let (a, p) = f.get(1)?.split_once(':')?;
            // A UDP socket with a peer is a client, not a service.
            if state == "07" && !f.get(2)?.ends_with(":0000") {
                return None;
            }
            Some(Socket {
                addr: hex_addr(a)?,
                port: u16::from_str_radix(p, 16).ok()?,
                uid: f.get(7)?.parse().ok()?,
                inode: f.get(9)?.parse().ok()?,
            })
        })
        .collect()
}

/// Who can reach an address: `all` (every interface), `loopback` (only this
/// machine) or `address` (one interface's address).
pub fn exposure(addr: IpAddr) -> &'static str {
    let mapped = match addr {
        IpAddr::V6(a) => a.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(addr),
        a => a,
    };
    if mapped.is_unspecified() {
        "all"
    } else if mapped.is_loopback() {
        "loopback"
    } else {
        "address"
    }
}

/// Socket inode to the pid holding it, for the processes we may look into.
pub(crate) fn owners(proc_root: &Path) -> HashMap<u64, u32> {
    let mut out = HashMap::new();
    let Ok(rd) = std::fs::read_dir(proc_root) else {
        return out;
    };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(link) = std::fs::read_link(fd.path()) {
                let l = link.to_string_lossy();
                if let Some(n) = l.strip_prefix("socket:[").and_then(|r| r.strip_suffix(']')) {
                    if let Ok(inode) = n.parse() {
                        out.insert(inode, pid);
                    }
                }
            }
        }
    }
    out
}

/// Every listening TCP port and bound UDP service port (below the ephemeral
/// range), with its process and directory where we can see them. One line
/// per process and port: IPv4 and IPv6 sockets fold together, keeping the
/// widest exposure. `owners` maps socket inodes to pids.
pub fn scan(owners: &HashMap<u64, u32>) -> Vec<Port> {
    let net = Path::new("/proc/net");
    let mut socks: Vec<(&str, Socket)> = vec![];
    for (proto, files, state) in [
        ("tcp", ["tcp", "tcp6"], "0A"),
        ("udp", ["udp", "udp6"], "07"),
    ] {
        for f in files {
            if let Some(t) = read(net.join(f)) {
                socks.extend(
                    parse_table(&t, state)
                        .into_iter()
                        .filter(|s| proto == "tcp" || s.port < 32768)
                        .map(|s| (proto, s)),
                );
            }
        }
    }
    let home = crate::paths::home().to_string_lossy().into_owned();
    let rank = |e: &str| match e {
        "all" => 2,
        "address" => 1,
        _ => 0,
    };
    let mut out: Vec<Port> = vec![];
    for (proto, s) in socks {
        let pid = owners.get(&s.inode).copied();
        let process = pid.and_then(|p| super::read_trim(format!("/proc/{p}/comm")));
        let cwd = pid
            .and_then(|p| std::fs::read_link(format!("/proc/{p}/cwd")).ok())
            .map(|c| c.to_string_lossy().into_owned())
            // The root and the home directory say nothing about which
            // project started it.
            .filter(|c| c != "/" && *c != home);
        let exp = exposure(s.addr);
        if let Some(p) = out
            .iter_mut()
            .find(|p| p.proto == proto && p.port == s.port && p.pid == pid)
        {
            if rank(exp) > rank(&p.exposure) {
                p.exposure = exp.into();
                p.addr = s.addr.to_string();
            }
            continue;
        }
        out.push(Port {
            proto: proto.into(),
            port: s.port,
            addr: s.addr.to_string(),
            exposure: exp.into(),
            uid: s.uid,
            pid,
            process,
            cwd,
        });
    }
    out.sort_by(|a, b| {
        rank(&b.exposure)
            .cmp(&rank(&a.exposure))
            .then(a.port.cmp(&b.port))
            .then(a.proto.cmp(&b.proto))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // From a desktop running sshd, a loopback service and CUPS.
    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:A435 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1233864 2 00000000d064b672 100 0 0 10 0
   1: 0100007F:9337 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12254624 2 000000005d921ea5 100 0 0 10 0
   2: 3301A8C0:01BB 6E4A7D8E:C350 01 00000000:00000000 00:00000000 00000000  1000        0 35194 1 00000000de3ea092 100 0 0 10 0
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 13169 1 000000007ba683c0 100 0 0 10 0
   1: 00000000000000000000000001000000:0277 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 10620 1 0000000062d4d547 100 0 0 10 5
";
    const UDP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
 1359: FB0000E0:14E9 00000000:0000 07 00000000:00000000 00:00000000 00000000  1000        0 14294352 2 000000009356e5ad 0
 1360: 3301A8C0:D2F0 08080808:0035 07 00000000:00000000 00:00000000 00000000  1000        0 14294353 2 000000009356e5ad 0
";

    #[test]
    fn reads_listeners() {
        let t = parse_table(TCP, "0A");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].addr, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!((t[0].port, t[0].uid, t[0].inode), (42037, 0, 1233864));
        let t6 = parse_table(TCP6, "0A");
        assert_eq!(t6[0].addr, "::".parse::<IpAddr>().unwrap());
        assert_eq!(t6[0].port, 22);
        assert_eq!(t6[1].addr, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(t6[1].port, 631);
        // mDNS on its multicast group; the DNS client with a peer is skipped.
        let u = parse_table(UDP, "07");
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].addr, "224.0.0.251".parse::<IpAddr>().unwrap());
        assert_eq!(u[0].port, 5353);
    }

    #[test]
    fn exposure_levels() {
        assert_eq!(exposure("0.0.0.0".parse().unwrap()), "all");
        assert_eq!(exposure("::".parse().unwrap()), "all");
        assert_eq!(exposure("127.0.0.1".parse().unwrap()), "loopback");
        assert_eq!(exposure("::ffff:127.0.0.1".parse().unwrap()), "loopback");
        assert_eq!(exposure("192.168.1.51".parse().unwrap()), "address");
    }
}
