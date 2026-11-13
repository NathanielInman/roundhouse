//! The waybar line: one lamp glyph (and a pinned number, if asked for) with
//! a class that follows the lamp so the bar's CSS can color it.

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

/// The glyph for a lamp level, falling back to `bar.icon`. A recording
/// session's glyph wins while the lamp is calm.
pub fn icon(cfg: &Config, level: Level, session: bool) -> String {
    let b = &cfg.bar;
    let pick = match level {
        Level::Calm if session => b.icon_session.as_ref(),
        Level::Calm => None,
        Level::HeadsUp => b.icon_heads_up.as_ref(),
        Level::Act => b.icon_act.as_ref(),
    };
    pick.unwrap_or(&b.icon).clone()
}

pub fn line(snap: &Snapshot, cfg: &Config) -> BarLine {
    let mut text = icon(cfg, snap.lamp.level, snap.session.is_some());
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
    if let Some(s) = &snap.session {
        tip.push(format!(
            "Recording session · {} · heavy builds wait",
            rh_core::duration(snap.now - s.since)
        ));
    }
    tip.push(String::new());
    let t = &cfg.lamp;
    let mut cpu = format!("CPU {:.0}%", m.cpu.usage);
    if let Some(temp) = m.cpu.temp {
        cpu.push_str(&format!(" · {}", t.cpu_temp(m.cpu.temp_limit).show(temp)));
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
        if let Some(temp) = g.temp {
            s.push_str(&format!(" · {}", t.gpu_temp(g.temp_limit).show(temp)));
        }
        tip.push(s);
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
        text: cfg
            .bar
            .icon_offline
            .clone()
            .unwrap_or_else(|| cfg.bar.icon.clone()),
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
    fn calm_line_is_just_the_lamp() {
        let mut s = Snapshot::default();
        s.machine.cpu.temp = Some(54.4);
        s.machine.cpu.temp_limit = Some(95.0);
        let l = line(&s, &Config::default());
        assert_eq!(l.text, Config::default().bar.icon);
        assert_eq!(l.class, "calm");
        assert!(l.tooltip.starts_with("All calm"));
        assert!(l.tooltip.contains("CPU 0% · 54 / 95°C"));
    }

    #[test]
    fn pins_a_number() {
        let mut s = Snapshot::default();
        s.machine.cpu.temp = Some(54.4);
        let mut cfg = Config::default();
        cfg.bar.pin = "cpu.temp".into();
        assert!(line(&s, &cfg).text.ends_with(" 54°"));
    }

    #[test]
    fn reasons_reach_the_tooltip() {
        let mut s = Snapshot::default();
        s.lamp.level = Level::Act;
        s.lamp.reasons.push(Reason {
            level: Level::Act,
            text: "/ is 96% full & <rising>".into(),
            tab: "storage".into(),
            key: "fs:/".into(),
        });
        let cfg = Config::default();
        let l = line(&s, &cfg);
        assert_eq!(l.text, cfg.bar.icon);
        assert_eq!(l.class, "act");
        assert!(l.tooltip.contains("96% full &amp; &lt;rising&gt;"));
    }

    #[test]
    fn each_level_can_have_its_own_glyph() {
        let mut cfg = Config::default();
        cfg.bar.icon_act = Some("!".into());
        cfg.bar.icon_session = Some("R".into());
        let mut s = Snapshot::default();
        assert_eq!(line(&s, &cfg).text, cfg.bar.icon);
        s.session = Some(rh_core::model::Session::default());
        assert_eq!(line(&s, &cfg).text, "R");
        s.lamp.level = Level::Act;
        assert_eq!(line(&s, &cfg).text, "!");
        // Unset levels fall back to the plain icon.
        s.lamp.level = Level::HeadsUp;
        assert_eq!(line(&s, &cfg).text, cfg.bar.icon);
        cfg.bar.icon_offline = Some("x".into());
        assert_eq!(offline(&cfg).text, "x");
    }
}
