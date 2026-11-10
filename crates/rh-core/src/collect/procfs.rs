//! Parsers for `/proc` files. Pure functions over text, tested against real
//! output.

use crate::model::Psi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

/// The aggregate `cpu` line of `/proc/stat`. Idle counts iowait too.
pub fn parse_stat(text: &str) -> Option<CpuTimes> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 5 {
        return None;
    }
    // user nice system idle iowait irq softirq steal (guest is already in user)
    let total: u64 = v.iter().take(8).sum();
    Some(CpuTimes {
        idle: v[3] + v[4],
        total,
    })
}

/// Percent busy between two samples.
pub fn usage(prev: CpuTimes, cur: CpuTimes) -> f32 {
    let dt = cur.total.saturating_sub(prev.total);
    if dt == 0 {
        return 0.0;
    }
    let di = cur.idle.saturating_sub(prev.idle).min(dt);
    (1.0 - di as f32 / dt as f32) * 100.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

pub fn parse_meminfo(text: &str) -> MemInfo {
    let mut m = MemInfo::default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else {
            continue;
        };
        let bytes = val.parse::<u64>().unwrap_or(0) * 1024;
        match key {
            "MemTotal:" => m.total = bytes,
            "MemAvailable:" => m.available = bytes,
            "SwapTotal:" => m.swap_total = bytes,
            "SwapFree:" => m.swap_free = bytes,
            _ => {}
        }
    }
    m
}

/// A `/proc/pressure/*` file.
pub fn parse_psi(text: &str) -> Option<Psi> {
    let mut p = Psi::default();
    let mut seen = false;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let kind = it.next()?;
        for kv in it {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            let Ok(v) = v.parse::<f32>() else { continue };
            let slot = match (kind, k) {
                ("some", "avg10") => &mut p.some10,
                ("some", "avg60") => &mut p.some60,
                ("some", "avg300") => &mut p.some300,
                ("full", "avg10") => &mut p.full10,
                ("full", "avg60") => &mut p.full60,
                ("full", "avg300") => &mut p.full300,
                _ => continue,
            };
            *slot = v;
            seen = true;
        }
    }
    seen.then_some(p)
}

/// Model name, thread count and average clock from `/proc/cpuinfo`.
pub fn cpu_info(text: &str) -> (String, usize, Option<f32>) {
    let mut model = String::new();
    let mut threads = 0;
    let (mut sum, mut n) = (0.0f32, 0);
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "processor" => threads += 1,
            "model name" if model.is_empty() => model = v.trim().to_string(),
            "cpu MHz" => {
                if let Ok(x) = v.trim().parse::<f32>() {
                    sum += x;
                    n += 1;
                }
            }
            _ => {}
        }
    }
    (short_cpu(&model), threads, (n > 0).then(|| sum / n as f32))
}

/// "AMD Ryzen 7 9800X3D 8-Core Processor" reads as "Ryzen 7 9800X3D".
pub fn short_cpu(model: &str) -> String {
    let mut s = model
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace("(tm)", "");
    for cut in [" with ", " CPU @", " @ "] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    let words: Vec<&str> = s
        .split_whitespace()
        .filter(|w| !matches!(*w, "AMD" | "Intel" | "Processor" | "CPU"))
        .filter(|w| !w.ends_with("-Core"))
        .collect();
    let out = words.join(" ");
    if out.is_empty() {
        model.trim().to_string()
    } else {
        out
    }
}

/// `PRETTY_NAME` from `/etc/os-release`.
pub fn os_name(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage() {
        let a = parse_stat("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
        let b = parse_stat("cpu  150 0 150 900 0 0 0 0 0 0\n").unwrap();
        assert_eq!(
            a,
            CpuTimes {
                idle: 800,
                total: 1000
            }
        );
        assert_eq!(usage(a, b), 50.0);
        assert_eq!(usage(a, a), 0.0);
    }

    #[test]
    fn meminfo() {
        let m = parse_meminfo("MemTotal:       63436824 kB\nMemFree: 1 kB\nMemAvailable:   51213456 kB\nSwapTotal:      15728636 kB\nSwapFree:       15728636 kB\n");
        assert_eq!(m.total, 63_436_824 * 1024);
        assert_eq!(m.available, 51_213_456 * 1024);
        assert_eq!(m.swap_total, m.swap_free);
    }

    #[test]
    fn psi() {
        let p = parse_psi("some avg10=0.24 avg60=0.66 avg300=0.35 total=985478126\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n").unwrap();
        assert_eq!(p.some60, 0.66);
        assert_eq!(p.full10, 0.0);
        assert!(parse_psi("").is_none());
    }

    #[test]
    fn cpu_names() {
        assert_eq!(
            short_cpu("AMD Ryzen 7 9800X3D 8-Core Processor"),
            "Ryzen 7 9800X3D"
        );
        assert_eq!(
            short_cpu("Intel(R) Core(TM) i7-1185G7 @ 3.00GHz"),
            "Core i7-1185G7"
        );
        assert_eq!(
            short_cpu("AMD Ryzen 7 7840U w/ Radeon 780M Graphics"),
            "Ryzen 7 7840U w/ Radeon 780M Graphics"
        );
        let (m, t, mhz) = cpu_info("processor\t: 0\nmodel name\t: AMD Ryzen 7 9800X3D 8-Core Processor\ncpu MHz\t\t: 3000.000\nprocessor\t: 1\ncpu MHz\t\t: 5000.000\n");
        assert_eq!((m.as_str(), t, mhz), ("Ryzen 7 9800X3D", 2, Some(4000.0)));
    }

    #[test]
    fn os() {
        assert_eq!(
            os_name("NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n"),
            "Arch Linux"
        );
    }
}
