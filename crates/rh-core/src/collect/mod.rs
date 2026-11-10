//! Collectors. Each one reads the kernel's own files (`/proc`, `/sys`) or a
//! library loaded at runtime (NVML), never a subprocess, so a sample costs
//! microseconds and the daemon stays out of its own top list.
//!
//! On systems without those files every collector comes back empty, which is
//! what macOS and Windows get until their collectors land.

pub mod apps;
pub mod gpu;
pub mod hwmon;
pub mod procfs;
pub mod storage;

use crate::config::Config;
use crate::model::{AppUsage, Cpu, Disk, Host, Machine, Memory, Pressure, Snapshot};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Samples kept for sparklines.
pub const HISTORY: usize = 60;

/// Storage is re-read this often while the drawer is closed (it only feeds
/// the lamp then).
const STORAGE_IDLE: Duration = Duration::from_secs(30);

pub(crate) fn push_hist(h: &mut VecDeque<f32>, v: f32) {
    if h.len() == HISTORY {
        h.pop_front();
    }
    h.push_back(v);
}

pub(crate) fn read(path: impl AsRef<std::path::Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

pub(crate) fn read_trim(path: impl AsRef<std::path::Path>) -> Option<String> {
    read(path)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Holds what one sample needs from the previous one: CPU counters, history
/// rings, the NVML handle and in-flight share probes.
pub struct Collector {
    prev_cpu: Option<procfs::CpuTimes>,
    cpu_hist: VecDeque<f32>,
    mem_hist: VecDeque<f32>,
    cpu_model: Option<(String, usize)>,
    mhz: Option<f32>,
    gpus: gpu::GpuReader,
    storage: storage::Storage,
    disks: Vec<Disk>,
    disks_at: Option<Instant>,
    apps: Vec<AppUsage>,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    pub fn new() -> Collector {
        Collector {
            prev_cpu: None,
            cpu_hist: VecDeque::with_capacity(HISTORY),
            mem_hist: VecDeque::with_capacity(HISTORY),
            cpu_model: None,
            mhz: None,
            gpus: gpu::GpuReader::default(),
            storage: storage::Storage::default(),
            disks: vec![],
            disks_at: None,
            apps: vec![],
        }
    }

    /// Takes one sample. `deep` is true while the drawer is open: it adds
    /// memory by app and clock speeds, and refreshes storage every time.
    pub fn sample(&mut self, deep: bool, cfg: &Config) -> Snapshot {
        let host = Host {
            name: read_trim("/proc/sys/kernel/hostname").unwrap_or_else(|| "localhost".into()),
            os: read("/etc/os-release")
                .map(|t| procfs::os_name(&t))
                .unwrap_or_default(),
            kernel: read_trim("/proc/sys/kernel/osrelease").unwrap_or_default(),
            uptime: read("/proc/uptime")
                .and_then(|t| t.split_whitespace().next().and_then(|v| v.parse().ok()))
                .unwrap_or_default(),
        };

        // CPU
        let mut usage = 0.0;
        if let Some(cur) = read("/proc/stat").and_then(|t| procfs::parse_stat(&t)) {
            if let Some(prev) = self.prev_cpu {
                usage = procfs::usage(prev, cur);
                push_hist(&mut self.cpu_hist, usage);
            }
            self.prev_cpu = Some(cur);
        }
        if self.cpu_model.is_none() || deep {
            if let Some(info) = read("/proc/cpuinfo") {
                let (model, threads, mhz) = procfs::cpu_info(&info);
                self.cpu_model = Some((model, threads));
                self.mhz = mhz;
            }
        }
        let temps = hwmon::scan();
        let (model, threads) = self.cpu_model.clone().unwrap_or_default();
        let cpu = Cpu {
            model,
            threads,
            usage,
            temp: temps.cpu,
            mhz: self.mhz,
            history: self.cpu_hist.iter().copied().collect(),
        };

        // Memory
        let mi = read("/proc/meminfo")
            .map(|t| procfs::parse_meminfo(&t))
            .unwrap_or_default();
        let memory = Memory {
            total: mi.total,
            available: mi.available,
            swap_total: mi.swap_total,
            swap_used: mi.swap_total.saturating_sub(mi.swap_free),
            history: vec![],
        };
        push_hist(&mut self.mem_hist, memory.used_pct());
        let memory = Memory {
            history: self.mem_hist.iter().copied().collect(),
            ..memory
        };

        let pressure = Pressure {
            cpu: read("/proc/pressure/cpu").and_then(|t| procfs::parse_psi(&t)),
            memory: read("/proc/pressure/memory").and_then(|t| procfs::parse_psi(&t)),
            io: read("/proc/pressure/io").and_then(|t| procfs::parse_psi(&t)),
        };

        let gpus = self.gpus.sample(&temps);

        // Storage: every sample while open, every 30 s for the lamp otherwise.
        if deep || self.disks_at.is_none_or(|t| t.elapsed() >= STORAGE_IDLE) {
            self.disks = self.storage.sample(&cfg.storage.hide, &temps.drives);
            self.disks_at = Some(Instant::now());
        }
        if deep {
            self.apps = apps::sample(8);
        }

        Snapshot {
            host,
            machine: Machine {
                cpu,
                memory,
                pressure,
                gpus,
            },
            disks: self.disks.clone(),
            apps: if deep { self.apps.clone() } else { vec![] },
            now: crate::now(),
            ..Default::default()
        }
    }
}
