//! Temperatures from `/sys/class/hwmon`: the CPU package, each NVMe or SATA
//! drive (matched to its block device) and each AMD GPU (matched to its PCI
//! device, for the GPU collector). Each reading carries the limit the
//! hardware reports for itself, when it reports one.

use super::{read, read_trim};
use std::path::{Path, PathBuf};

/// One sensor: its temperature and the point where the part protects itself
/// (throttles), both in °C.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    pub temp: f32,
    pub limit: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct Temps {
    pub cpu: Option<Reading>,
    /// Block device name (`nvme0n1`, `sda`) and its reading.
    pub drives: Vec<(String, Reading)>,
    /// Resolved PCI device path and its reading (amdgpu "edge").
    pub pci: Vec<(PathBuf, Reading)>,
    /// Every spinning fan: its label (or "chip fanN") and speed in RPM.
    pub fans: Vec<(String, u32)>,
}

/// The fans on one hwmon chip that are spinning. Headers with nothing on
/// them read 0 and are left out.
fn fans(dir: &Path, chip: &str) -> Vec<(String, u32)> {
    (1..=12)
        .filter_map(|n| {
            let rpm = read_trim(dir.join(format!("fan{n}_input")))?
                .parse::<u32>()
                .ok()
                .filter(|r| *r > 0)?;
            let name = read_trim(dir.join(format!("fan{n}_label")))
                .unwrap_or_else(|| format!("{chip} fan{n}"));
            Some((name, rpm))
        })
        .collect()
}

fn millideg(path: &Path) -> Option<f32> {
    read_trim(path)?.parse::<f32>().ok().map(|v| v / 1000.0)
}

/// The `N` of the `tempN_input` whose `tempN_label` is one of `labels`, else
/// 1.
fn sensor(dir: &Path, labels: &[&str]) -> usize {
    (1..=16)
        .find(|n| {
            read_trim(dir.join(format!("temp{n}_label")))
                .is_some_and(|l| labels.iter().any(|x| l.eq_ignore_ascii_case(x)))
        })
        .unwrap_or(1)
}

/// Drives report 65261.85°C for "no limit" on their extra sensors, and some
/// chips report 0; anything outside this range is not a real limit.
fn plausible(limit: f32) -> bool {
    (40.0..=150.0).contains(&limit)
}

/// Sensor `n` of `dir`, with its limit taken from the first of `limits`
/// (`max`, `crit`) that holds a believable value.
fn reading(dir: &Path, labels: &[&str], limits: &[&str]) -> Option<Reading> {
    let n = sensor(dir, labels);
    let temp = millideg(&dir.join(format!("temp{n}_input")))?;
    let limit = limits
        .iter()
        .filter_map(|l| millideg(&dir.join(format!("temp{n}_{l}"))))
        .find(|v| plausible(*v));
    Some(Reading { temp, limit })
}

/// TjMax for AMD CPUs whose limit is not the usual 95°C. `k10temp` does not
/// report one, so it comes from AMD's spec sheets by model.
pub fn amd_tjmax(model: &str) -> Option<f32> {
    let words: Vec<&str> = model.split_whitespace().collect();
    let has = |names: &[&str]| words.iter().any(|w| names.contains(w));
    if has(&["7800X3D", "7900X3D", "7950X3D"]) {
        Some(89.0)
    } else if has(&[
        "5800X3D", "5700X3D", "5600X3D", "5950X", "5900X", "5800X", "5700X",
    ]) {
        Some(90.0)
    } else {
        None
    }
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
        t.fans.extend(fans(&dir, &name));
        match name.as_str() {
            "k10temp" | "zenpower" => {
                if t.cpu.is_none() {
                    t.cpu = reading(&dir, &["Tctl", "Tdie"], &[]);
                }
            }
            "coretemp" => {
                // `crit` is TjMax; `max` is where Core 2 turned the fans up.
                if t.cpu.is_none() {
                    t.cpu = reading(&dir, &["Package id 0"], &["crit"]);
                }
            }
            "cpu_thermal" => {
                if t.cpu.is_none() {
                    t.cpu = reading(&dir, &[], &["crit"]);
                }
            }
            "nvme" | "drivetemp" => {
                // NVMe `max` is the drive's warning threshold (WCTEMP), where
                // it starts to throttle; `crit` (CCTEMP) is past that.
                if let (Some(dev), Some(r)) = (dev, reading(&dir, &["Composite"], &["max", "crit"]))
                {
                    for b in block_names(&dev) {
                        t.drives.push((b, r));
                    }
                }
            }
            "amdgpu" => {
                if let (Some(dev), Some(r)) = (dev, reading(&dir, &["edge"], &["crit"])) {
                    t.pci.push((dev, r));
                }
            }
            _ => {}
        }
    }
    t.drives.sort_by(|a, b| a.0.cmp(&b.0));
    t.fans.sort();
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, files: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).unwrap();
        for (f, v) in files {
            std::fs::write(dir.join(f), format!("{v}\n")).unwrap();
        }
    }

    // Values from a Ryzen 7 9800X3D desktop with a Sabrent and a Samsung 990
    // PRO, and from a Core i7 laptop.
    #[test]
    fn reads_a_fake_tree() {
        let root = std::env::temp_dir().join(format!("rh-hwmon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let hw = root.join("hwmon");
        let devs = root.join("devices");
        write(
            &hw.join("hwmon4"),
            &[
                ("name", "k10temp"),
                ("temp1_label", "Tctl"),
                ("temp1_input", "54250"),
                ("temp3_label", "Tccd1"),
                ("temp3_input", "51000"),
            ],
        );
        // The 990 PRO: Composite plus two sensors that report a 65261°C max.
        write(
            &hw.join("hwmon1"),
            &[
                ("name", "nvme"),
                ("temp1_label", "Composite"),
                ("temp1_input", "49850"),
                ("temp1_max", "81850"),
                ("temp1_crit", "84850"),
                ("temp2_label", "Sensor 1"),
                ("temp2_input", "49850"),
                ("temp2_max", "65261850"),
            ],
        );
        let ctrl = devs.join("nvme1");
        std::fs::create_dir_all(ctrl.join("nvme1n1")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ctrl, hw.join("hwmon1/device")).unwrap();
        let t = read_from(&hw);
        assert_eq!(
            t.cpu,
            Some(Reading {
                temp: 54.25,
                limit: None
            })
        );
        #[cfg(unix)]
        assert_eq!(
            t.drives,
            vec![(
                "nvme1n1".to_string(),
                Reading {
                    temp: 49.85,
                    limit: Some(81.85)
                }
            )]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn coretemp_uses_tjmax() {
        let dir = std::env::temp_dir().join(format!("rh-coretemp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir,
            &[
                ("temp1_label", "Package id 0"),
                ("temp1_input", "45000"),
                ("temp1_max", "80000"),
                ("temp1_crit", "100000"),
                ("temp2_label", "Core 0"),
                ("temp2_input", "43000"),
            ],
        );
        let r = reading(&dir, &["Package id 0"], &["crit"]).unwrap();
        assert_eq!((r.temp, r.limit), (45.0, Some(100.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn amd_limits() {
        assert_eq!(amd_tjmax("Ryzen 7 7800X3D"), Some(89.0));
        assert_eq!(amd_tjmax("Ryzen 9 5950X"), Some(90.0));
        assert_eq!(amd_tjmax("Ryzen 7 9800X3D"), None);
        assert_eq!(amd_tjmax("Core i7-12700K"), None);
    }

    #[test]
    fn spinning_fans_only() {
        // Laid out the way nct6775 reports a desktop board: unlabeled RPM
        // inputs, with 0 on the empty headers.
        let dir = std::env::temp_dir().join(format!("rh-fans-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir,
            &[
                ("fan1_input", "0"),
                ("fan2_input", "1084"),
                ("fan3_input", "742"),
                ("fan3_label", "Pump"),
            ],
        );
        assert_eq!(
            fans(&dir, "nct6798"),
            vec![
                ("nct6798 fan2".to_string(), 1084),
                ("Pump".to_string(), 742)
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
