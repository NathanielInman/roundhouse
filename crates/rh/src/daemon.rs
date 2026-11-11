//! The daemon: samples the machine, keeps the latest snapshot, serves the
//! socket, feeds the bar module, and starts the drawer the first time it is
//! asked for.
//!
//! It samples every second while the drawer is open and every five seconds
//! while it is closed, when only the lamp and the pinned number need data.
//!
//! Nothing writes to a socket while holding the state lock: each subscriber
//! has its own writer thread fed by a small queue, so a frozen drawer can
//! never stall a snapshot reply. Switchyard is polled on its own thread for
//! the same reason.

use rh_core::collect::Collector;
use rh_core::config::Config;
use rh_core::model::Snapshot;
use rh_core::protocol::{Event, Reply, Request, SubKind};
use rh_core::transport::{Listener, Stream};
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
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
    boards: Vec<Sub>,
    bars: Vec<Sub>,
    /// The recording session started with `rh session on`, while one is on.
    session: Option<rh_core::model::Session>,
    /// The session `session.auto` started because an app is recording.
    auto: AutoSession,
    /// Take a sample now rather than at the next tick.
    wake: bool,
    /// The latest answer from Switchyard, kept by its own thread.
    switchyard: Option<rh_core::model::SwitchyardView>,
    bar_line: String,
    /// A show the drawer has not received yet because it is still starting.
    pending: Option<Event>,
    board_started: Option<Instant>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

/// How long a recording must stay stopped before an automatic session ends.
const AUTO_RELEASE: f64 = 30.0;

/// A session started by a recording app (`session.auto`).
#[derive(Default)]
struct AutoSession {
    on: Option<rh_core::model::Session>,
    /// When the recording stopped, while it has.
    quiet_since: Option<f64>,
    /// `rh session off` ended it; stay off until the recording stops.
    dismissed: bool,
}

impl AutoSession {
    /// Follows whether a matching app is recording, at `now`.
    fn update(&mut self, recording: bool, now: f64) {
        if recording {
            self.quiet_since = None;
            if !self.dismissed {
                self.on.get_or_insert(rh_core::model::Session {
                    since: now,
                    auto: true,
                });
            }
            return;
        }
        self.dismissed = false;
        let quiet = *self.quiet_since.get_or_insert(now);
        if now - quiet >= AUTO_RELEASE {
            self.on = None;
        }
    }
}

impl State {
    /// The session in effect: one started by hand wins.
    fn session(&self) -> Option<rh_core::model::Session> {
        self.session.clone().or_else(|| self.auto.on.clone())
    }
}

/// Whether an app is recording from a card that may start a session.
fn recording(audio: &rh_core::model::Audio, cards: &[String]) -> bool {
    audio
        .streams
        .iter()
        .any(|s| s.capture && (cards.is_empty() || cards.iter().any(|c| c == &s.card)))
}

/// A subscriber: the queue its writer thread drains.
type Sub = SyncSender<Arc<str>>;

/// Lines a subscriber may fall behind before it misses some.
const QUEUE: usize = 8;

fn send_line(s: &mut Stream, line: &str) -> bool {
    s.write_all(line.as_bytes())
        .and_then(|_| s.write_all(b"\n"))
        .and_then(|_| s.flush())
        .is_ok()
}

/// Gives a subscribed stream its own writer thread. The thread ends, and the
/// queue disconnects, when a write fails or times out.
fn subscriber(mut stream: Stream, first: &[String]) -> Option<Sub> {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let (tx, rx) = sync_channel::<Arc<str>>(QUEUE);
    for line in first {
        tx.try_send(line.as_str().into()).ok()?;
    }
    std::thread::spawn(move || {
        for line in rx {
            if !send_line(&mut stream, &line) {
                return;
            }
        }
    });
    Some(tx)
}

/// Queues a line for every subscriber without blocking, and drops the ones
/// that went away. A subscriber whose queue is full skips this line. Returns
/// how many are still subscribed.
fn broadcast(subs: &mut Vec<Sub>, line: &str) -> usize {
    let line: Arc<str> = line.into();
    subs.retain(|s| !matches!(s.try_send(line.clone()), Err(TrySendError::Disconnected(_))));
    subs.len()
}

fn event_line(e: &Event) -> String {
    serde_json::to_string(e).unwrap_or_default()
}

fn load_session() -> Option<rh_core::model::Session> {
    let text = std::fs::read_to_string(rh_core::paths::session_file()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Keeps the session on disk so it survives a daemon restart.
fn save_session(s: Option<&rh_core::model::Session>) {
    let path = rh_core::paths::session_file();
    match s {
        Some(s) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, serde_json::to_string(s).unwrap_or_default());
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
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
                let mut st = lock.lock().unwrap();
                match kind {
                    SubKind::Board => {
                        let mut first = vec![event_line(&Event::Snapshot {
                            snapshot: Box::new(st.snap.clone()),
                        })];
                        if let Some(p) = st.pending.take() {
                            first.push(event_line(&p));
                        }
                        if let Some(sub) = subscriber(writer, &first) {
                            st.boards.push(sub);
                        }
                    }
                    SubKind::Bar => {
                        let first: Vec<String> = Some(st.bar_line.clone())
                            .filter(|l| !l.is_empty())
                            .into_iter()
                            .collect();
                        if let Some(sub) = subscriber(writer, &first) {
                            st.bars.push(sub);
                        }
                    }
                }
                // The stream now belongs to its writer thread.
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
            Request::Session { on } => {
                let mut st = lock.lock().unwrap();
                let on = on.unwrap_or(st.session().is_none());
                if on != st.session().is_some() {
                    st.session = on.then(|| rh_core::model::Session {
                        since: rh_core::now(),
                        auto: false,
                    });
                    if !on && st.auto.on.take().is_some() {
                        st.auto.dismissed = true;
                    }
                    save_session(st.session.as_ref());
                    st.wake = true;
                    cv.notify_all();
                }
                let mut snap = st.snap.clone();
                snap.session = st.session();
                snap.lamp.hold = rh_core::lamp::hold(&snap.lamp, snap.session.as_ref());
                Reply {
                    snapshot: Some(Box::new(snap)),
                    ..Reply::ok()
                }
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

/// Polls Switchyard every 2 s while the drawer is open and every 15 s while
/// it is closed. A sibling that does not answer costs this thread its 300 ms
/// deadline, never a sample.
fn sibling_watcher(shared: Shared) {
    let (lock, cv) = &*shared;
    loop {
        let (on, visible) = {
            let st = lock.lock().unwrap();
            (st.cfg.siblings.switchyard, st.visible)
        };
        let view = if on {
            rh_core::sibling::switchyard()
        } else {
            None
        };
        let mut st = lock.lock().unwrap();
        st.switchyard = view;
        let every = if visible {
            SIBLING_OPEN
        } else {
            SIBLING_CLOSED
        };
        let _ = cv
            .wait_timeout_while(st, every, |s| s.visible == visible)
            .unwrap();
    }
}

fn sampler(shared: Shared) {
    let mut collector = Collector::new();
    let mut mtime = config_mtime();
    let mut lamp = rh_core::lamp::LampState::new();
    let (lock, cv) = &*shared;
    // Ticks keep a fixed rate: the next one is due an interval after the
    // last was due, not an interval after the work finished.
    let mut due = Instant::now();
    let mut was_visible = false;
    loop {
        let (cfg, cfg_error, visible, switchyard, watch) = {
            let mut st = lock.lock().unwrap();
            st.wake = false;
            let now_mtime = config_mtime();
            if now_mtime != mtime {
                mtime = now_mtime;
                let (cfg, err) = Config::load();
                st.cfg = cfg;
                st.cfg_error = err;
            }
            (
                st.cfg.clone(),
                st.cfg_error.clone(),
                st.visible,
                st.switchyard.clone(),
                st.session().is_some(),
            )
        };

        collector.watch_audio = watch;
        let mut snap = collector.sample(visible, &cfg);
        let session = {
            let mut st = lock.lock().unwrap();
            if cfg.session.auto {
                let rec = recording(&snap.audio, &cfg.session.cards);
                st.auto.update(rec, snap.now);
            } else {
                st.auto = AutoSession::default();
            }
            st.session()
        };
        snap.session = session.clone();
        snap.siblings.switchyard = if cfg.siblings.switchyard {
            switchyard
        } else {
            None
        };
        snap.lamp = lamp.update(&snap, &cfg.lamp);
        snap.lamp.hold = rh_core::lamp::hold(&snap.lamp, session.as_ref());
        snap.config_error = cfg_error;
        let bar = serde_json::to_string(&crate::bar::line(&snap, &cfg)).unwrap_or_default();
        let ev = event_line(&Event::Snapshot {
            snapshot: Box::new(snap.clone()),
        });

        let mut st = lock.lock().unwrap();
        st.snap = snap;
        broadcast(&mut st.boards, &ev);
        if bar != st.bar_line {
            st.bar_line = bar.clone();
            broadcast(&mut st.bars, &bar);
        }
        // Sleep until the next tick, or until the drawer opens or closes.
        let every = if st.visible { OPEN_EVERY } else { CLOSED_EVERY };
        let now = Instant::now();
        due = if st.visible != was_visible { now } else { due } + every;
        if due < now {
            // Fell behind (a suspend, a slow share): start over from now.
            due = now + every;
        }
        was_visible = st.visible;
        let wait = due - now;
        let _ = cv
            .wait_timeout_while(st, wait, |s| s.visible == was_visible && !s.wake)
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
            switchyard: None,
            session: load_session(),
            auto: AutoSession::default(),
            wake: false,
        }),
        Condvar::new(),
    ));
    {
        let s = shared.clone();
        std::thread::spawn(move || sampler(s));
        let s = shared.clone();
        std::thread::spawn(move || sibling_watcher(s));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_sessions_follow_the_recording() {
        let mut a = AutoSession::default();
        a.update(true, 0.0);
        assert!(a.on.as_ref().is_some_and(|s| s.auto && s.since == 0.0));
        a.update(false, 10.0);
        assert!(a.on.is_some());
        a.update(true, 20.0);
        a.update(false, 25.0);
        assert!(a.on.is_some());
        a.update(false, 55.0);
        assert!(a.on.is_none());
    }

    #[test]
    fn a_dismissed_session_waits_for_the_next_recording() {
        let mut a = AutoSession::default();
        a.update(true, 0.0);
        a.on = None;
        a.dismissed = true;
        a.update(true, 5.0);
        assert!(a.on.is_none());
        a.update(false, 6.0);
        a.update(true, 7.0);
        assert!(a.on.is_some());
    }

    #[test]
    fn only_listed_cards_start_a_session() {
        let mut audio = rh_core::model::Audio::default();
        audio.streams.push(rh_core::model::AudioStream {
            card: "Cam Link 4K".into(),
            capture: true,
            ..Default::default()
        });
        assert!(recording(&audio, &[]));
        assert!(!recording(&audio, &["Yeti X".into()]));
    }
}
