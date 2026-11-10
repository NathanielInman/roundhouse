//! The bar lamp: one color for the whole machine. Calm unless something
//! needs you, with every reason listed so the drawer can say why.

use crate::config::LampConfig;
use crate::model::{DiskKind, Lamp, Level, Reason, Snapshot, TempLimit};

fn push(out: &mut Vec<Reason>, level: Level, tab: &str, text: String) {
    out.push(Reason {
        level,
        text,
        tab: tab.into(),
    });
}

/// A temperature at or past its heads-up line, said against the limit so the
/// number means something.
fn hot(out: &mut Vec<Reason>, tab: &str, name: &str, temp: f32, limit: TempLimit) {
    let level = limit.level(temp);
    if level != Level::Calm {
        push(
            out,
            level,
            tab,
            format!("{name} is at {temp:.0}°C (limit {:.0}°C)", limit.act),
        );
    }
}

/// Works out the lamp from a snapshot. Pure, so the rules are easy to test.
pub fn evaluate(s: &Snapshot, t: &LampConfig) -> Lamp {
    let mut r = vec![];
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
            if pct >= t.disk_act {
                push(
                    &mut r,
                    Level::Act,
                    "storage",
                    format!("{mount} is {pct:.0}% full"),
                );
            } else if pct >= t.disk_warn {
                push(
                    &mut r,
                    Level::HeadsUp,
                    "storage",
                    format!("{mount} is {pct:.0}% full"),
                );
            }
        }
        if let Some(temp) = disk.temp {
            let name = disk.model.as_deref().unwrap_or(&disk.name);
            hot(&mut r, "storage", name, temp, t.drive_temp(disk.temp_limit));
        }
    }

    if let Some(temp) = m.cpu.temp {
        hot(&mut r, "machine", "CPU", temp, t.cpu_temp(m.cpu.temp_limit));
    }
    for g in &m.gpus {
        if let Some(temp) = g.temp {
            hot(&mut r, "machine", &g.name, temp, t.gpu_temp(g.temp_limit));
        }
    }

    if m.memory.total > 0 {
        let free = m.memory.available as f32 / m.memory.total as f32 * 100.0;
        if free < t.mem_free_act {
            push(
                &mut r,
                Level::Act,
                "machine",
                format!("only {free:.0}% of memory is free"),
            );
        }
    }
    if let Some(p) = m.pressure.memory {
        if p.full10 >= t.mem_pressure_act {
            push(
                &mut r,
                Level::Act,
                "machine",
                format!(
                    "everything is stalling on memory ({:.0}% of the last 10 s)",
                    p.full10
                ),
            );
        } else if p.some60 >= t.mem_pressure_warn {
            push(
                &mut r,
                Level::HeadsUp,
                "machine",
                format!(
                    "tasks are waiting on memory ({:.0}% of the last minute)",
                    p.some60
                ),
            );
        }
    }

    r.sort_by_key(|x| std::cmp::Reverse(x.level));
    let level = r.first().map(|x| x.level).unwrap_or(Level::Calm);
    Lamp { level, reasons: r }
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
}
