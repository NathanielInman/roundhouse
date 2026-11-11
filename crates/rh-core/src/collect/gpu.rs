//! GPUs: NVIDIA through NVML (loaded at runtime, so a machine without the
//! driver just has no NVIDIA entries) and AMD through amdgpu's sysfs files.
//! The discrete card with the most memory comes first.

use super::{push_hist, read_trim};
use crate::collect::hwmon::Temps;
use crate::model::Gpu;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct GpuReader {
    #[cfg(target_os = "linux")]
    nvml: Option<nvml_wrapper::Nvml>,
    nvml_tried: bool,
    names: HashMap<PathBuf, String>,
    /// NVML slowdown temperature by device index; it never changes.
    #[cfg(target_os = "linux")]
    nv_limits: HashMap<u32, Option<f32>>,
    hist: HashMap<String, VecDeque<f32>>,
}

impl GpuReader {
    pub fn sample(&mut self, temps: &Temps) -> Vec<Gpu> {
        let mut out = self.nvidia();
        out.extend(self.amd(temps));
        for g in &mut out {
            let h = self.hist.entry(g.name.clone()).or_default();
            push_hist(h, g.usage.unwrap_or(0.0));
            g.history = h.iter().copied().collect();
        }
        out.sort_by_key(|g| std::cmp::Reverse(g.vram_total.unwrap_or(0)));
        out
    }

    #[cfg(target_os = "linux")]
    fn nvidia(&mut self) -> Vec<Gpu> {
        use nvml_wrapper::bitmasks::device::ThrottleReasons;
        use nvml_wrapper::enum_wrappers::device::{TemperatureSensor, TemperatureThreshold};
        if !self.nvml_tried {
            self.nvml_tried = true;
            self.nvml = nvml_wrapper::Nvml::init().ok();
        }
        let Some(nvml) = &self.nvml else {
            return vec![];
        };
        let count = nvml.device_count().unwrap_or(0);
        let limits = &mut self.nv_limits;
        (0..count)
            .filter_map(|i| nvml.device_by_index(i).ok().map(|d| (i, d)))
            .map(|(i, d)| {
                let mem = d.memory_info().ok();
                let temp_limit = *limits.entry(i).or_insert_with(|| {
                    d.temperature_threshold(TemperatureThreshold::Slowdown)
                        .ok()
                        .map(|t| t as f32)
                });
                Gpu {
                    temp_limit,
                    name: d
                        .name()
                        .map(|n| short_gpu(&n))
                        .unwrap_or_else(|_| "NVIDIA GPU".into()),
                    vendor: "nvidia".into(),
                    usage: d.utilization_rates().ok().map(|u| u.gpu as f32),
                    temp: d.temperature(TemperatureSensor::Gpu).ok().map(|t| t as f32),
                    vram_used: mem.as_ref().map(|m| m.used),
                    vram_total: mem.as_ref().map(|m| m.total),
                    power_w: d.power_usage().ok().map(|mw| mw as f32 / 1000.0),
                    fan: d.fan_speed(0).ok().map(|f| f as f32),
                    throttling: d.current_throttle_reasons().is_ok_and(|r| {
                        r.intersects(
                            ThrottleReasons::HW_THERMAL_SLOWDOWN
                                | ThrottleReasons::SW_THERMAL_SLOWDOWN,
                        )
                    }),
                    history: vec![],
                }
            })
            .collect()
    }

    #[cfg(not(target_os = "linux"))]
    fn nvidia(&mut self) -> Vec<Gpu> {
        let _ = self.nvml_tried;
        vec![]
    }

    fn amd(&mut self, temps: &Temps) -> Vec<Gpu> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir("/sys/class/drm") else {
            return out;
        };
        let mut cards: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit())
            })
            .collect();
        cards.sort();
        for card in cards {
            let dev = card.join("device");
            if read_trim(dev.join("vendor")).as_deref() != Some("0x1002") {
                continue;
            }
            let Some(busy) =
                read_trim(dev.join("gpu_busy_percent")).and_then(|v| v.parse::<f32>().ok())
            else {
                continue;
            };
            let real = std::fs::canonicalize(&dev).unwrap_or(dev.clone());
            let name = self
                .names
                .entry(real.clone())
                .or_insert_with(|| amd_name(&dev))
                .clone();
            let reading = temps.pci.iter().find(|(p, _)| *p == real).map(|(_, r)| *r);
            let num = |f: &str| read_trim(dev.join(f)).and_then(|v| v.parse::<u64>().ok());
            out.push(Gpu {
                name,
                vendor: "amd".into(),
                usage: Some(busy),
                temp: reading.map(|r| r.temp),
                temp_limit: reading.and_then(|r| r.limit),
                vram_used: num("mem_info_vram_used"),
                vram_total: num("mem_info_vram_total"),
                power_w: None,
                throttling: false,
                fan: None,
                history: vec![],
            });
        }
        out
    }
}

/// "NVIDIA GeForce RTX 3080" reads as "RTX 3080".
pub fn short_gpu(name: &str) -> String {
    let s: Vec<&str> = name
        .split_whitespace()
        .filter(|w| !matches!(*w, "NVIDIA" | "GeForce" | "AMD" | "ATI"))
        .collect();
    let s = s.join(" ");
    let s = s.trim_start_matches('[').trim_end_matches(']').to_string();
    if s.is_empty() {
        name.to_string()
    } else {
        s
    }
}

/// The marketing name from the PCI ID database, which hwdata installs on
/// Arch: "Granite Ridge [Radeon 610M]" reads as "Radeon 610M".
fn amd_name(dev: &Path) -> String {
    let id = read_trim(dev.join("device")).map(|d| d.trim_start_matches("0x").to_lowercase());
    let db = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok());
    if let (Some(id), Some(db)) = (id, db) {
        if let Some(name) = pci_name(&db, "1002", &id) {
            return name;
        }
    }
    "AMD GPU".into()
}

pub fn pci_name(db: &str, vendor: &str, device: &str) -> Option<String> {
    let mut in_vendor = false;
    for line in db.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            in_vendor = line.starts_with(vendor);
            continue;
        }
        if in_vendor && !line.starts_with("\t\t") {
            let l = line.trim_start();
            if let Some(rest) = l.strip_prefix(device) {
                let full = rest.trim();
                let short = match (full.find('['), full.rfind(']')) {
                    (Some(a), Some(b)) if b > a => full[a + 1..b].to_string(),
                    _ => full.to_string(),
                };
                return Some(short);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(short_gpu("NVIDIA GeForce RTX 3080"), "RTX 3080");
        let db = "1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t13c0  Granite Ridge [Radeon 610M]\n\t\t1043 8877  sub\n10de  NVIDIA Corporation\n\t2206  GA102 [GeForce RTX 3080]\n";
        assert_eq!(pci_name(db, "1002", "13c0").as_deref(), Some("Radeon 610M"));
        assert_eq!(pci_name(db, "1002", "2206"), None);
    }
}
