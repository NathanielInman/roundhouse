//! The bar lamp: one color for the whole machine. Calm unless something
//! needs you, with every reason listed so the drawer can say why.
//!
//! Two layers. `evaluate` is pure: it turns one snapshot into reasons, each
//! with the level its rule gives right now. `LampState` sits on top in the
//! daemon and gives the light an envelope, so it does not chatter:
//!
//! - Hysteresis: a level holds until its value drops a margin under the line
//!   that raised it (3°C, one point of disk, two points of pressure).
//! - Attack: a temperature must stay past its heads-up line for `attack`
//!   seconds, and at its limit for `sustain` seconds, before the lamp rises.
//!   CPUs boost until they reach their limit, so touching it is normal; sitting
//!   there is throttling. A part that says it is throttling skips the wait.
//! - Release: a condition must stay clear for `release` seconds before the
//!   lamp falls.

use crate::config::LampConfig;
use crate::model::{DiskKind, Lamp, Level, Reason, Snapshot, TempLimit};
use std::collections::HashMap;

/// How far under a line a value must fall before the level it raised clears.
const TEMP_MARGIN: f32 = 3.0;
const DISK_MARGIN: f32 = 1.0;
const PRESSURE_MARGIN: f32 = 2.0;
const FREE_MARGIN: f32 = 1.0;

/// What kind of rule raised a reason, which sets its attack times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A temperature: heads-up after `attack`, act after `sustain`.
    Temp,
    /// Already averaged or slow to change (disk use, PSI, a throttle report):
    /// no attack.
    Steady,
}

/// One rule's verdict on one snapshot.
#[derive(Debug, Clone, PartialEq)]
struct Raw {
    reason: Reason,
    kind: Kind,
}

/// The level of `value` against `warn` and `act`, holding `prev` until the
/// value falls `margin` under the line that raised it.
fn classify(value: f32, warn: f32, act: f32, prev: Level, margin: f32) -> Level {
    if value >= act || (prev == Level::Act && value >= act - margin) {
        Level::Act
    } else if value >= warn || (prev >= Level::HeadsUp && value >= warn - margin) {
        Level::HeadsUp
    } else {
        Level::Calm
    }
}

struct Rules<'a> {
    out: Vec<Raw>,
    prev: &'a dyn Fn(&str) -> Level,
}

impl Rules<'_> {
    fn push(&mut self, key: String, level: Level, tab: &str, text: String, kind: Kind) {
        if level == Level::Calm {
            return;
        }
        self.out.push(Raw {
            reason: Reason {
                level,
                text,
                tab: tab.into(),
                key,
            },
            kind,
        });
    }

    /// A temperature at or past its heads-up line, said against the limit so
    /// the number means something.
    fn hot(&mut self, key: String, tab: &str, name: &str, temp: f32, limit: TempLimit) {
        let level = classify(temp, limit.warn, limit.act, (self.prev)(&key), TEMP_MARGIN);
        let text = format!("{name} is at {temp:.0}°C (limit {:.0}°C)", limit.act);
        self.push(key, level, tab, text, Kind::Temp);
    }
}

fn rules(s: &Snapshot, t: &LampConfig, prev: &dyn Fn(&str) -> Level) -> Vec<Raw> {
    let mut r = Rules { out: vec![], prev };
    let m = &s.machine;

    for disk in &s.disks {
        if disk.kind == DiskKind::OnDemand {
            continue;
        }
        for fs in &disk.filesystems {
            if fs.stale || fs.readonly || fs.total == 0 {
                continue;
            }
            let pct = fs.used_pct();
            let mount = fs.mounts.first().map(String::as_str).unwrap_or(&fs.source);
            let key = format!("fs:{mount}");
            let level = classify(pct, t.disk_warn, t.disk_act, prev(&key), DISK_MARGIN);
            r.push(
                key,
                level,
                "storage",
                format!("{mount} is {pct:.0}% full"),
                Kind::Steady,
            );
        }
        if let Some(temp) = disk.temp {
            let name = disk.model.as_deref().unwrap_or(&disk.name);
            r.hot(
                format!("drive:{}", disk.name),
                "storage",
                name,
                temp,
                t.drive_temp(disk.temp_limit),
            );
        }
    }

    if let Some(temp) = m.cpu.temp {
        r.hot(
            "cpu.temp".into(),
            "machine",
            "CPU",
            temp,
            t.cpu_temp(m.cpu.temp_limit),
        );
    }
    if m.cpu.throttling {
        r.push(
            "cpu.throttle".into(),
            Level::Act,
            "machine",
            "CPU is slowing itself down for heat".into(),
            Kind::Steady,
        );
    }
    for g in &m.gpus {
        if let Some(temp) = g.temp {
            r.hot(
                format!("gpu:{}", g.name),
                "machine",
                &g.name,
                temp,
                t.gpu_temp(g.temp_limit),
            );
        }
        if g.throttling {
            r.push(
                format!("gpu.throttle:{}", g.name),
                Level::Act,
                "machine",
                format!("{} is slowing itself down for heat", g.name),
                Kind::Steady,
            );
        }
    }

    if m.memory.total > 0 {
        let free = m.memory.available as f32 / m.memory.total as f32 * 100.0;
        // Lower is worse: classify how far under the line it is.
        let key = "mem.free".to_string();
        let level = classify(
            -free,
            f32::INFINITY,
            -t.mem_free_act,
            prev(&key),
            FREE_MARGIN,
        );
        r.push(
            key,
            level,
            "machine",
            format!("only {free:.0}% of memory is free"),
            Kind::Steady,
        );
    }
    if let Some(p) = m.pressure.memory {
        let key = "mem.pressure".to_string();
        let was = prev(&key);
        let act = classify(
            p.full10,
            f32::INFINITY,
            t.mem_pressure_act,
            was,
            PRESSURE_MARGIN,
        );
        if act == Level::Act {
            r.push(
                key,
                Level::Act,
                "machine",
                format!(
                    "everything is stalling on memory ({:.0}% of the last 10 s)",
                    p.full10
                ),
                Kind::Steady,
            );
        } else {
            let level = classify(
                p.some60,
                t.mem_pressure_warn,
                f32::INFINITY,
                was.min(Level::HeadsUp),
                PRESSURE_MARGIN,
            );
            r.push(
                key,
                level,
                "machine",
                format!(
                    "tasks are waiting on memory ({:.0}% of the last minute)",
                    p.some60
                ),
                Kind::Steady,
            );
        }
    }
    // Fans are noise in the take: worth a light only while recording.
    if s.session.is_some() {
        let mut loud: Vec<String> = m
            .fans
            .iter()
            .filter(|f| f.rpm >= t.fan_loud_rpm)
            .map(|f| format!("{} {} RPM", f.name, f.rpm))
            .collect();
        loud.extend(m.gpus.iter().filter_map(|g| {
            let f = g.fan?;
            (f >= t.fan_loud_pct).then(|| format!("{} fan {f:.0}%", g.name))
        }));
        if !loud.is_empty() {
            r.push(
                "fans.loud".into(),
                Level::HeadsUp,
                "machine",
                format!("fans are loud: {}", loud.join(", ")),
                Kind::Temp,
            );
        }
    }

    // Dropouts matter while recording; outside a session a stray xrun from a
    // Bluetooth headset is not worth a light.
    if s.session.is_some() {
        if let Some(g) = &s.audio.graph {
            if g.xruns_minute > 0 {
                let n = g.xruns_minute;
                let what = if n == 1 { "dropout" } else { "dropouts" };
                r.push(
                    "audio.xruns".into(),
                    Level::HeadsUp,
                    "machine",
                    format!("{n} audio {what} in the last minute"),
                    Kind::Steady,
                );
            }
        }
    }
    r.out
}

fn lamp_of(mut reasons: Vec<Reason>) -> Lamp {
    reasons.sort_by_key(|x| std::cmp::Reverse(x.level));
    let level = reasons.first().map(|x| x.level).unwrap_or(Level::Calm);
    Lamp {
        level,
        reasons,
        hold: None,
    }
}

/// What `lamp.hold` says during a recording session.
pub const SESSION_HOLD: &str = "a recording session is on";

/// Rules about heat. More work only adds heat, and a part at its limit
/// protects itself, so these never hold builds.
fn about_heat(key: &str) -> bool {
    ["cpu.", "gpu", "drive:"].iter().any(|p| key.starts_with(p))
}

/// Why heavy work should wait: a recording session, else the first act
/// reason that more work would make worse.
pub fn hold(lamp: &Lamp, session: Option<&crate::model::Session>) -> Option<String> {
    if session.is_some() {
        return Some(SESSION_HOLD.into());
    }
    lamp.reasons
        .iter()
        .find(|r| r.level == Level::Act && !about_heat(&r.key))
        .map(|r| r.text.clone())
}

/// Works out the lamp from one snapshot, with no memory of earlier ones.
/// Pure, so the rules are easy to test; `rh status` without a daemon uses it.
pub fn evaluate(s: &Snapshot, t: &LampConfig) -> Lamp {
    lamp_of(
        rules(s, t, &|_| Level::Calm)
            .into_iter()
            .map(|r| r.reason)
            .collect(),
    )
}

/// One rule's envelope.
#[derive(Debug, Clone, Default)]
struct Track {
    /// What the lamp shows for this rule.
    shown: Level,
    /// The rule's own level on the last sample (hysteresis input).
    raw: Level,
    /// When the raw level first reached heads-up and act, while it has.
    above: [Option<f64>; 2],
    /// When the raw level last dropped under heads-up and act, while it has.
    below: [Option<f64>; 2],
    /// The latest words for this rule, kept while the lamp releases.
    reason: Option<Reason>,
}

fn slot(l: Level) -> Option<usize> {
    match l {
        Level::Calm => None,
        Level::HeadsUp => Some(0),
        Level::Act => Some(1),
    }
}

/// The lamp with an envelope: hysteresis, attack and release. The daemon keeps
/// one and feeds it every snapshot.
#[derive(Debug, Default)]
pub struct LampState {
    tracks: HashMap<String, Track>,
}

impl LampState {
    pub fn new() -> LampState {
        LampState::default()
    }

    /// The lamp for this snapshot, taken at `s.now` (Unix seconds).
    pub fn update(&mut self, s: &Snapshot, t: &LampConfig) -> Lamp {
        let now = s.now;
        let raws = {
            let tracks = &self.tracks;
            rules(s, t, &|k| {
                tracks.get(k).map(|tr| tr.raw).unwrap_or_default()
            })
        };
        let mut seen: HashMap<String, Raw> = raws
            .into_iter()
            .map(|r| (r.reason.key.clone(), r))
            .collect();
        for key in seen.keys() {
            self.tracks.entry(key.clone()).or_default();
        }
        let mut out = vec![];
        self.tracks.retain(|key, tr| {
            let raw = seen.remove(key);
            let level = raw.as_ref().map_or(Level::Calm, |r| r.reason.level);
            let kind = raw.as_ref().map_or(Kind::Steady, |r| r.kind);
            tr.raw = level;
            for (i, l) in [Level::HeadsUp, Level::Act].into_iter().enumerate() {
                if level >= l {
                    tr.above[i].get_or_insert(now);
                    tr.below[i] = None;
                } else {
                    tr.above[i] = None;
                    tr.below[i].get_or_insert(now);
                }
            }
            if let Some(r) = raw {
                tr.reason = Some(r.reason);
            }
            let attack = |l: Level| match (kind, l) {
                (Kind::Steady, _) | (_, Level::Calm) => 0.0,
                (Kind::Temp, Level::HeadsUp) => t.attack,
                (Kind::Temp, Level::Act) => t.sustain,
            };
            let held =
                |since: Option<f64>, need: f32| since.is_some_and(|at| now - at >= f64::from(need));
            // Rise to the highest level that has held through its attack.
            let ready = [Level::Act, Level::HeadsUp]
                .into_iter()
                .find(|l| held(tr.above[slot(*l).unwrap()], attack(*l)))
                .unwrap_or(Level::Calm);
            if ready > tr.shown {
                tr.shown = ready;
            }
            // Fall one level at a time once each has stayed clear.
            while let Some(i) = slot(tr.shown) {
                if tr.shown <= ready || !held(tr.below[i], t.release) {
                    break;
                }
                tr.shown = if i == 1 { Level::HeadsUp } else { Level::Calm };
                tr.shown = tr.shown.max(ready);
            }
            if tr.shown == Level::Calm {
                return level != Level::Calm;
            }
            if let Some(r) = &tr.reason {
                out.push(Reason {
                    level: tr.shown,
                    ..r.clone()
                });
            }
            true
        });
        out.sort_by(|a, b| a.key.cmp(&b.key));
        lamp_of(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Disk, Filesystem, Psi};

    fn fs(mount: &str, used: u64, avail: u64) -> Filesystem {
        Filesystem {
            source: "/dev/x".into(),
            fstype: "ext4".into(),
            mounts: vec![mount.into()],
            total: used + avail,
            used,
            avail,
            ..Default::default()
        }
    }

    #[test]
    fn calm_by_default() {
        let lamp = evaluate(&Snapshot::default(), &LampConfig::default());
        assert_eq!(lamp.level, Level::Calm);
        assert!(lamp.reasons.is_empty());
    }

    #[test]
    fn full_disk_and_hot_cpu() {
        let mut s = Snapshot::default();
        s.disks.push(Disk {
            name: "nvme0n1".into(),
            filesystems: vec![fs("/", 90, 10), fs("/data", 96, 4)],
            ..Default::default()
        });
        s.machine.cpu.temp = Some(87.0);
        let lamp = evaluate(&s, &LampConfig::default());
        assert_eq!(lamp.level, Level::Act);
        assert_eq!(lamp.reasons[0].text, "/data is 96% full");
        assert_eq!(lamp.reasons.len(), 3);
        assert!(lamp.reasons.iter().all(|r| r.level != Level::Calm));
    }

    #[test]
    fn stale_shares_and_automounts_do_not_count() {
        let mut s = Snapshot::default();
        let mut share = fs("/home/nate/Rime", 99, 1);
        share.stale = true;
        s.disks.push(Disk {
            name: "//nas/private".into(),
            kind: DiskKind::Network,
            filesystems: vec![share],
            ..Default::default()
        });
        s.disks.push(Disk {
            name: "auto".into(),
            kind: DiskKind::OnDemand,
            filesystems: vec![fs("/mnt", 99, 1)],
            ..Default::default()
        });
        assert_eq!(evaluate(&s, &LampConfig::default()).level, Level::Calm);
    }

    #[test]
    fn temperatures_use_each_part_limit() {
        let mut s = Snapshot::default();
        // 70°C is past the old fixed 65°C, but a 990 PRO warns at 82°C.
        s.disks.push(Disk {
            name: "nvme1n1".into(),
            model: Some("Samsung SSD 990 PRO 2TB".into()),
            temp: Some(70.0),
            temp_limit: Some(81.85),
            ..Default::default()
        });
        s.machine.cpu.temp = Some(87.0);
        s.machine.cpu.temp_limit = Some(95.0);
        let lamp = evaluate(&s, &LampConfig::default());
        assert_eq!(lamp.level, Level::HeadsUp);
        assert_eq!(lamp.reasons.len(), 1);
        assert_eq!(lamp.reasons[0].text, "CPU is at 87°C (limit 95°C)");
        // A 7800X3D throttles at 89°C.
        s.machine.cpu.temp = Some(89.0);
        s.machine.cpu.temp_limit = Some(89.0);
        assert_eq!(evaluate(&s, &LampConfig::default()).level, Level::Act);
    }

    #[test]
    fn memory_pressure() {
        let mut s = Snapshot::default();
        s.machine.pressure.memory = Some(Psi {
            some60: 14.0,
            ..Default::default()
        });
        assert_eq!(evaluate(&s, &LampConfig::default()).level, Level::HeadsUp);
        s.machine.pressure.memory = Some(Psi {
            full10: 22.0,
            ..Default::default()
        });
        assert_eq!(evaluate(&s, &LampConfig::default()).level, Level::Act);
    }

    fn at(now: f64, cpu: f32) -> Snapshot {
        let mut s = Snapshot {
            now,
            ..Default::default()
        };
        s.machine.cpu.temp = Some(cpu);
        s.machine.cpu.temp_limit = Some(95.0);
        s
    }

    #[test]
    fn a_boosting_cpu_is_not_an_alarm() {
        // A Zen 5 CPU under a build sits at its 95°C limit by design.
        let t = LampConfig::default();
        let mut l = LampState::new();
        assert_eq!(l.update(&at(0.0, 95.0), &t).level, Level::Calm);
        assert_eq!(l.update(&at(9.0, 95.0), &t).level, Level::Calm);
        let lamp = l.update(&at(10.0, 95.0), &t);
        assert_eq!(lamp.level, Level::HeadsUp);
        assert_eq!(lamp.reasons[0].text, "CPU is at 95°C (limit 95°C)");
        assert_eq!(l.update(&at(29.0, 95.0), &t).level, Level::HeadsUp);
        // Thirty seconds at the limit is throttling.
        assert_eq!(l.update(&at(30.0, 95.0), &t).level, Level::Act);
    }

    #[test]
    fn spikes_do_not_change_the_light() {
        let t = LampConfig::default();
        let mut l = LampState::new();
        l.update(&at(0.0, 60.0), &t);
        assert_eq!(l.update(&at(5.0, 92.0), &t).level, Level::Calm);
        assert_eq!(l.update(&at(10.0, 60.0), &t).level, Level::Calm);
        assert_eq!(l.update(&at(20.0, 60.0), &t).level, Level::Calm);
    }

    #[test]
    fn release_waits_and_hysteresis_holds() {
        let t = LampConfig::default();
        let mut l = LampState::new();
        l.update(&at(0.0, 88.0), &t);
        assert_eq!(l.update(&at(10.0, 88.0), &t).level, Level::HeadsUp);
        // 83°C is under the 85°C line but inside the 3°C margin: still hot.
        assert_eq!(l.update(&at(20.0, 83.0), &t).level, Level::HeadsUp);
        assert_eq!(l.update(&at(100.0, 83.0), &t).level, Level::HeadsUp);
        // Clear of the margin, it releases only after 30 s.
        assert_eq!(l.update(&at(110.0, 70.0), &t).level, Level::HeadsUp);
        assert_eq!(l.update(&at(139.0, 70.0), &t).level, Level::HeadsUp);
        assert_eq!(l.update(&at(140.0, 70.0), &t).level, Level::Calm);
        assert!(l.tracks.is_empty());
    }

    #[test]
    fn a_disk_at_the_line_does_not_flap() {
        let t = LampConfig::default();
        let mut l = LampState::new();
        let disk = |now: f64, used: u64| {
            let mut s = Snapshot {
                now,
                ..Default::default()
            };
            s.disks.push(Disk {
                name: "nvme0n1".into(),
                filesystems: vec![fs("/", used, 1000 - used)],
                ..Default::default()
            });
            s
        };
        assert_eq!(l.update(&disk(0.0, 851), &t).level, Level::HeadsUp);
        for (i, used) in [849, 851, 848, 850].into_iter().enumerate() {
            let lamp = l.update(&disk(5.0 * (i + 1) as f64, used), &t);
            assert_eq!(lamp.level, Level::HeadsUp, "{used}");
        }
        l.update(&disk(30.0, 800), &t);
        assert_eq!(l.update(&disk(60.0, 800), &t).level, Level::Calm);
    }

    #[test]
    fn a_throttle_report_acts_at_once() {
        let t = LampConfig::default();
        let mut l = LampState::new();
        let mut s = at(0.0, 70.0);
        s.machine.cpu.throttling = true;
        let lamp = l.update(&s, &t);
        assert_eq!(lamp.level, Level::Act);
        assert_eq!(lamp.reasons[0].key, "cpu.throttle");
    }

    #[test]
    fn classify_holds_inside_the_margin() {
        assert_eq!(classify(84.0, 85.0, 95.0, Level::Calm, 3.0), Level::Calm);
        assert_eq!(
            classify(84.0, 85.0, 95.0, Level::HeadsUp, 3.0),
            Level::HeadsUp
        );
        assert_eq!(classify(93.0, 85.0, 95.0, Level::Act, 3.0), Level::Act);
        assert_eq!(classify(91.0, 85.0, 95.0, Level::Act, 3.0), Level::HeadsUp);
        assert_eq!(classify(81.0, 85.0, 95.0, Level::Act, 3.0), Level::Calm);
    }

    #[test]
    fn holds_for_sessions_and_disks_but_not_heat() {
        let mut s = Snapshot::default();
        s.machine.cpu.throttling = true;
        let lamp = evaluate(&s, &LampConfig::default());
        assert_eq!(lamp.level, Level::Act);
        assert_eq!(hold(&lamp, None), None);
        let session = crate::model::Session::default();
        assert_eq!(hold(&lamp, Some(&session)).as_deref(), Some(SESSION_HOLD));
        s.disks.push(Disk {
            name: "nvme0n1".into(),
            filesystems: vec![fs("/data", 96, 4)],
            ..Default::default()
        });
        let lamp = evaluate(&s, &LampConfig::default());
        assert_eq!(hold(&lamp, None).as_deref(), Some("/data is 96% full"));
    }

    #[test]
    fn hold_is_always_sent() {
        let json = serde_json::to_string(&Lamp::default()).unwrap();
        assert_eq!(json, r#"{"level":"calm","reasons":[],"hold":null}"#);
    }

    #[test]
    fn dropouts_count_only_while_recording() {
        let mut s = Snapshot::default();
        s.audio.graph = Some(crate::model::AudioGraph {
            xruns_minute: 2,
            ..Default::default()
        });
        assert_eq!(evaluate(&s, &LampConfig::default()).level, Level::Calm);
        s.session = Some(crate::model::Session::default());
        let lamp = evaluate(&s, &LampConfig::default());
        assert_eq!(lamp.level, Level::HeadsUp);
        assert_eq!(lamp.reasons[0].text, "2 audio dropouts in the last minute");
    }

    #[test]
    fn loud_fans_count_only_while_recording() {
        let mut s = Snapshot::default();
        s.machine.fans.push(crate::model::Fan {
            name: "nct6798 fan2".into(),
            rpm: 2100,
        });
        assert_eq!(evaluate(&s, &LampConfig::default()).level, Level::Calm);
        s.session = Some(crate::model::Session::default());
        let lamp = evaluate(&s, &LampConfig::default());
        assert_eq!(lamp.reasons[0].text, "fans are loud: nct6798 fan2 2100 RPM");
        assert_eq!(hold(&lamp, None), None);
    }
}
