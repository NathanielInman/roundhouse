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

/// `rh` pointed at a throwaway directory.
fn rh(root: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rh"));
    c.env("ROUNDHOUSE_RUNTIME_DIR", root.join("run"))
        .env("ROUNDHOUSE_STATE_DIR", root.join("state"))
        .env("ROUNDHOUSE_CONFIG_DIR", root.join("config"))
        // Never start a real drawer.
        .env("ROUNDHOUSE_BOARD", "/bin/true")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c
}

impl Daemon {
    fn start(name: &str) -> Daemon {
        Daemon::with_config(name, "")
    }

    fn with_config(name: &str, config: &str) -> Daemon {
        let root = std::env::temp_dir().join(format!("rh-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["run", "state", "config"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("config/config.toml"), config).unwrap();
        let child = rh(&root).arg("daemon").spawn().unwrap();
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
fn sessions_are_kept_and_recorded() {
    let d = Daemon::start("session");
    let r = d.request(r#"{"cmd":"session","on":true}"#);
    assert!(r["snapshot"]["session"]["since"].as_f64().unwrap() > 0.0);
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
    assert!(r["snapshot"]["session"].is_null());
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

#[test]
fn a_second_daemon_leaves_the_first_alone() {
    let d = Daemon::start("twice");
    let status = rh(&d.root).arg("daemon").status().unwrap();
    assert!(!status.success());
    assert!(d.sock().exists());
    assert_eq!(d.request(r#"{"cmd":"ping"}"#)["ok"], true);
}

#[test]
fn one_machine_reads_anothers_lamp() {
    // A free loopback port for the serving daemon.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let _server = Daemon::with_config(
        "remote-a",
        &format!("[remote]\nlisten = \"127.0.0.1:{port}\"\ntoken = \"s3cret\"\n"),
    );
    let watcher = Daemon::with_config(
        "remote-b",
        &format!(
            "[[remote.hosts]]\nname = \"a\"\naddr = \"127.0.0.1:{port}\"\ntoken = \"s3cret\"\n\n[[remote.hosts]]\nname = \"wrong\"\naddr = \"127.0.0.1:{port}\"\ntoken = \"guess\"\n"
        ),
    );
    let start = Instant::now();
    loop {
        let s = watcher.request(r#"{"cmd":"snapshot"}"#);
        let hosts = s["snapshot"]["remotes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let a = hosts.iter().find(|h| h["name"] == "a");
        if a.is_some_and(|h| h["online"] == true) {
            let wrong = hosts.iter().find(|h| h["name"] == "wrong").unwrap();
            assert_eq!(wrong["online"], false, "a bad token must not be served");
            assert!(!a.unwrap()["hostname"].as_str().unwrap().is_empty());
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "never read the lamp: {s}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
