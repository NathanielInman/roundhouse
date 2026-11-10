//! The daemon: samples the machine, keeps the latest snapshot, serves the
//! socket, feeds the bar module, and starts the drawer the first time it is
//! asked for.
//!
//! It samples every second while the drawer is open and every five seconds
//! while it is closed, when only the lamp and the pinned number need data.

use rh_core::collect::Collector;
use rh_core::config::Config;
use rh_core::model::Snapshot;
use rh_core::protocol::{Event, Reply, Request, SubKind};
use rh_core::transport::{Listener, Stream};
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

const OPEN_EVERY: Duration = Duration::from_secs(1);
const CLOSED_EVERY: Duration = Duration::from_secs(5);
/// How often to look for siblings.
const SIBLING_OPEN: Duration = Duration::from_secs(2);
const SIBLING_CLOSED: Duration = Duration::from_secs(15);

struct State {
    cfg: Config,
    cfg_error: Option<String>,
    snap: Snapshot,
    visible: bool,
    boards: Vec<Stream>,
    bars: Vec<Stream>,
    bar_line: String,
    /// A show the drawer has not received yet because it is still starting.
    pending: Option<Event>,
    board_started: Option<Instant>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn send_line(s: &mut Stream, line: &str) -> bool {
    s.write_all(line.as_bytes())
        .and_then(|_| s.write_all(b"\n"))
        .and_then(|_| s.flush())
        .is_ok()
}

/// Writes to every subscriber and drops the ones that went away. Returns how
/// many received it.
fn broadcast(subs: &mut Vec<Stream>, line: &str) -> usize {
    subs.retain_mut(|s| send_line(s, line));
    subs.len()
}

fn event_line(e: &Event) -> String {
    serde_json::to_string(e).unwrap_or_default()
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(rh_core::paths::config_file())
        .and_then(|m| m.modified())
        .ok()
}

/// Starts `rh-board`, preferring the one installed next to `rh`.
fn spawn_board() {
    let exe = std::env::current_exe().ok();
    let local = exe
        .as_ref()
        .and_then(|e| e.parent())
        .map(|d| d.join("rh-board"))
        .filter(|p| p.exists());
    let bin = local
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| "rh-board".into());
    let log = rh_core::paths::board_log();
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log);
    let mut cmd = std::process::Command::new(&bin);
    cmd.stdin(std::process::Stdio::null());
    if let Ok(f) = out {
        if let Ok(f2) = f.try_clone() {
            cmd.stdout(f).stderr(f2);
        }
    }
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("rh: cannot start {}: {e}", bin.to_string_lossy()),
    }
}

/// Delivers a show or toggle to the drawer, starting it when none is
/// connected.
fn to_board(st: &mut State, ev: Event) {
    if broadcast(&mut st.boards, &event_line(&ev)) > 0 {
        return;
    }
    // Nobody to toggle: whatever was asked, the answer is to show.
    let show = match ev {
        Event::Toggle { anchor, tab } | Event::Show { anchor, tab } => Event::Show { anchor, tab },
        other => other,
    };
    if matches!(show, Event::Hide) {
        return;
    }
    st.pending = Some(show);
    if st
        .board_started
        .is_none_or(|t| t.elapsed() > Duration::from_secs(5))
    {
        st.board_started = Some(Instant::now());
        spawn_board();
    }
}

fn handle(shared: Shared, stream: Stream) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    let reader = BufReader::new(read_half);
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_line(
                    &mut writer,
                    &serde_json::to_string(&Reply::err(format!("bad request: {e}")))
                        .unwrap_or_default(),
                );
                continue;
            }
        };
        let (lock, cv) = &*shared;
        let reply = match req {
            Request::Ping => Reply {
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Reply::ok()
            },
            Request::Snapshot => {
                let st = lock.lock().unwrap();
                Reply {
                    snapshot: Some(Box::new(st.snap.clone())),
                    ..Reply::ok()
                }
            }
            Request::Subscribe { kind } => {
                let _ = writer.set_write_timeout(Some(Duration::from_secs(2)));
                let mut st = lock.lock().unwrap();
                match kind {
                    SubKind::Board => {
                        let mut ok = send_line(
                            &mut writer,
                            &event_line(&Event::Snapshot {
                                snapshot: Box::new(st.snap.clone()),
                            }),
                        );
                        if let Some(p) = st.pending.take() {
                            ok &= send_line(&mut writer, &event_line(&p));
                        }
                        if ok {
                            st.boards.push(writer);
                        }
                    }
                    SubKind::Bar => {
                        let line = st.bar_line.clone();
                        if line.is_empty() || send_line(&mut writer, &line) {
                            st.bars.push(writer);
                        }
                    }
                }
                // The stream now belongs to the subscriber list.
                return;
            }
            Request::Toggle { anchor, tab } => {
                to_board(&mut lock.lock().unwrap(), Event::Toggle { anchor, tab });
                Reply::ok()
            }
            Request::Show { anchor, tab } => {
                to_board(&mut lock.lock().unwrap(), Event::Show { anchor, tab });
                Reply::ok()
            }
            Request::Hide => {
                let mut st = lock.lock().unwrap();
                st.pending = None;
                broadcast(&mut st.boards, &event_line(&Event::Hide));
                Reply::ok()
            }
            Request::BoardVisible { visible } => {
                let mut st = lock.lock().unwrap();
                st.visible = visible;
                cv.notify_all();
                Reply::ok()
            }
        };
        if !send_line(
            &mut writer,
            &serde_json::to_string(&reply).unwrap_or_default(),
        ) {
            return;
        }
    }
}

fn sampler(shared: Shared) {
    let mut collector = Collector::new();
    let mut mtime = config_mtime();
    let mut sibling = None;
    let mut sibling_at: Option<Instant> = None;
    loop {
        let (lock, cv) = &*shared;
        let (cfg, visible) = {
            let mut st = lock.lock().unwrap();
            let now_mtime = config_mtime();
            if now_mtime != mtime {
                mtime = now_mtime;
                let (cfg, err) = Config::load();
                st.cfg = cfg;
                st.cfg_error = err;
            }
            (st.cfg.clone(), st.visible)
        };

        let mut snap = collector.sample(visible, &cfg);
        let every = if visible {
            SIBLING_OPEN
        } else {
            SIBLING_CLOSED
        };
        if !cfg.siblings.switchyard {
            sibling = None;
        } else if sibling_at.is_none_or(|t| t.elapsed() >= every) {
            sibling = rh_core::sibling::switchyard();
            sibling_at = Some(Instant::now());
        }
        snap.siblings.switchyard = sibling.clone();
        snap.lamp = rh_core::lamp::evaluate(&snap, &cfg.lamp);

        let mut st = lock.lock().unwrap();
        snap.config_error = st.cfg_error.clone();
        let bar = serde_json::to_string(&crate::bar::line(&snap, &cfg)).unwrap_or_default();
        st.snap = snap;
        let ev = event_line(&Event::Snapshot {
            snapshot: Box::new(st.snap.clone()),
        });
        broadcast(&mut st.boards, &ev);
        if bar != st.bar_line {
            st.bar_line = bar.clone();
            broadcast(&mut st.bars, &bar);
        }
        // Sleep until the next tick, or until the drawer opens.
        let wait = if st.visible { OPEN_EVERY } else { CLOSED_EVERY };
        let was_visible = st.visible;
        let _ = cv
            .wait_timeout_while(st, wait, |s| s.visible == was_visible)
            .unwrap();
    }
}

pub fn run() -> anyhow::Result<i32> {
    let listener = Listener::bind()?;
    let (cfg, cfg_error) = Config::load();
    if let Some(e) = &cfg_error {
        eprintln!("rh: {e}");
    }
    let shared: Shared = Arc::new((
        Mutex::new(State {
            cfg,
            cfg_error,
            snap: Snapshot::default(),
            visible: false,
            boards: vec![],
            bars: vec![],
            bar_line: String::new(),
            pending: None,
            board_started: None,
        }),
        Condvar::new(),
    ));
    {
        let s = shared.clone();
        std::thread::spawn(move || sampler(s));
    }
    eprintln!(
        "rh: listening on {}",
        rh_core::paths::socket_path().display()
    );
    loop {
        match listener.accept() {
            Ok(stream) => {
                let s = shared.clone();
                std::thread::spawn(move || handle(s, stream));
            }
            Err(e) => {
                eprintln!("rh: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}
