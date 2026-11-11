//! VPNs and tunnels. The throughput counter leaves them out on purpose
//! (their traffic also crosses the real NIC), but whether the VPN is up is
//! worth seeing: WireGuard and tun interfaces from `/sys/class/net`, their
//! addresses from `getifaddrs`, and Tailscale's own view from its local API
//! socket when Tailscale runs.

use super::read_trim;
use std::collections::HashMap;
use std::path::Path;

/// One tunnel interface: (name, kind, up).
pub fn find(root: &Path) -> Vec<(String, String, bool)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut out: Vec<(String, String, bool)> = rd
        .flatten()
        .filter(|e| !e.path().join("device").exists())
        .filter_map(|e| {
            let dir = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let uevent = read_trim(dir.join("uevent")).unwrap_or_default();
            let devtype = uevent
                .lines()
                .find_map(|l| l.strip_prefix("DEVTYPE="))
                .unwrap_or("");
            let kind = if devtype == "wireguard" {
                "WireGuard"
            } else if dir.join("tun_flags").exists() {
                if name.starts_with("tailscale") {
                    "Tailscale"
                } else {
                    "tun"
                }
            } else if read_trim(dir.join("type")).as_deref() == Some("512") {
                "PPP"
            } else {
                return None;
            };
            // IFF_UP in the flags, and not reported down.
            let flags = read_trim(dir.join("flags"))
                .and_then(|f| u32::from_str_radix(f.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0);
            let up = flags & 1 == 1 && read_trim(dir.join("operstate")).as_deref() != Some("down");
            Some((name, kind.to_string(), up))
        })
        .collect();
    out.sort();
    out
}

/// Every interface's addresses, by name.
#[cfg(unix)]
pub fn addresses() -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `head` with a list we free below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: cur is a node of the list getifaddrs returned.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: ifa_name is a NUL-terminated string owned by the list.
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: ifa_addr is non-null and its family says which struct it is.
        let addr = unsafe {
            match i32::from((*ifa.ifa_addr).sa_family) {
                libc::AF_INET => {
                    let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    Some(std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string())
                }
                libc::AF_INET6 => {
                    let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                    let a = std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                    // Link-local addresses say nothing about the tunnel.
                    (a.segments()[0] & 0xffc0 != 0xfe80).then(|| a.to_string())
                }
                _ => None,
            }
        };
        if let Some(a) = addr {
            out.entry(name).or_default().push(a);
        }
    }
    // SAFETY: head came from getifaddrs and is freed once.
    unsafe { libc::freeifaddrs(head) };
    out
}

#[cfg(not(unix))]
pub fn addresses() -> HashMap<String, Vec<String>> {
    HashMap::new()
}

/// Tailscale's state from its local API reply to `GET /localapi/v0/status`.
pub fn parse_tailscale(body: &str) -> Option<crate::model::Tailscale> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let peers = v.get("Peer").and_then(|p| p.as_object());
    Some(crate::model::Tailscale {
        state: v.get("BackendState")?.as_str()?.to_string(),
        online: v
            .get("Self")
            .and_then(|s| s.get("Online"))
            .and_then(|o| o.as_bool())
            .unwrap_or(false),
        ips: v
            .get("TailscaleIPs")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        peers: peers.map_or(0, |p| p.len()),
        peers_online: peers.map_or(0, |p| {
            p.values()
                .filter(|x| x.get("Online").and_then(|o| o.as_bool()) == Some(true))
                .count()
        }),
    })
}

/// Asks tailscaled over its socket, when it runs. Called from a helper
/// thread; the socket gets a 2 s deadline.
#[cfg(unix)]
pub fn tailscale() -> Option<crate::model::Tailscale> {
    use std::io::{Read, Write};
    let path = [
        "/run/tailscale/tailscaled.sock",
        "/var/run/tailscale/tailscaled.sock",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())?;
    let mut s = std::os::unix::net::UnixStream::connect(path).ok()?;
    let t = Some(std::time::Duration::from_secs(2));
    s.set_read_timeout(t).ok()?;
    s.set_write_timeout(t).ok()?;
    s.write_all(
        b"GET /localapi/v0/status?peers=true HTTP/1.0\r\nHost: local-tailscaled.sock\r\n\r\n",
    )
    .ok()?;
    let mut buf = String::new();
    s.take(4 << 20).read_to_string(&mut buf).ok()?;
    let (head, body) = buf.split_once("\r\n\r\n")?;
    head.starts_with("HTTP/1.").then_some(())?;
    head.split_whitespace().nth(1).filter(|c| *c == "200")?;
    parse_tailscale(body)
}

#[cfg(not(unix))]
pub fn tailscale() -> Option<crate::model::Tailscale> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, files: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).unwrap();
        for (f, v) in files {
            std::fs::write(dir.join(f), format!("{v}\n")).unwrap();
        }
    }

    #[test]
    fn finds_tunnels_not_bridges() {
        let root = std::env::temp_dir().join(format!("rh-tun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // What the kernel shows for each kind.
        write(
            &root.join("wg0"),
            &[
                ("uevent", "DEVTYPE=wireguard\nINTERFACE=wg0"),
                ("flags", "0x91"),
                ("operstate", "unknown"),
                ("type", "65534"),
            ],
        );
        write(
            &root.join("tailscale0"),
            &[
                ("uevent", "INTERFACE=tailscale0"),
                ("tun_flags", "0x1002"),
                ("flags", "0x11"),
                ("operstate", "unknown"),
                ("type", "65534"),
            ],
        );
        write(
            &root.join("docker0"),
            &[
                ("uevent", "DEVTYPE=bridge\nINTERFACE=docker0"),
                ("flags", "0x1003"),
                ("operstate", "down"),
                ("type", "1"),
            ],
        );
        write(
            &root.join("lo"),
            &[
                ("uevent", "INTERFACE=lo"),
                ("flags", "0x9"),
                ("operstate", "unknown"),
                ("type", "772"),
            ],
        );
        assert_eq!(
            find(&root),
            vec![
                ("tailscale0".to_string(), "Tailscale".to_string(), true),
                ("wg0".to_string(), "WireGuard".to_string(), true),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_tailscale_status() {
        // Trimmed from a tailscaled 1.7x status reply.
        let body = r#"{"Version":"1.76.6","BackendState":"Running","TailscaleIPs":["100.101.102.103","fd7a:115c:a1e0::1"],
            "Self":{"HostName":"desk","Online":true},
            "Peer":{"nodekey:a":{"HostName":"nas","Online":true},"nodekey:b":{"HostName":"phone","Online":false}}}"#;
        let t = parse_tailscale(body).unwrap();
        assert_eq!(t.state, "Running");
        assert!(t.online);
        assert_eq!(t.ips[0], "100.101.102.103");
        assert_eq!((t.peers, t.peers_online), (2, 1));
        assert!(parse_tailscale(r#"{"BackendState":"NeedsLogin"}"#).is_some_and(|t| !t.online));
    }
}
