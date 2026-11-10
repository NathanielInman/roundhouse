//! The waybar line: one icon, an optional pinned number, and a class that
//! follows the lamp so the bar's CSS can color it.

use rh_core::config::Config;
use rh_core::model::{Level, Snapshot};
use rh_core::protocol::BarLine;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The number beside the icon, from `bar.pin`.
pub fn pinned(snap: &Snapshot, pin: &str) -> Option<String> {
    let m = &snap.machine;
    let gpu = m.gpus.first();
    match pin {
        "cpu.temp" => m.cpu.temp.map(|t| format!("{t:.0}°")),
        "cpu" => Some(format!("{:.0}%", m.cpu.usage)),
        "memory" => Some(format!("{:.0}%", m.memory.used_pct())),
        "gpu.temp" => gpu.and_then(|g| g.temp).map(|t| format!("{t:.0}°")),
        "gpu" => gpu.and_then(|g| g.usage).map(|u| format!("{u:.0}%")),
        _ => None,
    }
}

pub fn line(snap: &Snapshot, cfg: &Config) -> BarLine {
    let mut text = cfg.bar.icon.clone();
    if let Some(p) = pinned(snap, &cfg.bar.pin) {
        text.push(' ');
        text.push_str(&p);
    }
    let level = snap.lamp.level;
    let m = &snap.machine;
    let mut tip = vec![];
    if level == Level::Calm {
        tip.push("All calm".to_string());
    } else {
        tip.extend(snap.lamp.reasons.iter().map(|r| format!("• {}", r.text)));
    }
    tip.push(String::new());
    let mut cpu = format!("CPU {:.0}%", m.cpu.usage);
    if let Some(t) = m.cpu.temp {
        cpu.push_str(&format!(" · {t:.0}°C"));
    }
    tip.push(cpu);
    tip.push(format!(
        "Memory {} / {}",
        rh_core::human(m.memory.used()),
        rh_core::human(m.memory.total)
    ));
    if let Some(g) = m.gpus.first() {
        let mut s = g.name.clone();
        if let Some(u) = g.usage {
            s.push_str(&format!(" {u:.0}%"));
        }
        if let Some(t) = g.temp {
            s.push_str(&format!(" · {t:.0}°C"));
        }
        tip.push(s);
    }
    if let Some(y) = &snap.siblings.switchyard {
        if let Some(cur) = &y.current {
            tip.push(format!("Switchyard: building {}", y.label(cur)));
        }
    }
    BarLine {
        text,
        class: level.as_str().into(),
        alt: level.as_str().into(),
        tooltip: escape(&tip.join("\n")),
    }
}

/// What the bar shows while the daemon cannot be reached.
pub fn offline(cfg: &Config) -> BarLine {
    BarLine {
        text: cfg.bar.icon.clone(),
        class: "offline".into(),
        alt: "offline".into(),
        tooltip: "Roundhouse daemon is not running".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rh_core::model::Reason;

    #[test]
    fn calm_line_pins_cpu_temp() {
        let mut s = Snapshot::default();
        s.machine.cpu.temp = Some(54.4);
        let l = line(&s, &Config::default());
        assert!(l.text.ends_with(" 54°"));
        assert_eq!(l.class, "calm");
        assert!(l.tooltip.starts_with("All calm"));
    }

    #[test]
    fn reasons_reach_the_tooltip() {
        let mut s = Snapshot::default();
        s.lamp.level = Level::Act;
        s.lamp.reasons.push(Reason {
            level: Level::Act,
            text: "/ is 96% full & <rising>".into(),
            tab: "storage".into(),
        });
        let mut cfg = Config::default();
        cfg.bar.pin = "none".into();
        let l = line(&s, &cfg);
        assert_eq!(l.text, cfg.bar.icon);
        assert_eq!(l.class, "act");
        assert!(l.tooltip.contains("96% full &amp; &lt;rising&gt;"));
    }
}
