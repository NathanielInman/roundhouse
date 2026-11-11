//! Lamps from other machines. A daemon can serve a small, read-only view of
//! itself (host name, lamp, recording session) over TCP to anyone holding its
//! token, and can watch other daemons the same way, so the NAS's full pool
//! lights this bar too.
//!
//! The listener binds one explicit address, meant for a private network such
//! as a tailnet (`100.x.y.z`); an unspecified address (`0.0.0.0`, `::`) is
//! refused. One request per connection, a 2 s deadline each way, and only
//! `{"cmd":"lamp","token":"..."}` is answered.

use crate::model::{Host, Lamp, RemoteHost, Session, Snapshot};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// The default port, when an address leaves it out.
pub const PORT: u16 = 7461;
const DEADLINE: Duration = Duration::from_secs(2);

/// What a remote reader gets: enough for a lamp, nothing about apps, ports
/// or disks beyond the reasons' words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct View {
    pub host: Host,
    pub lamp: Lamp,
    #[serde(default)]
    pub session: Option<Session>,
    pub now: f64,
}

impl View {
    pub fn of(s: &Snapshot) -> View {
        View {
            host: s.host.clone(),
            lamp: s.lamp.clone(),
            session: s.session.clone(),
            now: s.now,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Ask {
    cmd: String,
    #[serde(default)]
    token: String,
}

/// Parses `host:port` (or a bare host, on the default port) for listening,
/// refusing an address that would answer on every interface.
pub fn listen_addr(s: &str) -> Result<SocketAddr, String> {
    let a = s
        .parse::<SocketAddr>()
        .or_else(|_| s.parse::<IpAddr>().map(|ip| SocketAddr::new(ip, PORT)))
        .map_err(|_| format!("remote.listen: `{s}` is not an IP address (and port)"))?;
    if a.ip().is_unspecified() {
        return Err(format!(
            "remote.listen: `{s}` would answer on every interface; give one address, such as your tailnet IP"
        ));
    }
    Ok(a)
}

/// Compares in constant time, so a token cannot be guessed byte by byte.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// The reply line to one request line.
pub fn answer(line: &str, token: &str, view: &View) -> String {
    let reply = match serde_json::from_str::<Ask>(line) {
        // Before the first sample there is no lamp to give.
        Ok(a) if a.cmd == "lamp" && view.now <= 0.0 => {
            serde_json::json!({"ok": false, "error": "starting"})
        }
        Ok(a) if a.cmd == "lamp" && !token.is_empty() && same(&a.token, token) => {
            serde_json::json!({"ok": true, "view": view})
        }
        Ok(a) if a.cmd == "lamp" => serde_json::json!({"ok": false, "error": "bad token"}),
        _ => serde_json::json!({"ok": false, "error": "only lamp is served"}),
    };
    reply.to_string()
}

/// Serves one connection: one line in, one line out.
pub fn serve(stream: TcpStream, token: &str, view: &View) {
    let _ = stream.set_read_timeout(Some(DEADLINE));
    let _ = stream.set_write_timeout(Some(DEADLINE));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut line = String::new();
    // A line longer than 4 KiB is not a request.
    if BufReader::new(std::io::Read::take(read_half, 4096))
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let mut w = stream;
    let _ = writeln!(w, "{}", answer(line.trim(), token, view));
}

/// Reads a reply into a view.
pub fn parse_reply(line: &str) -> Option<View> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("ok")?.as_bool()? {
        serde_json::from_value(v.get("view")?.clone()).ok()
    } else {
        None
    }
}

/// Asks one remote daemon for its lamp.
pub fn ask(addr: &str, token: &str) -> Option<View> {
    let target = if addr.parse::<SocketAddr>().is_ok() {
        addr.to_string()
    } else if let Ok(ip) = addr.parse::<IpAddr>() {
        SocketAddr::new(ip, PORT).to_string()
    } else if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{addr}:{PORT}")
    };
    let sa = target.to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&sa, DEADLINE).ok()?;
    s.set_read_timeout(Some(DEADLINE)).ok()?;
    s.set_write_timeout(Some(DEADLINE)).ok()?;
    let req = serde_json::json!({"cmd": "lamp", "token": token});
    writeln!(s, "{req}").ok()?;
    let mut line = String::new();
    BufReader::new(std::io::Read::take(s, 1 << 20))
        .read_line(&mut line)
        .ok()?;
    parse_reply(&line)
}

/// Folds an answer (or its absence) into what the drawer shows for a host.
pub fn fold(prev: Option<&RemoteHost>, name: &str, addr: &str, view: Option<View>) -> RemoteHost {
    match view {
        Some(v) => RemoteHost {
            name: name.into(),
            addr: addr.into(),
            online: true,
            ever: true,
            hostname: v.host.name,
            level: v.lamp.level,
            reasons: v
                .lamp
                .reasons
                .into_iter()
                .map(|r| (r.level, r.text))
                .collect(),
            session: v.session.is_some(),
        },
        None => RemoteHost {
            name: name.into(),
            addr: addr.into(),
            online: false,
            ever: prev.is_some_and(|p| p.ever),
            ..prev.cloned().unwrap_or_default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Level, Reason};

    fn view() -> View {
        let mut v = View {
            now: 1.0,
            ..Default::default()
        };
        v.host.name = "rime".into();
        v.lamp.level = Level::Act;
        v.lamp.reasons.push(Reason {
            level: Level::Act,
            text: "/mnt/tank is 96% full".into(),
            tab: "storage".into(),
            key: "fs:/mnt/tank".into(),
        });
        v
    }

    #[test]
    fn refuses_every_interface() {
        assert!(listen_addr("0.0.0.0:7461").is_err());
        assert!(listen_addr("::").is_err());
        assert_eq!(
            listen_addr("100.101.102.103").unwrap(),
            "100.101.102.103:7461".parse().unwrap()
        );
        assert_eq!(listen_addr("100.101.102.103:9000").unwrap().port(), 9000);
        assert_eq!(
            listen_addr("fd7a:115c:a1e0::1").unwrap(),
            "[fd7a:115c:a1e0::1]:7461".parse().unwrap()
        );
        assert!(listen_addr("rime").is_err());
    }

    #[test]
    fn answers_only_with_the_token() {
        let v = view();
        let ok = answer(r#"{"cmd":"lamp","token":"s3cret"}"#, "s3cret", &v);
        assert_eq!(parse_reply(&ok), Some(v.clone()));
        let bad = answer(r#"{"cmd":"lamp","token":"guess"}"#, "s3cret", &v);
        assert_eq!(parse_reply(&bad), None);
        assert!(bad.contains("bad token"));
        // No token configured: nothing is served.
        assert_eq!(
            parse_reply(&answer(r#"{"cmd":"lamp","token":""}"#, "", &v)),
            None
        );
        assert!(answer(r#"{"cmd":"toggle"}"#, "s3cret", &v).contains("only lamp"));
        assert!(answer("junk", "s3cret", &v).contains("only lamp"));
        let starting = View::default();
        assert!(
            answer(r#"{"cmd":"lamp","token":"s3cret"}"#, "s3cret", &starting).contains("starting")
        );
    }

    #[test]
    fn a_host_that_went_away_keeps_its_last_words() {
        let up = fold(None, "rime", "100.1.2.3", Some(view()));
        assert!(up.online && up.ever);
        assert_eq!(up.reasons[0].1, "/mnt/tank is 96% full");
        let down = fold(Some(&up), "rime", "100.1.2.3", None);
        assert!(!down.online && down.ever);
        assert_eq!(down.hostname, "rime");
        let never = fold(None, "vps", "100.9.9.9", None);
        assert!(!never.ever);
    }
}
