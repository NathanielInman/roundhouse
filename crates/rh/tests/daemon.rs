//! Drives a real `rh daemon` in a throwaway directory over its socket: the
//! bar and drawer subscriptions, toggle and hide, the show held for a drawer
//! that is still starting, recording sessions and a clean stop.

#![cfg(target_os = "linux")]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Daemon {
    root: PathBuf,
    child: Child,
}

impl Daemon {
    fn start(name: &str) -> Daemon {
        let root = std::env::temp_dir().join(format!("rh-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["run", "state", "config"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let child = Command::new(env!("CARGO_BIN_EXE_rh"))
            .arg("daemon")
            .env("ROUNDHOUSE_RUNTIME_DIR", root.join("run"))
            .env("ROUNDHOUSE_STATE_DIR", root.join("state"))
            .env("ROUNDHOUSE_CONFIG_DIR", root.join("config"))
            // Never start a real drawer, and never find a real Switchyard.
            .env("ROUNDHOUSE_BOARD", "/bin/true")
            .env("SWITCHYARD_RUNTIME_DIR", root.join("no-yard"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let d = Daemon { root, child };
        let start = Instant::now();
        while !d.sock().exists() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "daemon never listened"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        d
    }

    fn sock(&self) -> PathBuf {
        self.root.join("run/rh.sock")
    }

    fn connect(&self) -> UnixStream {
        let s = UnixStream::connect(self.sock()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s
    }

    fn request(&self, line: &str) -> Value {
        let mut s = self.connect();
        writeln!(s, "{line}").unwrap();
        let mut out = String::new();
        BufReader::new(s).read_line(&mut out).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    fn subscribe(&self, kind: &str) -> impl Iterator<Item = Value> {
        let mut s = self.connect();
        writeln!(s, r#"{{"cmd":"subscribe","kind":"{kind}"}}"#).unwrap();
        BufReader::new(s)
            .lines()
            .map_while(Result::ok)
            .map(|l| serde_json::from_str(&l).unwrap())
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn event(v: &Value) -> &str {
    v["event"].as_str().unwrap_or("")
}

/// The next event that is not a snapshot.
fn next_action(events: &mut impl Iterator<Item = Value>) -> Value {
    events.find(|e| event(e) != "snapshot").unwrap()
}

#[test]
fn answers_and_feeds_the_bar() {
    let d = Daemon::start("bar");
    let ping = d.request(r#"{"cmd":"ping"}"#);
    assert_eq!(ping["ok"], true);
    assert_eq!(ping["version"], env!("CARGO_PKG_VERSION"));
    let bad = d.request("{\"cmd\":\"fly\"}");
    assert_eq!(bad["ok"], false);
    // The bar gets a formatted waybar line with the lamp as its class.
    let line = d.subscribe("bar").next().unwrap();
    assert!(["calm", "heads-up", "act"].contains(&line["class"].as_str().unwrap()));
    assert!(line["tooltip"].as_str().unwrap().contains("Memory"));
}

#[test]
fn toggles_reach_the_drawer() {
    let d = Daemon::start("board");
    let mut board = d.subscribe("board");
    assert_eq!(event(&board.next().unwrap()), "snapshot");
    d.request(r#"{"cmd":"toggle","tab":"storage"}"#);
    let e = next_action(&mut board);
    assert_eq!((event(&e), e["tab"].as_str()), ("toggle", Some("storage")));
    d.request(r#"{"cmd":"hide"}"#);
    assert_eq!(event(&next_action(&mut board)), "hide");
}

#[test]
fn a_starting_drawer_gets_the_show_it_missed() {
    let d = Daemon::start("pending");
    // No drawer yet: the daemon starts one (here /bin/true) and holds the
    // toggle as a show until a drawer subscribes.
    d.request(r#"{"cmd":"toggle","tab":"network"}"#);
    let mut board = d.subscribe("board");
    assert_eq!(event(&board.next().unwrap()), "snapshot");
    let e = board.next().unwrap();
    assert_eq!((event(&e), e["tab"].as_str()), ("show", Some("network")));
}

#[test]
fn sessions_hold_builds_and_are_recorded() {
    let d = Daemon::start("session");
    let r = d.request(r#"{"cmd":"session","on":true}"#);
    assert_eq!(r["snapshot"]["lamp"]["hold"], "a recording session is on");
    assert_eq!(r["snapshot"]["session"]["auto"], false);
    assert!(d.root.join("state/session.json").exists());
    // The sampler wakes for the change, so the next snapshot carries it and
    // the history records it.
    let start = Instant::now();
    loop {
        let s = d.request(r#"{"cmd":"snapshot"}"#);
        if s["snapshot"]["session"].is_object() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(100));
    }
    let history = std::fs::read_to_string(d.root.join("state/history.jsonl")).unwrap();
    assert!(history.contains(r#""session":true"#));
    let r = d.request(r#"{"cmd":"session","on":false}"#);
    assert!(r["snapshot"]["lamp"]["hold"].is_null());
    assert!(!d.root.join("state/session.json").exists());
}

#[test]
fn stopping_removes_the_socket() {
    let mut d = Daemon::start("stop");
    let pid = d.child.id() as i32;
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let _ = d.child.wait();
    assert!(!Path::new(&d.sock()).exists());
}
