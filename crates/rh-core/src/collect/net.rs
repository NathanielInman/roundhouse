//! Network throughput across the physical interfaces, and the public IP.
//!
//! Throughput comes from `/sys/class/net/*/statistics`, summed over every
//! interface backed by a real device, so docker bridges, veths and loopback
//! never count and a renamed NIC needs no config.
//!
//! The public IP is opt-in (`network.public_ip`). It is fetched on a helper
//! thread with one plain HTTP request, only while the drawer is open, at most
//! once a minute, so a sample never waits on the network.

use super::{push_hist, read_trim};
use crate::model::Network;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const IP_EVERY: Duration = Duration::from_secs(60);
const IP_HOST: &str = "ifconfig.co";

#[derive(Default)]
struct IpState {
    ip: Option<String>,
    fetched: Option<Instant>,
    busy: bool,
}

#[derive(Default)]
pub struct NetReader {
    prev: Option<(Instant, u64, u64)>,
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

/// Pulls the address out of an HTTP response whose body is just the IP.
pub fn parse_ip_response(resp: &str) -> Option<String> {
    let (head, body) = resp.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return None;
    }
    let ip = body.trim();
    ip.parse::<std::net::IpAddr>().ok().map(|_| ip.to_string())
}

fn fetch_ip() -> Option<String> {
    let addr = (IP_HOST, 80)
        .to_socket_addrs()
        .ok()?
        .find(|a| a.is_ipv4())?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let req = format!(
        "GET /ip HTTP/1.1\r\nHost: {IP_HOST}\r\nUser-Agent: roundhouse/{}\r\nAccept: text/plain\r\nConnection: close\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut buf = String::new();
    s.take(4096).read_to_string(&mut buf).ok()?;
    parse_ip_response(&buf)
}

impl NetReader {
    fn refresh_ip(&self) {
        let mut st = self.ip.lock().unwrap();
        if st.busy || st.fetched.is_some_and(|t| t.elapsed() < IP_EVERY) {
            return;
        }
        st.busy = true;
        let shared = self.ip.clone();
        std::thread::spawn(move || {
            let ip = fetch_ip();
            let mut st = shared.lock().unwrap();
            st.busy = false;
            st.fetched = Some(Instant::now());
            if ip.is_some() {
                st.ip = ip;
            }
        });
    }

    pub fn sample(&mut self, deep: bool, public_ip: bool) -> Network {
        let root = Path::new("/sys/class/net");
        let ifaces = physical(root);
        let (mut rx, mut tx) = (0u64, 0u64);
        for (name, _) in &ifaces {
            let stat = |f: &str| {
                read_trim(root.join(name).join("statistics").join(f))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            rx += stat("rx_bytes");
            tx += stat("tx_bytes");
        }
        let now = Instant::now();
        let (mut down, mut up) = (0.0, 0.0);
        if let Some((t, prx, ptx)) = self.prev {
            let dt = now.duration_since(t).as_secs_f64();
            if dt > 0.0 {
                down = rx.saturating_sub(prx) as f64 / dt;
                up = tx.saturating_sub(ptx) as f64 / dt;
            }
            push_hist(&mut self.down, down as f32);
            push_hist(&mut self.up, up as f32);
        }
        self.prev = Some((now, rx, tx));
        if public_ip && deep {
            self.refresh_ip();
        }
        Network {
            interfaces: ifaces
                .into_iter()
                .filter(|(_, up)| *up)
                .map(|(n, _)| n)
                .collect(),
            down,
            up,
            down_history: self.down.iter().copied().collect(),
            up_history: self.up.iter().copied().collect(),
            public_ip: if public_ip {
                self.ip.lock().unwrap().ip.clone()
            } else {
                None
            },
        }
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
    fn ip_responses() {
        let ok = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\n203.0.113.42\n";
        assert_eq!(parse_ip_response(ok).as_deref(), Some("203.0.113.42"));
        assert!(parse_ip_response("HTTP/1.1 429 Too Many\r\n\r\nslow down").is_none());
        assert!(parse_ip_response("HTTP/1.1 200 OK\r\n\r\n<html>").is_none());
    }

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
}
