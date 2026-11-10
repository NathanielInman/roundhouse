//! Temperatures from `/sys/class/hwmon`: the CPU package, each NVMe or SATA
//! drive (matched to its block device) and each AMD GPU (matched to its PCI
//! device, for the GPU collector).

use super::{read, read_trim};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Temps {
    pub cpu: Option<f32>,
    /// Block device name (`nvme0n1`, `sda`) and its temperature.
    pub drives: Vec<(String, f32)>,
    /// Resolved PCI device path and its temperature (amdgpu "edge").
    pub pci: Vec<(PathBuf, f32)>,
}

fn millideg(path: &Path) -> Option<f32> {
    read_trim(path)?.parse::<f32>().ok().map(|v| v / 1000.0)
}

/// The `tempN_input` whose `tempN_label` is one of `labels`, else `temp1`.
fn labelled(dir: &Path, labels: &[&str]) -> Option<f32> {
    for n in 1..=16 {
        let Some(label) = read_trim(dir.join(format!("temp{n}_label"))) else {
            continue;
        };
        if labels.iter().any(|l| label.eq_ignore_ascii_case(l)) {
            return millideg(&dir.join(format!("temp{n}_input")));
        }
    }
    millideg(&dir.join("temp1_input"))
}

/// Block devices under a device directory: NVMe namespaces next to the
/// controller (`nvme0` has `nvme0n1`), or `block/sdX` under a SCSI device.
fn block_names(dev: &Path) -> Vec<String> {
    let mut out = vec![];
    let ctrl = dev
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if let Ok(rd) = std::fs::read_dir(dev) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !ctrl.is_empty() && name.starts_with(&format!("{ctrl}n")) {
                out.push(name);
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(dev.join("block")) {
        out.extend(
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string()),
        );
    }
    out.sort();
    out
}

pub fn scan() -> Temps {
    read_from(Path::new("/sys/class/hwmon"))
}

pub fn read_from(root: &Path) -> Temps {
    let mut t = Temps::default();
    let Ok(rd) = std::fs::read_dir(root) else {
        return t;
    };
    for e in rd.flatten() {
        let dir = e.path();
        let Some(name) = read(dir.join("name")).map(|n| n.trim().to_string()) else {
            continue;
        };
        let dev = std::fs::canonicalize(dir.join("device")).ok();
        match name.as_str() {
            "k10temp" | "zenpower" => {
                if t.cpu.is_none() {
                    t.cpu = labelled(&dir, &["Tctl", "Tdie"]);
                }
            }
            "coretemp" => {
                if t.cpu.is_none() {
                    t.cpu = labelled(&dir, &["Package id 0"]);
                }
            }
            "cpu_thermal" => {
                if t.cpu.is_none() {
                    t.cpu = millideg(&dir.join("temp1_input"));
                }
            }
            "nvme" | "drivetemp" => {
                if let (Some(dev), Some(temp)) = (dev, labelled(&dir, &["Composite"])) {
                    for b in block_names(&dev) {
                        t.drives.push((b, temp));
                    }
                }
            }
            "amdgpu" => {
                if let (Some(dev), Some(temp)) = (dev, labelled(&dir, &["edge"])) {
                    t.pci.push((dev, temp));
                }
            }
            _ => {}
        }
    }
    t.drives.sort_by(|a, b| a.0.cmp(&b.0));
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_fake_tree() {
        let root = std::env::temp_dir().join(format!("rh-hwmon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let hw = root.join("hwmon");
        let devs = root.join("devices");
        let cpu = hw.join("hwmon4");
        std::fs::create_dir_all(&cpu).unwrap();
        std::fs::write(cpu.join("name"), "k10temp\n").unwrap();
        std::fs::write(cpu.join("temp1_label"), "Tctl\n").unwrap();
        std::fs::write(cpu.join("temp1_input"), "54250\n").unwrap();
        let ctrl = devs.join("nvme0");
        std::fs::create_dir_all(ctrl.join("nvme0n1")).unwrap();
        let nv = hw.join("hwmon0");
        std::fs::create_dir_all(&nv).unwrap();
        std::fs::write(nv.join("name"), "nvme\n").unwrap();
        std::fs::write(nv.join("temp1_label"), "Composite\n").unwrap();
        std::fs::write(nv.join("temp1_input"), "43850\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ctrl, nv.join("device")).unwrap();
        let t = read_from(&hw);
        assert_eq!(t.cpu, Some(54.25));
        #[cfg(unix)]
        assert_eq!(t.drives, vec![("nvme0n1".to_string(), 43.85)]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
