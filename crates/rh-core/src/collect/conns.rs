//! Network traffic by app, and who each app talks to.
//!
//! The kernel's socket diagnostics (netlink `NETLINK_SOCK_DIAG`, what `ss`
//! reads) list every TCP connection with its `tcp_info`, which counts the
//! bytes it has received and the bytes the peer has acknowledged. No
//! privilege is needed. A socket's inode leads to the process holding it
//! through `/proc/*/fd`, and the process to its app through its cgroup, the
//! way memory by app counts.
//!
//! What it cannot see: UDP carries no byte counts, so QUIC (much of a
//! browser's traffic) shows its peers but no traffic; a connection that
//! opens and closes between two samples; and which app holds a socket that
//! belongs to another user's process.

use super::apps;
use crate::model::{NetApp, Peer};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::time::{Duration, Instant};

/// One socket from a sock_diag dump.
#[derive(Debug, Clone, PartialEq)]
pub struct Conn {
    /// `tcp` or `udp`.
    pub proto: &'static str,
    pub remote: IpAddr,
    pub port: u16,
    pub uid: u32,
    /// 0 for a socket no process holds any more (closing).
    pub inode: u64,
    /// The kernel's id for the socket, unique for as long as it lives.
    pub cookie: u64,
    /// Bytes the peer acknowledged and bytes received (TCP only).
    pub bytes: Option<(u64, u64)>,
}

const SOCK_DIAG_BY_FAMILY: u16 = 20;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const INET_DIAG_INFO: u16 = 2;
/// `struct nlmsghdr`.
const HDR: usize = 16;
/// `struct inet_diag_msg`.
const MSG: usize = 72;
/// `AF_INET` and `AF_INET6` as the kernel writes them.
const INET: u8 = 2;
const INET6: u8 = 10;
/// Where `tcpi_bytes_acked` and `tcpi_bytes_received` sit in `struct
/// tcp_info` (Linux 4.1 and later).
const BYTES_ACKED: usize = 120;
const BYTES_RECEIVED: usize = 128;

/// TCP states worth listing: established and the ones either side of it.
/// Listeners are the Ports tab's; TIME_WAIT sockets carry no counters.
const TCP_STATES: u32 = 1 << 1 | 1 << 2 | 1 << 4 | 1 << 5 | 1 << 8 | 1 << 9 | 1 << 11;
/// A UDP socket "established" is one connected to a peer (a QUIC client).
const UDP_STATES: u32 = 1 << 1;

fn align(n: usize) -> usize {
    (n + 3) & !3
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// An address from an inet_diag socket id. An IPv4-mapped IPv6 address (a
/// dual-stack socket talking IPv4) reads as IPv4.
fn addr(family: u8, raw: &[u8]) -> Option<IpAddr> {
    match family {
        INET => Some(IpAddr::V4(Ipv4Addr::from(
            <[u8; 4]>::try_from(raw.get(..4)?).ok()?,
        ))),
        INET6 => {
            let a = Ipv6Addr::from(<[u8; 16]>::try_from(raw.get(..16)?).ok()?);
            Some(a.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(a)))
        }
        _ => None,
    }
}

/// One `inet_diag_msg` and its attributes.
fn conn(m: &[u8], proto: &'static str) -> Option<Conn> {
    if m.len() < MSG {
        return None;
    }
    let remote = addr(m[0], &m[24..40])?;
    let mut bytes = None;
    let mut a = MSG;
    while a + 4 <= m.len() {
        let len = u16_at(m, a)? as usize;
        if len < 4 || a + len > m.len() {
            break;
        }
        if u16_at(m, a + 2)? & 0x3fff == INET_DIAG_INFO {
            let info = &m[a + 4..a + len];
            if let (Some(up), Some(down)) =
                (u64_at(info, BYTES_ACKED), u64_at(info, BYTES_RECEIVED))
            {
                bytes = Some((up, down));
            }
        }
        a += align(len);
    }
    Some(Conn {
        proto,
        remote,
        port: u16::from_be_bytes([m[6], m[7]]),
        uid: u32_at(m, 64)?,
        inode: u32_at(m, 68)? as u64,
        cookie: u32_at(m, 44)? as u64 | (u32_at(m, 48)? as u64) << 32,
        bytes,
    })
}

/// Reads the sockets out of one buffer of a sock_diag reply into `out`.
/// True once the dump is over (done, an error, or a reply that makes no
/// sense).
pub fn parse(buf: &[u8], proto: &'static str, out: &mut Vec<Conn>) -> bool {
    let mut off = 0;
    while off + HDR <= buf.len() {
        let (Some(len), Some(kind)) = (u32_at(buf, off), u16_at(buf, off + 4)) else {
            return true;
        };
        let len = len as usize;
        if len < HDR || off + len > buf.len() {
            return true;
        }
        match kind {
            NLMSG_DONE | NLMSG_ERROR => return true,
            SOCK_DIAG_BY_FAMILY => out.extend(conn(&buf[off + HDR..off + len], proto)),
            _ => {}
        }
        off += align(len);
    }
    false
}

/// Asks the kernel for every socket of one family and protocol in `states`,
/// with a one-second deadline.
#[cfg(target_os = "linux")]
fn dump(family: u8, protocol: u8, states: u32, proto: &'static str) -> Vec<Conn> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    const DEADLINE: Duration = Duration::from_secs(1);
    let mut out = vec![];
    // SAFETY: socket() has no preconditions; the fd is owned below.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_SOCK_DIAG,
        )
    };
    if fd < 0 {
        return out;
    }
    // SAFETY: fd is a fresh descriptor nobody else owns.
    let sock = unsafe { OwnedFd::from_raw_fd(fd) };
    let tv = libc::timeval {
        tv_sec: DEADLINE.as_secs() as libc::time_t,
        tv_usec: 0,
    };
    // SAFETY: tv outlives the call and the size is right.
    unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
    // nlmsghdr, then inet_diag_req_v2 with an empty socket id (match all).
    let mut req = [0u8; HDR + 56];
    let len = req.len() as u32;
    req[0..4].copy_from_slice(&len.to_ne_bytes());
    req[4..6].copy_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    let flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
    req[6..8].copy_from_slice(&flags.to_ne_bytes());
    req[8..12].copy_from_slice(&1u32.to_ne_bytes());
    req[16] = family;
    req[17] = protocol;
    if protocol == libc::IPPROTO_TCP as u8 {
        req[18] = 1 << (INET_DIAG_INFO - 1);
    }
    req[20..24].copy_from_slice(&states.to_ne_bytes());
    // SAFETY: an all-zero sockaddr_nl is valid; the kernel is port 0.
    let mut sa: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    sa.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    // SAFETY: req and sa are valid for the lengths given.
    let sent = unsafe {
        libc::sendto(
            sock.as_raw_fd(),
            req.as_ptr() as *const libc::c_void,
            req.len(),
            0,
            &sa as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        return out;
    }
    let start = Instant::now();
    let mut buf = vec![0u8; 64 * 1024];
    while start.elapsed() < DEADLINE {
        // SAFETY: buf is valid for its length.
        let n = unsafe {
            libc::recv(
                sock.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if n <= 0 || parse(&buf[..n as usize], proto, &mut out) {
            break;
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
fn dump(_: u8, _: u8, _: u32, _: &'static str) -> Vec<Conn> {
    vec![]
}

/// Every connection off this machine: TCP with its byte counts, and UDP
/// sockets connected to a peer. Loopback and sockets no process holds are
/// left out.
pub fn scan() -> Vec<Conn> {
    let (tcp, udp) = (6u8, 17u8);
    let mut out = vec![];
    for family in [INET, INET6] {
        out.extend(dump(family, tcp, TCP_STATES, "tcp"));
        out.extend(dump(family, udp, UDP_STATES, "udp"));
    }
    out.retain(|c| c.inode != 0 && !c.remote.is_loopback() && !c.remote.is_unspecified());
    out
}

/// Socket owners are looked up again this often.
const OWNERS_EVERY: Duration = Duration::from_secs(5);
/// ...or this often while a busy socket has no known owner (a new
/// connection).
const OWNERS_SOON: Duration = Duration::from_secs(1);
/// Samples further apart than this (the drawer was closed) start the counts
/// over instead of crediting a connection's whole life to one second.
const STALE: Duration = Duration::from_secs(3);
/// Peers kept per app.
const PEERS: usize = 8;
/// The time constant of an app's smoothed rate, which orders the list so it
/// does not reshuffle every second.
const SMOOTH: f64 = 5.0;
/// An app whose last connection closed stays this long as a shadow, so the
/// list does not jump as short connections come and go.
const LINGER: Duration = Duration::from_secs(10);

/// An app seen recently.
#[derive(Debug)]
struct Seen {
    name: String,
    recent: f64,
    /// How long since its last connection closed.
    gone_for: Duration,
}

/// What one sample needs from the last: each socket's counters and who owns
/// which socket.
#[derive(Default)]
pub struct Tracker {
    prev: HashMap<u64, (u64, u64)>,
    prev_at: Option<Instant>,
    owners: HashMap<u64, u32>,
    owners_at: Option<Instant>,
    names: HashMap<u32, Option<String>>,
    /// By lowercase name.
    seen: HashMap<String, Seen>,
}

impl Tracker {
    /// Socket inode to the pid holding it, looked up again when older than
    /// `max_age`. The Ports tab reads it too.
    pub fn owners(&mut self, max_age: Duration) -> &HashMap<u64, u32> {
        if self.owners_at.is_none_or(|t| t.elapsed() >= max_age) {
            self.owners = super::ports::owners(Path::new("/proc"));
            self.owners_at = Some(Instant::now());
            self.names.clear();
        }
        &self.owners
    }

    /// Traffic and peers by app since the last call.
    pub fn sample(&mut self) -> Vec<NetApp> {
        let conns = scan();
        let now = Instant::now();
        let gap = self.prev_at.map(|t| now - t);
        let rates = self.rates(&conns, now);
        // A busy socket nobody owns yet is a new connection: look sooner.
        let unknown = conns
            .iter()
            .zip(&rates)
            .any(|(c, r)| r.0 + r.1 > 0.0 && !self.owners.contains_key(&c.inode));
        self.owners(if unknown { OWNERS_SOON } else { OWNERS_EVERY });
        let names: Vec<String> = conns.iter().map(|c| self.name(c)).collect();
        self.settle(group(&conns, &rates, &names), gap)
    }

    /// Smooths each app's rate, keeps apps whose connections all closed as
    /// shadows for a while, and orders the list by the smoothed rate. `gap`
    /// is the time since the last sample.
    fn settle(&mut self, mut apps: Vec<NetApp>, gap: Option<Duration>) -> Vec<NetApp> {
        let gap = gap.filter(|g| *g <= STALE);
        if gap.is_none() {
            self.seen.clear();
        }
        let step = gap.unwrap_or_default();
        let alpha = 1.0 - (-step.as_secs_f64() / SMOOTH).exp();
        for a in &mut apps {
            let now = a.down + a.up;
            let e = self
                .seen
                .entry(a.name.to_lowercase())
                .or_insert_with(|| Seen {
                    name: a.name.clone(),
                    recent: now,
                    gone_for: Duration::ZERO,
                });
            e.recent += alpha * (now - e.recent);
            e.gone_for = Duration::ZERO;
            a.recent = e.recent;
        }
        let here: std::collections::HashSet<String> =
            apps.iter().map(|a| a.name.to_lowercase()).collect();
        self.seen.retain(|key, e| {
            if here.contains(key) {
                return true;
            }
            e.gone_for += step;
            e.recent *= 1.0 - alpha;
            e.gone_for <= LINGER
        });
        for (key, e) in &self.seen {
            if !here.contains(key) {
                apps.push(NetApp {
                    name: e.name.clone(),
                    recent: e.recent,
                    gone: true,
                    ..Default::default()
                });
            }
        }
        apps.sort_by(|x, y| {
            y.recent
                .partial_cmp(&x.recent)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(x.gone.cmp(&y.gone))
                .then((y.tcp + y.udp).cmp(&(x.tcp + x.udp)))
                .then(x.name.cmp(&y.name))
        });
        apps
    }

    /// Bytes per second (down, up) for each connection since the last
    /// sample. A connection new since then counts all its bytes.
    fn rates(&mut self, conns: &[Conn], now: Instant) -> Vec<(f64, f64)> {
        let gap = self.prev_at.map(|t| now - t);
        let fresh = gap.is_some_and(|g| g <= STALE);
        let secs = gap.map(|g| g.as_secs_f64()).unwrap_or(1.0).max(0.05);
        let mut next = HashMap::with_capacity(conns.len());
        let rates = conns
            .iter()
            .map(|c| {
                let Some((up, down)) = c.bytes else {
                    return (0.0, 0.0);
                };
                next.insert(c.cookie, (up, down));
                if !fresh {
                    return (0.0, 0.0);
                }
                let (pu, pd) = self.prev.get(&c.cookie).copied().unwrap_or((0, 0));
                (
                    down.saturating_sub(pd) as f64 / secs,
                    up.saturating_sub(pu) as f64 / secs,
                )
            })
            .collect();
        self.prev = next;
        self.prev_at = Some(now);
        rates
    }

    fn name(&mut self, c: &Conn) -> String {
        let named = self.owners.get(&c.inode).and_then(|pid| {
            self.names
                .entry(*pid)
                .or_insert_with(|| apps::name_of(*pid))
                .clone()
        });
        named.unwrap_or_else(|| match c.uid {
            0 => "system".into(),
            uid => format!("uid {uid}"),
        })
    }
}

/// Folds connections into apps (by name, case-insensitively) and each app's
/// connections into peers. Busiest first, then most connections.
pub fn group(conns: &[Conn], rates: &[(f64, f64)], names: &[String]) -> Vec<NetApp> {
    let mut apps: HashMap<String, NetApp> = HashMap::new();
    for ((c, &(down, up)), name) in conns.iter().zip(rates).zip(names) {
        let a = apps.entry(name.to_lowercase()).or_insert_with(|| NetApp {
            name: name.clone(),
            ..Default::default()
        });
        a.down += down;
        a.up += up;
        if c.proto == "tcp" {
            a.tcp += 1;
        } else {
            a.udp += 1;
        }
        let addr = c.remote.to_string();
        match a
            .peers
            .iter_mut()
            .find(|p| p.proto == c.proto && p.port == c.port && p.addr == addr)
        {
            Some(p) => {
                p.conns += 1;
                p.down += down;
                p.up += up;
            }
            None => a.peers.push(Peer {
                proto: c.proto.into(),
                addr,
                port: c.port,
                conns: 1,
                down,
                up,
            }),
        }
    }
    let busier = |a: (f64, usize), b: (f64, usize)| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.1.cmp(&a.1))
    };
    let mut out: Vec<NetApp> = apps.into_values().collect();
    for a in &mut out {
        a.peers.sort_by(|x, y| {
            busier((x.down + x.up, x.conns), (y.down + y.up, y.conns)).then(x.addr.cmp(&y.addr))
        });
        a.peer_count = a.peers.len();
        a.peers.truncate(PEERS);
    }
    out.sort_by(|x, y| {
        busier(
            (x.down + x.up, x.tcp + x.udp),
            (y.down + y.up, y.tcp + y.udp),
        )
        .then(x.name.cmp(&y.name))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real replies from a desktop, addresses rewritten to documentation
    // ranges: five established TCP connections to port 443 with tcp_info,
    // then NLMSG_DONE; and two connected UDP sockets.
    const TCP: &[u8] = include_bytes!("../../testdata/sock-diag-tcp.bin");
    const UDP: &[u8] = include_bytes!("../../testdata/sock-diag-udp.bin");

    #[test]
    fn reads_tcp_with_counters() {
        let mut out = vec![];
        assert!(parse(TCP, "tcp", &mut out));
        assert_eq!(out.len(), 5);
        let c = &out[0];
        assert_eq!(c.remote, "203.0.113.1".parse::<IpAddr>().unwrap());
        assert_eq!((c.port, c.uid, c.inode), (443, 1000, 18918141));
        // `ss -ti` said bytes_acked:14727 bytes_received:8973.
        assert_eq!(c.bytes, Some((14727, 8973)));
        assert_eq!(out[3].bytes, Some((34748252, 2207218)));
        assert!(out.iter().all(|c| c.cookie != 0));
    }

    #[test]
    fn reads_udp_without_counters() {
        let mut out = vec![];
        assert!(parse(UDP, "udp", &mut out));
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| c.bytes.is_none() && c.proto == "udp"));
        assert_eq!(out[1].uid, 964);
    }

    #[test]
    fn a_cut_off_reply_is_not_done() {
        let mut out = vec![];
        // The first message alone: more is coming.
        let first = u32_at(TCP, 0).unwrap() as usize;
        assert!(!parse(&TCP[..first], "tcp", &mut out));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn mapped_addresses_read_as_ipv4() {
        let a: Ipv6Addr = "::ffff:192.0.2.7".parse().unwrap();
        assert_eq!(addr(INET6, &a.octets()), Some("192.0.2.7".parse().unwrap()));
        let b: Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert_eq!(addr(INET6, &b.octets()), Some(IpAddr::V6(b)));
    }

    fn c(proto: &'static str, remote: &str, cookie: u64, bytes: Option<(u64, u64)>) -> Conn {
        Conn {
            proto,
            remote: remote.parse().unwrap(),
            port: 443,
            uid: 1000,
            inode: cookie,
            cookie,
            bytes,
        }
    }

    #[test]
    fn rates_need_a_recent_sample() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        let a = [c("tcp", "192.0.2.1", 1, Some((1000, 5000)))];
        // The first sample only sets the baseline.
        assert_eq!(t.rates(&a, t0), vec![(0.0, 0.0)]);
        let a2 = [
            c("tcp", "192.0.2.1", 1, Some((1500, 9000))),
            c("tcp", "192.0.2.2", 2, Some((100, 200))),
        ];
        let r = t.rates(&a2, t0 + Duration::from_secs(2));
        // 4000 down and 500 up over two seconds; the new one counts whole.
        assert_eq!(r, vec![(2000.0, 250.0), (100.0, 50.0)]);
        // After a long gap the counts start over.
        let r = t.rates(&a2, t0 + Duration::from_secs(60));
        assert_eq!(r, vec![(0.0, 0.0), (0.0, 0.0)]);
    }

    fn app(name: &str, down: f64) -> NetApp {
        NetApp {
            name: name.into(),
            down,
            tcp: 1,
            ..Default::default()
        }
    }

    #[test]
    fn apps_linger_as_shadows_and_hold_their_place() {
        let mut t = Tracker::default();
        let s = Some(Duration::from_secs(1));
        let out = t.settle(vec![app("Curl", 4000.0), app("Slack", 100.0)], None);
        assert_eq!(out[0].recent, 4000.0);
        // Curl goes quiet for a second: its smoothed rate still ranks first.
        let out = t.settle(vec![app("Curl", 0.0), app("Slack", 200.0)], s);
        assert_eq!(out[0].name, "Curl");
        assert!(out[0].recent > 3000.0 && out[0].recent < 4000.0);
        // Its connection closes: a shadow, still there.
        let out = t.settle(vec![app("Slack", 200.0)], s);
        let curl = out.iter().find(|a| a.name == "Curl").unwrap();
        assert!(curl.gone && curl.tcp == 0 && curl.down == 0.0);
        // Ten seconds on, it is gone.
        let mut out = vec![];
        for _ in 0..10 {
            out = t.settle(vec![app("Slack", 200.0)], s);
        }
        assert_eq!(out.len(), 1);
        // A long gap (the drawer was closed) forgets everything.
        t.settle(vec![app("Curl", 1.0)], s);
        let out = t.settle(vec![app("Slack", 5.0)], Some(Duration::from_secs(60)));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].recent, 5.0);
    }

    #[test]
    fn groups_by_app_and_peer() {
        let conns = [
            c("tcp", "192.0.2.1", 1, Some((0, 0))),
            c("tcp", "192.0.2.1", 2, Some((0, 0))),
            c("udp", "192.0.2.9", 3, None),
            c("tcp", "198.51.100.4", 4, Some((0, 0))),
        ];
        let rates = [(10.0, 1.0), (5.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        let names = ["Chrome", "chrome", "Chrome", "Slack"].map(String::from);
        let apps = group(&conns, &rates, &names);
        assert_eq!(apps.len(), 2);
        let chrome = &apps[0];
        assert_eq!(
            (chrome.name.as_str(), chrome.tcp, chrome.udp),
            ("Chrome", 2, 1)
        );
        assert_eq!((chrome.down, chrome.up), (15.0, 1.0));
        assert_eq!(chrome.peers[0].addr, "192.0.2.1");
        assert_eq!(chrome.peers[0].conns, 2);
        assert_eq!(chrome.peers[1].proto, "udp");
        assert_eq!(apps[1].name, "Slack");
        assert_eq!(chrome.peer_count, 2);
        assert_eq!(chrome.peers_text(1), "192.0.2.1:443 +1");
        assert_eq!(chrome.peers[1].endpoint(), "192.0.2.9:443/udp");
        assert_eq!(chrome.counts(), "2 TCP · 1 UDP");
    }
}
