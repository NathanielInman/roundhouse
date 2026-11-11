//! Network throughput across the physical interfaces, and the public IP.
//!
//! Throughput comes from `/sys/class/net/*/statistics`, summed over every
//! interface backed by a real device, so docker bridges, veths and loopback
//! never count and a renamed NIC needs no config.
//!
//! The public IP is opt-in (`network.public_ip`). It is fetched on a helper
//! thread with one plain HTTP request, only while the drawer is open, at most
//! once a minute, so a sample never waits on the network.

use super::{push_hist, read, read_trim};
use crate::model::{Link, Network};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const IP_EVERY: Duration = Duration::from_secs(60);

#[derive(Default)]
struct IpState {
    seen: super::stun::Seen,
    fetched: Option<Instant>,
    busy: bool,
}

/// One interface's counters on the last sample.
#[derive(Debug, Clone, Copy, Default)]
struct Counters {
    rx: u64,
    tx: u64,
    errors: u64,
    carrier: u64,
}

#[derive(Default)]
struct LinkState {
    prev: Option<(Instant, Counters)>,
    best_speed: Option<u32>,
    /// Error and flap increments with when they were seen.
    events: VecDeque<(Instant, u64, u64)>,
}

#[derive(Default)]
pub struct NetReader {
    links: HashMap<String, LinkState>,
    down: VecDeque<f32>,
    up: VecDeque<f32>,
    ip: Arc<Mutex<IpState>>,
}

/// Interfaces backed by a device (`eno1`, `wlp8s0`), with their state.
pub fn physical(root: &Path) -> Vec<(String, bool)> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut out: Vec<(String, bool)> = rd
        .flatten()
        .filter(|e| e.path().join("device").exists())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let up = read_trim(e.path().join("operstate")).as_deref() == Some("up");
            (name, up)
        })
        .collect();
    out.sort();
    out
}

/// `/proc/net/wireless`: signal level in dBm by interface.
pub fn parse_wireless(text: &str) -> Vec<(String, f32)> {
    text.lines()
        .skip(2)
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let level = rest.split_whitespace().nth(2)?;
            Some((
                name.trim().to_string(),
                level.trim_end_matches('.').parse().ok()?,
            ))
        })
        .collect()
}

/// How far a counter moved, or 0 when it went backwards (the interface was
/// replugged or reset).
fn delta(now: u64, prev: u64) -> u64 {
    now.saturating_sub(prev)
}

impl NetReader {
    fn refresh_ip(&self, servers: Vec<String>) {
        let mut st = self.ip.lock().unwrap();
        if st.busy || st.fetched.is_some_and(|t| t.elapsed() < IP_EVERY) {
            return;
        }
        st.busy = true;
        let shared = self.ip.clone();
        std::thread::spawn(move || {
            let seen = super::stun::discover(&servers);
            let mut st = shared.lock().unwrap();
            st.busy = false;
            st.fetched = Some(Instant::now());
            // Keep the last answer through a failed round.
            if seen.v4.is_some() || seen.v6.is_some() {
                st.seen = seen;
            }
        });
    }

    /// `stun` is the servers to ask for the public address, or empty when
    /// `network.public_ip` is off.
    pub fn sample(&mut self, deep: bool, stun: &[String]) -> Network {
        let public_ip = !stun.is_empty();
        let root = Path::new("/sys/class/net");
        let wireless = read("/proc/net/wireless")
            .map(|t| parse_wireless(&t))
            .unwrap_or_default();
        let now = Instant::now();
        let minute = Duration::from_secs(60);
        let mut links = vec![];
        let (mut down, mut up) = (0.0, 0.0);
        let mut have_rates = false;
        let ifaces = physical(root);
        for (name, is_up) in &ifaces {
            let dir = root.join(name);
            let stat = |f: &str| {
                read_trim(dir.join("statistics").join(f))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            let c = Counters {
                rx: stat("rx_bytes"),
                tx: stat("tx_bytes"),
                errors: stat("rx_errors") + stat("tx_errors"),
                carrier: read_trim(dir.join("carrier_changes"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            };
            // `speed` reads -1 or fails while the link is down or wireless.
            let speed = read_trim(dir.join("speed"))
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|s| *s > 0)
                .map(|s| s as u32);
            let st = self.links.entry(name.clone()).or_default();
            if let Some(s) = speed {
                st.best_speed = Some(st.best_speed.map_or(s, |b| b.max(s)));
            }
            let (mut rd, mut wr) = (0.0, 0.0);
            if let Some((t, p)) = st.prev {
                let dt = now.duration_since(t).as_secs_f64();
                if dt > 0.0 {
                    rd = delta(c.rx, p.rx) as f64 / dt;
                    wr = delta(c.tx, p.tx) as f64 / dt;
                    have_rates = true;
                }
                let (e, f) = (delta(c.errors, p.errors), delta(c.carrier, p.carrier));
                if e > 0 || f > 0 {
                    st.events.push_back((now, e, f));
                }
            }
            while st
                .events
                .front()
                .is_some_and(|(t, _, _)| now.duration_since(*t) > minute)
            {
                st.events.pop_front();
            }
            st.prev = Some((now, c));
            down += rd;
            up += wr;
            links.push(Link {
                name: name.clone(),
                up: *is_up,
                speed,
                best_speed: st.best_speed,
                down: rd,
                up_rate: wr,
                errors_minute: st.events.iter().map(|e| e.1).sum(),
                // A carrier change counts down and up separately.
                flaps_minute: st.events.iter().map(|e| e.2).sum::<u64>() / 2,
                signal_dbm: wireless.iter().find(|(n, _)| n == name).map(|(_, s)| *s),
                wireless: dir.join("wireless").exists(),
            });
        }
        // Forget interfaces that went away, so a replug starts fresh.
        self.links.retain(|n, _| ifaces.iter().any(|(i, _)| i == n));
        if have_rates {
            push_hist(&mut self.down, down as f32);
            push_hist(&mut self.up, up as f32);
        }
        if public_ip && deep {
            self.refresh_ip(stun.to_vec());
        }
        let seen = if public_ip {
            self.ip.lock().unwrap().seen.clone()
        } else {
            Default::default()
        };
        Network {
            interfaces: ifaces
                .iter()
                .filter(|(_, up)| *up)
                .map(|(n, _)| n.clone())
                .collect(),
            down,
            up,
            down_history: self.down.iter().copied().collect(),
            up_history: self.up.iter().copied().collect(),
            public_ip: seen.v4.map(|a| a.to_string()),
            public_ipv6: seen.v6.map(|a| a.to_string()),
            nat_keeps_port: seen.same_port,
            links,
            ..Default::default()
        }
    }
}

/// Link speed the way it is sold: "2.5 Gb/s", "100 Mb/s".
pub fn speed(mbps: u32) -> String {
    if mbps >= 1000 {
        let g = mbps as f32 / 1000.0;
        if g.fract() == 0.0 {
            format!("{g:.0} Gb/s")
        } else {
            format!("{g:.1} Gb/s")
        }
    } else {
        format!("{mbps} Mb/s")
    }
}

/// A rate in bits per second, the way link speeds are given.
pub fn rate_bits(bps: f64) -> String {
    let b = bps * 8.0;
    if b < 1000.0 {
        format!("{b:.0} b/s")
    } else if b < 1_000_000.0 {
        format!("{:.0} kb/s", b / 1000.0)
    } else if b < 1_000_000_000.0 {
        format!("{:.1} Mb/s", b / 1_000_000.0)
    } else {
        format!("{:.2} Gb/s", b / 1_000_000_000.0)
    }
}

/// A rate in the units `network.units` asks for: "bytes" (the default) or
/// "bits".
pub fn rate_in(bps: f64, units: &str) -> String {
    if units == "bits" {
        rate_bits(bps)
    } else {
        rate(bps)
    }
}

/// Bytes per second in the units network tools use: 1000-based.
pub fn rate(bps: f64) -> String {
    if bps < 1000.0 {
        format!("{bps:.0} B/s")
    } else if bps < 1_000_000.0 {
        format!("{:.0} kB/s", bps / 1000.0)
    } else {
        format!("{:.1} MB/s", bps / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(rate(512.0), "512 B/s");
        assert_eq!(rate(84_000.0), "84 kB/s");
        assert_eq!(rate(1_234_567.0), "1.2 MB/s");
    }

    #[test]
    fn only_real_devices() {
        let root = std::env::temp_dir().join(format!("rh-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("eno1/device")).unwrap();
        std::fs::write(root.join("eno1/operstate"), "up\n").unwrap();
        std::fs::create_dir_all(root.join("docker0")).unwrap();
        std::fs::create_dir_all(root.join("lo")).unwrap();
        assert_eq!(physical(&root), vec![("eno1".to_string(), true)]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn wireless_signal() {
        // /proc/net/wireless with an associated Intel card.
        let text =
            "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE
 face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22
wlp8s0: 0000   54.  -56.  -256        0      0      0      0     21        0
";
        assert_eq!(parse_wireless(text), vec![("wlp8s0".to_string(), -56.0)]);
    }

    #[test]
    fn speeds_and_bits() {
        assert_eq!(speed(2500), "2.5 Gb/s");
        assert_eq!(speed(1000), "1 Gb/s");
        assert_eq!(speed(100), "100 Mb/s");
        assert_eq!(rate_bits(125_000.0), "1.0 Mb/s");
        assert_eq!(rate_in(84_000.0, "bytes"), "84 kB/s");
        assert_eq!(delta(5, 9), 0);
    }
}
