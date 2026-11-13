//! Drives a real `rh daemon` in a throwaway directory over its socket: the
//! bar and drawer subscriptions, toggle and hide, the show held for a drawer
//! that is still starting, recording sessions, a clean stop, and retiring
//! when an upgrade replaces the binary.

#![cfg(target_os = "linux")]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

struct Daemon {
    root: PathBuf,
    /// The `rh` this daemon runs.
    bin: PathBuf,
    child: Child,
}

/// `rh` pointed at a throwaway directory.
fn rh(root: &Path) -> Command {
    rh_at(Path::new(env!("CARGO_BIN_EXE_rh")), root)
}

fn rh_at(bin: &Path, root: &Path) -> Command {
    let mut c = Command::new(bin);
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
        Daemon::launch(name, config, false)
    }

    /// A daemon running its own copy of `rh`, which a test may replace the
    /// way an upgrade does.
    fn installed(name: &str) -> Daemon {
        Daemon::launch(name, "", true)
    }

    fn launch(name: &str, config: &str, copy: bool) -> Daemon {
        let root = std::env::temp_dir().join(format!("rh-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["run", "state", "config", "bin"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("config/config.toml"), config).unwrap();
        let bin = if copy {
            install(&root.join("bin/rh"));
            root.join("bin/rh")
        } else {
            PathBuf::from(env!("CARGO_BIN_EXE_rh"))
        };
        let child = spawn(rh_at(&bin, &root).arg("daemon"));
        let d = Daemon { root, bin, child };
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

/// Puts a fresh copy of the built `rh` at `to` the way `install` does: a new
/// file renamed over the old one, so a process running the old one sees its
/// binary as replaced.
fn install(to: &Path) {
    let tmp = to.with_extension("new");
    std::fs::copy(env!("CARGO_BIN_EXE_rh"), &tmp).unwrap();
    std::fs::rename(&tmp, to).unwrap();
}

/// Spawns, retrying while another test thread's fork still holds the
/// freshly copied binary open for writing (ETXTBSY).
fn spawn(cmd: &mut Command) -> Child {
    let start = Instant::now();
    loop {
        match cmd.spawn() {
            Ok(c) => return c,
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                assert!(start.elapsed() < Duration::from_secs(5), "{e}");
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("cannot start rh: {e}"),
        }
    }
}

/// Waits for a child to exit, for at most `limit`.
fn exit_within(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(s) = child.try_wait().unwrap() {
            return Some(s);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
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
fn quit_tells_the_drawer_and_exits() {
    let mut d = Daemon::start("quit");
    let ping = d.request(r#"{"cmd":"ping"}"#);
    assert!(ping.get("stale").is_none(), "a fresh daemon is not stale");
    let mut board = d.subscribe("board");
    assert_eq!(event(&board.next().unwrap()), "snapshot");
    assert_eq!(d.request(r#"{"cmd":"quit"}"#)["ok"], true);
    assert_eq!(event(&next_action(&mut board)), "quit");
    let status = exit_within(&mut d.child, Duration::from_secs(5)).expect("daemon kept running");
    assert!(status.success());
}

#[test]
fn an_upgraded_daemon_retires_within_a_tick() {
    let mut d = Daemon::installed("upgrade");
    let mut board = d.subscribe("board");
    assert_eq!(event(&board.next().unwrap()), "snapshot");
    install(&d.bin);
    // Until it retires, a ping says it runs old code.
    let start = Instant::now();
    let status = loop {
        if let Some(s) = d.child.try_wait().unwrap() {
            break s;
        }
        if let Ok(mut s) = UnixStream::connect(d.sock()) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = writeln!(s, r#"{{"cmd":"ping"}}"#);
            let mut out = String::new();
            if BufReader::new(s).read_line(&mut out).is_ok_and(|n| n > 0) {
                let r: Value = serde_json::from_str(&out).unwrap();
                assert_eq!(r["stale"], true, "{out}");
            }
        }
        // The sampler ticks every 5 seconds while the drawer is closed.
        assert!(start.elapsed() < Duration::from_secs(10), "never retired");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(status.success());
    assert_eq!(event(&next_action(&mut board)), "quit");
}

/// The bar module follows an upgrade on its own: the old daemon retires,
/// the bar re-executes itself in the same process, and the new bar starts
/// the new daemon.
#[test]
fn the_bar_brings_up_the_new_daemon_after_an_upgrade() {
    let mut d = Daemon::installed("bar-upgrade");
    let mut bar = spawn(rh_at(&d.bin, &d.root).arg("bar"));
    let exe = |pid: u32| std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
    // Give the bar a moment to subscribe. It re-executes either way: it
    // checks its binary before every connect too.
    std::thread::sleep(Duration::from_millis(500));
    install(&d.bin);
    let status = exit_within(&mut d.child, Duration::from_secs(10)).expect("daemon kept running");
    assert!(status.success());
    let start = Instant::now();
    let fresh = loop {
        if let Ok(mut s) = UnixStream::connect(d.sock()) {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = writeln!(s, r#"{{"cmd":"ping"}}"#);
            let mut out = String::new();
            if BufReader::new(s).read_line(&mut out).is_ok_and(|n| n > 0) {
                break serde_json::from_str::<Value>(&out).unwrap();
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the bar never started a new daemon"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    // Stop the new daemon (it runs in its own session) before asserting.
    let _ = d.request(r#"{"cmd":"quit"}"#);
    let bar_exe = exe(bar.id());
    let _ = bar.kill();
    let _ = bar.wait();
    assert_eq!(fresh["ok"], true);
    assert!(
        fresh.get("stale").is_none(),
        "the new daemon is stale: {fresh}"
    );
    assert!(
        !bar_exe.to_string_lossy().ends_with("(deleted)"),
        "the bar still runs the old binary: {}",
        bar_exe.display()
    );
}

#[test]
fn a_second_daemon_leaves_the_first_alone() {
    let d = Daemon::start("twice");
    let status = rh(&d.root).arg("daemon").status().unwrap();
    // 75 (EX_TEMPFAIL), which the systemd unit does not restart.
    assert_eq!(status.code(), Some(75));
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
