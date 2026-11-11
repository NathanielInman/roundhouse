//! The way out: is the router there, is the internet there, does DNS work?
//! Three probes on a helper thread, so a sample never waits on the network.
//!
//! - The default route from `/proc/net/route` (and whether IPv6 has one).
//! - The gateway, with an ICMP echo on an unprivileged ping socket
//!   (`SOCK_DGRAM`/`IPPROTO_ICMP`, allowed by `net.ipv4.ping_group_range`).
//!   It never leaves the LAN, so it is always on.
//! - Opt-in (`network.internet_check`): an echo to an outside address and one
//!   DNS lookup through the system resolver, each with a deadline.
//!
//! A probe counts as failed after two misses in a row, so one lost packet
//! does not change the light.

use super::read;
use crate::model::{Reach, Route};
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long one echo or lookup may take.
const DEADLINE: Duration = Duration::from_secs(2);
/// Between rounds while the drawer is open, and while it is closed.
const OPEN_EVERY: Duration = Duration::from_secs(10);
const CLOSED_EVERY: Duration = Duration::from_secs(30);

/// The IPv4 default route from `/proc/net/route`: interface and gateway.
pub fn parse_route(text: &str) -> Option<(String, Ipv4Addr)> {
    text.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let (iface, dest, gw, flags) = (f.first()?, f.get(1)?, f.get(2)?, f.get(3)?);
        let flags = u32::from_str_radix(flags, 16).ok()?;
        // RTF_UP and RTF_GATEWAY, to 0.0.0.0.
        if *dest != "00000000" || flags & 0x3 != 0x3 {
            return None;
        }
        let gw = u32::from_str_radix(gw, 16).ok()?;
        Some((iface.to_string(), Ipv4Addr::from(gw.swap_bytes())))
    })
}

/// Whether `/proc/net/ipv6_route` has a usable default route (not the
/// loopback reject entries every kernel has).
pub fn parse_route6(text: &str) -> bool {
    text.lines().any(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        let Some(flags) = f.get(8).and_then(|v| u32::from_str_radix(v, 16).ok()) else {
            return false;
        };
        f.first() == Some(&"00000000000000000000000000000000")
            && f.get(1) == Some(&"00")
            && f.get(9) != Some(&"lo")
            // RTF_UP, not RTF_REJECT.
            && flags & 0x1 != 0
            && flags & 0x200 == 0
    })
}

/// Sends one ICMP echo on an unprivileged ping socket and returns the round
/// trip in milliseconds.
#[cfg(target_os = "linux")]
pub fn ping(addr: IpAddr) -> Option<f32> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let (domain, proto, echo) = match addr {
        IpAddr::V4(_) => (libc::AF_INET, libc::IPPROTO_ICMP, 8u8),
        IpAddr::V6(_) => (libc::AF_INET6, libc::IPPROTO_ICMPV6, 128u8),
    };
    // SAFETY: socket() has no preconditions; the fd is owned below.
    let fd = unsafe { libc::socket(domain, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, proto) };
    if fd < 0 {
        return None;
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
    // Type, code, checksum and id are filled by the kernel for ping sockets
    // except the type; the sequence number lets us match the reply.
    let seq: u16 =
        (std::process::id() as u16).wrapping_add(Instant::now().elapsed().subsec_nanos() as u16);
    let mut pkt = [0u8; 16];
    pkt[0] = echo;
    pkt[6..8].copy_from_slice(&seq.to_be_bytes());
    let (sa, len) = sockaddr(addr);
    let start = Instant::now();
    // SAFETY: pkt and sa are valid for the lengths given.
    let sent = unsafe {
        libc::sendto(
            sock.as_raw_fd(),
            pkt.as_ptr() as *const libc::c_void,
            pkt.len(),
            0,
            &sa as *const _ as *const libc::sockaddr,
            len,
        )
    };
    if sent < 0 {
        return None;
    }
    let mut buf = [0u8; 128];
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
        if n < 8 {
            return None;
        }
        // An echo reply (0, or 129 for ICMPv6) with our sequence number.
        let reply = if echo == 8 { 0 } else { 129 };
        if buf[0] == reply && buf[6..8] == seq.to_be_bytes() {
            return Some(start.elapsed().as_secs_f32() * 1000.0);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn sockaddr(addr: IpAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
    // SAFETY: an all-zero sockaddr_storage is a valid value.
    let mut ss: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = match addr {
        IpAddr::V4(a) => {
            // SAFETY: sockaddr_storage is large enough and aligned for sockaddr_in.
            let sin = unsafe { &mut *(&mut ss as *mut _ as *mut libc::sockaddr_in) };
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            sin.sin_addr.s_addr = u32::from_ne_bytes(a.octets());
            std::mem::size_of::<libc::sockaddr_in>()
        }
        IpAddr::V6(a) => {
            // SAFETY: as above, for sockaddr_in6.
            let sin6 = unsafe { &mut *(&mut ss as *mut _ as *mut libc::sockaddr_in6) };
            sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sin6.sin6_addr.s6_addr = a.octets();
            std::mem::size_of::<libc::sockaddr_in6>()
        }
    };
    (ss, len as libc::socklen_t)
}

#[cfg(not(target_os = "linux"))]
pub fn ping(_addr: IpAddr) -> Option<f32> {
    None
}

/// Resolves a name through the system resolver, on its own thread so a hung
/// resolver costs the deadline and no more.
pub fn lookup(name: &str) -> Option<f32> {
    let (tx, rx) = std::sync::mpsc::channel();
    let name = format!("{name}:443");
    let start = Instant::now();
    std::thread::spawn(move || {
        let ok = name.to_socket_addrs().is_ok_and(|mut a| a.next().is_some());
        let _ = tx.send(ok);
    });
    match rx.recv_timeout(DEADLINE) {
        Ok(true) => Some(start.elapsed().as_secs_f32() * 1000.0),
        _ => None,
    }
}

/// What the probe thread knows, shared with the sampler.
#[derive(Debug, Default)]
struct Known {
    route: Option<Route>,
    gateway: Option<Reach>,
    internet: Option<Reach>,
    dns: Option<Reach>,
    tailscale: Option<crate::model::Tailscale>,
}

/// Folds one attempt into a probe's state: it fails only after two misses
/// in a row.
pub fn fold(prev: Option<&Reach>, target: &str, ms: Option<f32>) -> Reach {
    let misses = match ms {
        Some(_) => 0,
        None => prev.map_or(0, |p| p.misses) + 1,
    };
    Reach {
        target: target.to_string(),
        ok: misses < 2,
        ms: ms.or_else(|| prev.and_then(|p| p.ms).filter(|_| misses < 2)),
        misses,
        ever: ms.is_some() || prev.is_some_and(|p| p.ever),
    }
}

/// What to probe, from the config.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Probe outside the LAN at all.
    pub internet: bool,
    pub host: String,
    pub name: String,
}

pub struct Prober {
    known: Arc<Mutex<Known>>,
    deep: Arc<AtomicBool>,
    plan: Arc<Mutex<Plan>>,
    started: bool,
}

impl Default for Prober {
    fn default() -> Self {
        Prober {
            known: Arc::new(Mutex::new(Known::default())),
            deep: Arc::new(AtomicBool::new(false)),
            plan: Arc::new(Mutex::new(Plan::default())),
            started: false,
        }
    }
}

impl Prober {
    /// The latest route and probe results. Starts the probe thread the first
    /// time.
    #[allow(clippy::type_complexity)]
    pub fn sample(
        &mut self,
        deep: bool,
        plan: Plan,
    ) -> (
        Option<Route>,
        [Option<Reach>; 3],
        Option<crate::model::Tailscale>,
    ) {
        self.deep.store(deep, Ordering::Relaxed);
        *self.plan.lock().unwrap() = plan;
        if !self.started {
            self.started = true;
            let (k, d, p) = (self.known.clone(), self.deep.clone(), self.plan.clone());
            std::thread::spawn(move || probe_loop(k, d, p));
        }
        let k = self.known.lock().unwrap();
        (
            k.route.clone(),
            [k.gateway.clone(), k.internet.clone(), k.dns.clone()],
            k.tailscale.clone(),
        )
    }
}

fn probe_loop(known: Arc<Mutex<Known>>, deep: Arc<AtomicBool>, plan: Arc<Mutex<Plan>>) {
    let mut last_deep = false;
    loop {
        let start = Instant::now();
        let plan_now = plan.lock().unwrap().clone();
        let route = read("/proc/net/route").and_then(|t| parse_route(&t));
        let v6 = read("/proc/net/ipv6_route").is_some_and(|t| parse_route6(&t));
        let gw = route.as_ref().map(|(_, g)| *g);
        let gw_ms = gw.and_then(|g| ping(IpAddr::V4(g)));
        let outside = plan_now.internet.then(|| {
            let host = plan_now.host.parse::<IpAddr>().ok();
            (host.and_then(ping), lookup(&plan_now.name))
        });
        let tailscale = super::tunnel::tailscale();
        {
            let mut k = known.lock().unwrap();
            k.tailscale = tailscale;
            k.route = route.as_ref().map(|(iface, g)| Route {
                iface: iface.clone(),
                gateway: g.to_string(),
                ipv6: v6,
            });
            k.gateway = gw.map(|g| fold(k.gateway.as_ref(), &g.to_string(), gw_ms));
            match outside {
                Some((inet, dns)) => {
                    k.internet = Some(fold(k.internet.as_ref(), &plan_now.host, inet));
                    k.dns = Some(fold(k.dns.as_ref(), &plan_now.name, dns));
                }
                None => {
                    k.internet = None;
                    k.dns = None;
                }
            }
        }
        // Sleep until the next round; opening the drawer starts one sooner.
        loop {
            let d = deep.load(Ordering::Relaxed);
            let every = if d { OPEN_EVERY } else { CLOSED_EVERY };
            if start.elapsed() >= every || (d && !last_deep) {
                last_deep = d;
                break;
            }
            last_deep = d;
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // From a desktop on a UniFi LAN with Docker installed.
    const ROUTE: &str =
        "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
eno1\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
eno1\t0942E19C\t0101A8C0\t0007\t0\t0\t0\tFFFFFFFF\t0\t0\t0
docker0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\t0\t0\t0
eno1\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0
";

    // No IPv6 default route: only the loopback reject entries.
    const ROUTE6: &str = "fe800000000000000000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001     eno1
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200       lo
";

    #[test]
    fn default_routes() {
        assert_eq!(
            parse_route(ROUTE),
            Some(("eno1".to_string(), Ipv4Addr::new(192, 168, 1, 1)))
        );
        assert_eq!(parse_route("Iface\tDestination\n"), None);
        assert!(!parse_route6(ROUTE6));
        let with = format!("{ROUTE6}00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00450003     eno1\n");
        assert!(parse_route6(&with));
    }

    #[test]
    fn one_miss_is_not_a_failure() {
        let a = fold(None, "192.168.1.1", Some(0.4));
        assert!(a.ok);
        let b = fold(Some(&a), "192.168.1.1", None);
        assert!(b.ok && b.misses == 1);
        let c = fold(Some(&b), "192.168.1.1", None);
        assert!(!c.ok && c.ms.is_none());
        assert!(fold(Some(&c), "192.168.1.1", Some(0.5)).ok);
    }
}
