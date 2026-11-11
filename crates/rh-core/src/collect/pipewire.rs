//! PipeWire's profiler, read over PipeWire's own socket with no library and
//! no subprocess: the quantum, the DSP load and, above all, xruns (audio
//! dropouts).
//!
//! The native protocol is small enough to speak directly. Each message is a
//! 16-byte header (object id, opcode and size, sequence, fd count) and an SPA
//! POD. The client says hello, asks for the registry, binds the global whose
//! type is `PipeWire:Interface:Profiler`, and then receives one `profile`
//! event per batch of graph cycles.
//!
//! Binding the profiler turns profiling on in PipeWire's graph, which costs
//! it a little on every cycle, so the watcher connects only while someone
//! wants the numbers (the drawer is open, or a recording session is on) and
//! hangs up otherwise.

use crate::model::AudioGraph;
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One decoded SPA POD.
#[derive(Debug, Clone, PartialEq)]
pub enum Pod {
    None,
    Bool(bool),
    Id(u32),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Str(String),
    Fraction(u32, u32),
    Struct(Vec<Pod>),
    Object {
        otype: u32,
        id: u32,
        props: Vec<(u32, Pod)>,
    },
    Other(u32),
}

impl Pod {
    fn int(&self) -> Option<i64> {
        match self {
            Pod::Int(v) => Some(i64::from(*v)),
            Pod::Long(v) => Some(*v),
            _ => None,
        }
    }

    fn float(&self) -> Option<f32> {
        match self {
            Pod::Float(v) => Some(*v),
            Pod::Double(v) => Some(*v as f32),
            _ => None,
        }
    }

    fn items(&self) -> &[Pod] {
        match self {
            Pod::Struct(v) => v,
            _ => &[],
        }
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Decodes the POD at the start of `b` and returns it with the offset of the
/// next one (PODs are padded to 8 bytes).
pub fn decode(b: &[u8]) -> Option<(Pod, usize)> {
    let size = u32_at(b, 0)? as usize;
    let ty = u32_at(b, 4)?;
    let body = b.get(8..8 + size)?;
    let next = 8 + size.div_ceil(8) * 8;
    let n4 = || body.get(..4).and_then(|x| x.try_into().ok());
    let n8 = || body.get(..8).and_then(|x| x.try_into().ok());
    let pod = match ty {
        1 => Pod::None,
        2 => Pod::Bool(u32::from_le_bytes(n4()?) != 0),
        3 => Pod::Id(u32::from_le_bytes(n4()?)),
        4 => Pod::Int(i32::from_le_bytes(n4()?)),
        5 => Pod::Long(i64::from_le_bytes(n8()?)),
        6 => Pod::Float(f32::from_le_bytes(n4()?)),
        7 => Pod::Double(f64::from_le_bytes(n8()?)),
        8 => {
            let end = body.iter().position(|c| *c == 0).unwrap_or(body.len());
            Pod::Str(String::from_utf8_lossy(&body[..end]).into_owned())
        }
        11 => Pod::Fraction(u32_at(body, 0)?, u32_at(body, 4)?),
        14 => {
            let mut items = vec![];
            let mut at = 0;
            while at < body.len() {
                let (p, n) = decode(&body[at..])?;
                items.push(p);
                at += n;
            }
            Pod::Struct(items)
        }
        15 => {
            let (otype, id) = (u32_at(body, 0)?, u32_at(body, 4)?);
            let mut props = vec![];
            let mut at = 8;
            while at < body.len() {
                let key = u32_at(body, at)?;
                let (p, n) = decode(&body[at + 8..])?;
                props.push((key, p));
                at += 8 + n;
            }
            Pod::Object { otype, id, props }
        }
        other => Pod::Other(other),
    };
    Some((pod, next))
}

fn pad(b: &mut Vec<u8>) {
    while !b.len().is_multiple_of(8) {
        b.push(0);
    }
}

fn enc_int(v: i32) -> Vec<u8> {
    let mut b = vec![];
    b.extend(4u32.to_le_bytes());
    b.extend(4u32.to_le_bytes());
    b.extend(v.to_le_bytes());
    pad(&mut b);
    b
}

fn enc_str(s: &str) -> Vec<u8> {
    let mut b = vec![];
    b.extend((s.len() as u32 + 1).to_le_bytes());
    b.extend(8u32.to_le_bytes());
    b.extend(s.as_bytes());
    b.push(0);
    pad(&mut b);
    b
}

fn enc_struct(items: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = items.concat();
    let mut b = vec![];
    b.extend((body.len() as u32).to_le_bytes());
    b.extend(14u32.to_le_bytes());
    b.extend(body);
    b
}

/// One native protocol message.
pub fn message(id: u32, opcode: u8, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![];
    b.extend(id.to_le_bytes());
    b.extend(((u32::from(opcode) << 24) | payload.len() as u32).to_le_bytes());
    b.extend(seq.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend(payload);
    b
}

/// One graph cycle of one driver, from a profile event.
#[derive(Debug, Clone, PartialEq)]
pub struct Cycle {
    /// The driver node: the device the graph runs on.
    pub driver: String,
    /// Samples per cycle.
    pub quantum: i64,
    pub rate: u32,
    /// Share of the cycle spent processing (the fast average), 0-1.
    pub load: f32,
    /// The driver's running xrun count.
    pub xruns: i64,
}

const PROFILER_INFO: u32 = 0x10001;
const PROFILER_CLOCK: u32 = 0x10002;
const PROFILER_DRIVER_BLOCK: u32 = 0x10003;

/// Pulls the cycles out of a profile event's payload: a struct holding a
/// struct of profiler objects, one per driver cycle.
pub fn parse_profile(payload: &[u8]) -> Vec<Cycle> {
    let Some((outer, _)) = decode(payload) else {
        return vec![];
    };
    let mut out = vec![];
    let objects = outer.items().iter().flat_map(|p| match p {
        Pod::Struct(v) => v.as_slice(),
        other => std::slice::from_ref(other),
    });
    for obj in objects {
        let Pod::Object { props, .. } = obj else {
            continue;
        };
        let get = |k: u32| {
            props
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, p)| p.items())
        };
        let (Some(info), Some(clock)) = (get(PROFILER_INFO), get(PROFILER_CLOCK)) else {
            continue;
        };
        let driver = get(PROFILER_DRIVER_BLOCK)
            .and_then(|d| match d.get(1) {
                Some(Pod::Str(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let rate = match clock.get(4) {
            Some(Pod::Fraction(_, den)) => *den,
            _ => 0,
        };
        out.push(Cycle {
            driver,
            quantum: clock.get(6).and_then(Pod::int).unwrap_or(0),
            rate,
            load: info.get(1).and_then(Pod::float).unwrap_or(0.0),
            xruns: info.get(4).and_then(Pod::int).unwrap_or(0),
        });
    }
    out
}

#[derive(Default)]
struct Tally {
    last: HashMap<String, i64>,
    recent: VecDeque<(Instant, u32)>,
    latest: HashMap<String, Cycle>,
    at: Option<Instant>,
}

impl Tally {
    fn add(&mut self, c: Cycle, now: Instant) {
        if let Some(prev) = self.last.insert(c.driver.clone(), c.xruns) {
            if c.xruns > prev {
                self.recent.push_back((now, (c.xruns - prev) as u32));
            }
        }
        self.latest.insert(c.driver.clone(), c);
        self.at = Some(now);
    }

    fn graph(&mut self, now: Instant) -> Option<AudioGraph> {
        while self
            .recent
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(60))
        {
            self.recent.pop_front();
        }
        // Nothing heard for a while: the graph is idle or we are gone.
        if self
            .at
            .is_none_or(|t| now.duration_since(t) > Duration::from_secs(5))
        {
            return None;
        }
        let busiest = self
            .latest
            .values()
            .max_by(|a, b| a.load.total_cmp(&b.load))?;
        Some(AudioGraph {
            driver: busiest.driver.clone(),
            quantum: busiest.quantum,
            rate: busiest.rate,
            load: busiest.load * 100.0,
            xruns_minute: self.recent.iter().map(|(_, n)| n).sum(),
        })
    }
}

/// Watches PipeWire on a helper thread while `want` is set.
pub struct Watcher {
    want: Arc<AtomicBool>,
    tally: Arc<Mutex<Tally>>,
    started: bool,
}

impl Default for Watcher {
    fn default() -> Self {
        Watcher {
            want: Arc::new(AtomicBool::new(false)),
            tally: Arc::new(Mutex::new(Tally::default())),
            started: false,
        }
    }
}

impl Watcher {
    /// Sets whether the numbers are wanted and returns the latest ones.
    pub fn sample(&mut self, want: bool) -> Option<AudioGraph> {
        self.want.store(want, Ordering::Relaxed);
        if want && !self.started {
            self.started = true;
            let (w, t) = (self.want.clone(), self.tally.clone());
            std::thread::spawn(move || watch(w, t));
        }
        if !want {
            return None;
        }
        self.tally.lock().unwrap().graph(Instant::now())
    }
}

#[cfg(unix)]
fn socket_path() -> Option<std::path::PathBuf> {
    let dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))?;
    let name = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    Some(std::path::PathBuf::from(dir).join(name))
}

/// Connects while wanted, reconnects after a failure, and hangs up when no
/// longer wanted.
#[cfg(unix)]
fn watch(want: Arc<AtomicBool>, tally: Arc<Mutex<Tally>>) {
    loop {
        if want.load(Ordering::Relaxed) {
            let _ = session(&want, &tally);
            *tally.lock().unwrap() = Tally::default();
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(not(unix))]
fn watch(_want: Arc<AtomicBool>, _tally: Arc<Mutex<Tally>>) {}

const CORE: u32 = 0;
const CLIENT: u32 = 1;
const REGISTRY: u32 = 2;
const PROFILER: u32 = 3;
const PROFILER_TYPE: &str = "PipeWire:Interface:Profiler";

#[cfg(unix)]
fn session(want: &AtomicBool, tally: &Mutex<Tally>) -> Option<()> {
    let mut s = std::os::unix::net::UnixStream::connect(socket_path()?).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut seq = 0u32;
    let mut send = |s: &mut std::os::unix::net::UnixStream, id: u32, op: u8, payload: Vec<u8>| {
        seq += 1;
        s.write_all(&message(id, op, seq, &payload)).ok()
    };
    // Core.Hello(version 4), Client.UpdateProperties, Core.GetRegistry.
    send(&mut s, CORE, 1, enc_struct(&[enc_int(4)]))?;
    let props = enc_struct(&[
        enc_int(1),
        enc_str("application.name"),
        enc_str("roundhouse"),
    ]);
    send(&mut s, CLIENT, 2, enc_struct(&[props]))?;
    send(
        &mut s,
        CORE,
        5,
        enc_struct(&[enc_int(3), enc_int(REGISTRY as i32)]),
    )?;

    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut bound = false;
    while want.load(Ordering::Relaxed) {
        let n = match s.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return None,
        };
        buf.extend_from_slice(&chunk[..n]);
        while let (Some(id), Some(opsize)) = (u32_at(&buf, 0), u32_at(&buf, 4)) {
            let size = (opsize & 0x00ff_ffff) as usize;
            if buf.len() < 16 + size {
                break;
            }
            let op = (opsize >> 24) as u8;
            let payload: Vec<u8> = buf[16..16 + size].to_vec();
            buf.drain(..16 + size);
            match (id, op) {
                // Registry.Global: bind the profiler.
                (REGISTRY, 0) if !bound => {
                    let Some((Pod::Struct(g), _)) = decode(&payload) else {
                        continue;
                    };
                    if let (Some(Pod::Int(gid)), Some(Pod::Str(ty))) = (g.first(), g.get(2)) {
                        if ty == PROFILER_TYPE {
                            bound = true;
                            send(
                                &mut s,
                                REGISTRY,
                                1,
                                enc_struct(&[
                                    enc_int(*gid),
                                    enc_str(PROFILER_TYPE),
                                    enc_int(3),
                                    enc_int(PROFILER as i32),
                                ]),
                            )?;
                        }
                    }
                }
                (PROFILER, 0) => {
                    let now = Instant::now();
                    let mut t = tally.lock().unwrap();
                    for c in parse_profile(&payload) {
                        t.add(c, now);
                    }
                }
                // Core.Ping: answer with Pong(id, seq) or be dropped.
                (CORE, 2) => {
                    if let Some((Pod::Struct(p), _)) = decode(&payload) {
                        if let (Some(Pod::Int(a)), Some(Pod::Int(b))) = (p.first(), p.get(1)) {
                            send(&mut s, CORE, 3, enc_struct(&[enc_int(*a), enc_int(*b)]))?;
                        }
                    }
                }
                // Core.Error on the core itself: give up.
                (CORE, 3) => {
                    if let Some((Pod::Struct(p), _)) = decode(&payload) {
                        if matches!(p.first(), Some(Pod::Int(0))) {
                            return None;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // One profile event from PipeWire 1.6 on a desktop: a null sink driving
    // the graph at 512 samples and 48 kHz, with a game as its follower.
    const PROFILE: &[u8] = include_bytes!("../../testdata/pipewire-profile.bin");

    #[test]
    fn reads_a_real_profile() {
        let c = parse_profile(PROFILE);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].driver, "agents_mute");
        assert_eq!((c[0].quantum, c[0].rate), (512, 48000));
        assert_eq!(c[0].xruns, 0);
        assert!(c[0].load > 0.0 && c[0].load < 0.01);
    }

    #[test]
    fn encodes_what_it_decodes() {
        let s = enc_struct(&[enc_int(7), enc_str("roundhouse")]);
        let (p, n) = decode(&s).unwrap();
        assert_eq!(n, s.len());
        assert_eq!(
            p,
            Pod::Struct(vec![Pod::Int(7), Pod::Str("roundhouse".into())])
        );
        let m = message(2, 1, 9, &s);
        assert_eq!(u32_at(&m, 4), Some((1 << 24) | s.len() as u32));
        assert!(decode(&[1, 0, 0]).is_none());
    }

    #[test]
    fn counts_xruns_over_a_minute() {
        let mut t = Tally::default();
        let start = Instant::now();
        let cycle = |x: i64| Cycle {
            driver: "alsa_output.usb".into(),
            quantum: 256,
            rate: 48000,
            load: 0.2,
            xruns: x,
        };
        t.add(cycle(4), start);
        t.add(cycle(4), start + Duration::from_secs(1));
        t.add(cycle(7), start + Duration::from_secs(2));
        let g = t.graph(start + Duration::from_secs(3)).unwrap();
        assert_eq!(g.xruns_minute, 3);
        assert_eq!(g.quantum, 256);
        assert!((g.load - 20.0).abs() < 1e-4);
        t.add(cycle(7), start + Duration::from_secs(70));
        assert_eq!(
            t.graph(start + Duration::from_secs(70))
                .unwrap()
                .xruns_minute,
            0
        );
        // Silence for more than 5 s: no graph to report.
        assert!(t.graph(start + Duration::from_secs(80)).is_none());
    }
}
