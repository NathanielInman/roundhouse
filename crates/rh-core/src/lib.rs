//! Everything Roundhouse knows that is not drawing: where files live, the
//! config, the collectors, the lamp rules, the socket protocol, the waybar
//! reader and the Switchyard sibling client.

pub mod collect;
pub mod config;
pub mod model;
pub mod paths;
pub mod protocol;
pub mod transport;

pub use bar_anchor::{Anchor, Edge};

/// Formats a byte count the way `df -h` does: 1024-based, one decimal under 10.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{v:.0} {}", UNITS[i])
    }
}

/// Formats seconds as "3d 4h", "14h 39m" or "12m".
pub fn duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (d, h, m) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Unix time in seconds.
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(137 * 1024 * 1024 * 1024), "137 G");
        assert_eq!(human(6_600 * 1024 * 1024 * 1024), "6.4 T");
        assert_eq!(human(1536), "1.5 K");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(52_740.0), "14h 39m");
        assert_eq!(duration(3.0 * 86_400.0 + 4.0 * 3600.0), "3d 4h");
        assert_eq!(duration(720.0), "12m");
    }
}
