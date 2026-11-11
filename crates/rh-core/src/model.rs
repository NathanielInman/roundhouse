//! The snapshot the daemon publishes. Clients (drawer, bar module, CLI,
//! siblings) read it; only the daemon writes it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Snapshot {
    pub host: Host,
    pub machine: Machine,
    pub disks: Vec<Disk>,
    /// Memory by app, largest first. Filled while the drawer is open.
    pub apps: Vec<AppUsage>,
    pub lamp: Lamp,
    pub siblings: Siblings,
    /// A config problem to show in the drawer.
    #[serde(default)]
    pub config_error: Option<String>,
    /// The recording session, while one is on.
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub audio: Audio,
    /// Unix seconds when the sample was taken.
    pub now: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Host {
    pub name: String,
    pub os: String,
    pub kernel: String,
    pub uptime: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Machine {
    pub cpu: Cpu,
    pub memory: Memory,
    pub pressure: Pressure,
    pub gpus: Vec<Gpu>,
    #[serde(default)]
    pub network: Network,
    /// When each recent sample was taken (Unix seconds), oldest first. Every
    /// `history` series ends at the same sample as this list, so sparklines
    /// can be drawn against time: samples come every 5 s while the drawer is
    /// closed and every second while it is open.
    #[serde(default)]
    pub times: Vec<f64>,
}

/// Throughput summed over the interfaces backed by real devices.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Network {
    /// Interfaces that are up.
    pub interfaces: Vec<String>,
    /// Bytes per second.
    pub down: f64,
    pub up: f64,
    pub down_history: Vec<f32>,
    pub up_history: Vec<f32>,
    /// Only when `network.public_ip` is on.
    pub public_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Cpu {
    pub model: String,
    pub threads: usize,
    /// Percent busy across all threads since the last sample.
    pub usage: f32,
    pub temp: Option<f32>,
    /// The temperature where the CPU throttles (TjMax), when known.
    #[serde(default)]
    pub temp_limit: Option<f32>,
    pub mhz: Option<f32>,
    /// The CPU reported thermal throttling since the last sample (Intel's
    /// `thermal_throttle` counters; AMD reports none).
    #[serde(default)]
    pub throttling: bool,
    /// Recent usage, oldest first.
    pub history: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    /// Recent used percentage, oldest first.
    pub history: Vec<f32>,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn used_pct(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f32 / self.total as f32 * 100.0
        }
    }
}

/// Pressure stall information: the share of time something waited on a
/// resource. `some` means at least one task waited; `full` means all did.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct Psi {
    pub some10: f32,
    pub some60: f32,
    pub some300: f32,
    pub full10: f32,
    pub full60: f32,
    pub full300: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Pressure {
    pub cpu: Option<Psi>,
    pub memory: Option<Psi>,
    pub io: Option<Psi>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Gpu {
    pub name: String,
    pub vendor: String,
    pub usage: Option<f32>,
    pub temp: Option<f32>,
    /// The temperature where the card slows itself down, as it reports it.
    #[serde(default)]
    pub temp_limit: Option<f32>,
    pub vram_used: Option<u64>,
    pub vram_total: Option<u64>,
    pub power_w: Option<f32>,
    /// The card says it is slowing down for heat right now (NVML).
    #[serde(default)]
    pub throttling: bool,
    pub history: Vec<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiskKind {
    #[default]
    Local,
    Network,
    /// An automount point with nothing mounted on it right now.
    OnDemand,
}

/// One physical disk (or network share) and the filesystems on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Disk {
    /// Kernel name (`nvme0n1`) or share source (`//192.168.1.51/private`).
    pub name: String,
    pub model: Option<String>,
    pub kind: DiskKind,
    pub temp: Option<f32>,
    /// The drive's own warning temperature (NVMe WCTEMP), as it reports it.
    #[serde(default)]
    pub temp_limit: Option<f32>,
    pub filesystems: Vec<Filesystem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Filesystem {
    pub source: String,
    pub fstype: String,
    /// Mount points, the main one first.
    pub mounts: Vec<String>,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
    pub readonly: bool,
    /// The last size query did not answer in time (a share that went away).
    #[serde(default)]
    pub stale: bool,
}

impl Filesystem {
    pub fn used_pct(&self) -> f32 {
        let denom = self.used + self.avail;
        if denom == 0 {
            0.0
        } else {
            self.used as f32 / denom as f32 * 100.0
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AppUsage {
    pub name: String,
    /// "38 procs", "scope", "service".
    pub detail: String,
    pub mem: u64,
    /// The memory limit of the app's cgroup, when it has one (lowprio).
    pub cap: Option<u64>,
    pub procs: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    #[default]
    Calm,
    HeadsUp,
    Act,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Calm => "calm",
            Level::HeadsUp => "heads-up",
            Level::Act => "act",
        }
    }
}

/// Where one temperature sensor turns the lamp yellow and red, in °C.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempLimit {
    pub warn: f32,
    pub act: f32,
}

impl TempLimit {
    pub fn level(&self, temp: f32) -> Level {
        if temp >= self.act {
            Level::Act
        } else if temp >= self.warn {
            Level::HeadsUp
        } else {
            Level::Calm
        }
    }

    /// "58 / 95°C": the reading next to the point where the part throttles.
    pub fn show(&self, temp: f32) -> String {
        format!("{temp:.0} / {:.0}°C", self.act)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Reason {
    pub level: Level,
    pub text: String,
    /// The drawer tab that explains it.
    pub tab: String,
    /// Which rule raised it (`cpu.temp`, `fs:/data`), stable across samples.
    #[serde(default)]
    pub key: String,
}

/// The bar lamp: the most urgent level and every reason behind it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Lamp {
    pub level: Level,
    pub reasons: Vec<Reason>,
    /// Why heavy work should wait right now, or `null`. Switchyard holds its
    /// next heavy build while this is set. It is set by a recording session
    /// or by an act reason that more work would make worse (disk, memory);
    /// a hot part is not one, since the build is what heats it. Always sent,
    /// so a sibling can tell a calm answer from an older Roundhouse.
    #[serde(default)]
    pub hold: Option<String>,
}

/// What the sound cards are doing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Audio {
    /// Running PCM streams.
    pub streams: Vec<AudioStream>,
    /// Hardware MIDI devices that are plugged in.
    pub midi: Vec<String>,
    /// PipeWire's graph, while the drawer is open or a session is on.
    #[serde(default)]
    pub graph: Option<AudioGraph>,
}

impl Audio {
    /// Something is recording from a sound card.
    pub fn capturing(&self) -> bool {
        self.streams.iter().any(|s| s.capture)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AudioStream {
    /// The card's name ("Yeti X").
    pub card: String,
    /// `card4/pcm0c`.
    pub device: String,
    /// Recording rather than playing.
    pub capture: bool,
    pub rate: Option<u32>,
    pub format: String,
    pub channels: Option<u32>,
    /// The process holding the device (`pipewire`, or an app using ALSA
    /// directly).
    pub owner: Option<String>,
    /// One period, when an app holds the device itself. Behind a sound
    /// server the server's quantum is what counts.
    pub latency_ms: Option<f32>,
}

/// PipeWire's graph, from its profiler.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AudioGraph {
    /// The driver node the graph runs on.
    pub driver: String,
    /// Samples per cycle.
    pub quantum: i64,
    pub rate: u32,
    /// Percent of each cycle spent processing.
    pub load: f32,
    /// Xruns (dropouts) in the last minute.
    pub xruns_minute: u32,
}

impl AudioGraph {
    /// One cycle, in milliseconds.
    pub fn latency_ms(&self) -> f32 {
        if self.rate == 0 {
            0.0
        } else {
            self.quantum as f32 / self.rate as f32 * 1000.0
        }
    }
}

/// A recording session: heavy work waits until it ends, without turning the
/// lamp red.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Session {
    /// Unix seconds when it started.
    pub since: f64,
    /// Started on its own because an app is recording (`session.auto`),
    /// rather than with `rh session on`.
    #[serde(default)]
    pub auto: bool,
}

/// What the siblings that are running right now report. A missing sibling is
/// `None`, never an error.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Siblings {
    pub switchyard: Option<SwitchyardView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SwitchyardView {
    /// Switchyard's aggregate phase: idle, queued, building, running, ok,
    /// failed.
    pub phase: String,
    pub tasks: usize,
    /// The heavy build on the main line.
    pub current: Option<String>,
    pub queue: Vec<String>,
    pub waiting_memory: bool,
    pub building: Vec<String>,
    pub running: Vec<String>,
    pub failed: Vec<String>,
    /// Task id to label.
    pub labels: Vec<(String, String)>,
    /// Why the next heavy build is waiting, in Switchyard's words ("held:
    /// Roundhouse says ...", "waiting for 16 G free ..."), while it is.
    #[serde(default)]
    pub waiting: Option<String>,
}

impl SwitchyardView {
    pub fn label<'a>(&'a self, id: &'a str) -> &'a str {
        self.labels
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, l)| l.as_str())
            .unwrap_or(id)
    }

    /// One line for the Machine tab while Switchyard is busy with heavy
    /// work, since a build explains a busy machine. `None` while it is not.
    pub fn status_line(&self) -> Option<String> {
        if let Some(cur) = &self.current {
            let rest = self.queue.iter().filter(|q| *q != cur).count();
            let mut s = format!("Switchyard is building {}", self.label(cur));
            if rest > 0 {
                s.push_str(&format!(" · {rest} queued"));
            }
            return Some(s);
        }
        if let (Some(head), Some(why)) = (self.queue.first(), &self.waiting) {
            let name = self.label(head);
            return Some(match why.strip_prefix("held: ") {
                Some(r) => format!("Switchyard is holding {name}: {r}"),
                None => format!("Switchyard: {name} is {why}"),
            });
        }
        if !self.building.is_empty() {
            let names: Vec<&str> = self.building.iter().map(|b| self.label(b)).collect();
            return Some(format!("Switchyard is building {}", names.join(", ")));
        }
        None
    }
}
