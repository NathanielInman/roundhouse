//! The bar lamp: one color for the whole machine. Calm unless something
//! needs you, with every reason listed so the drawer can say why.

use crate::config::LampConfig;
use crate::model::{DiskKind, Lamp, Level, Reason, Snapshot};

fn push(out: &mut Vec<Reason>, level: Level, tab: &str, text: String) {
    out.push(Reason {
        level,
        text,
        tab: tab.into(),
    });
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
            if temp >= t.drive_temp_act {
                push(
                    &mut r,
                    Level::Act,
                    "storage",
                    format!("{name} is at {temp:.0}°C"),
                );
            } else if temp >= t.drive_temp_warn {
                push(
                    &mut r,
                    Level::HeadsUp,
                    "storage",
                    format!("{name} is at {temp:.0}°C"),
                );
            }
        }
    }

    if let Some(temp) = m.cpu.temp {
        if temp >= t.cpu_temp_act {
            push(
                &mut r,
                Level::Act,
                "machine",
                format!("CPU is at {temp:.0}°C"),
            );
        } else if temp >= t.cpu_temp_warn {
            push(
                &mut r,
                Level::HeadsUp,
                "machine",
                format!("CPU is at {temp:.0}°C"),
            );
        }
    }
    for g in &m.gpus {
        if let Some(temp) = g.temp {
            if temp >= t.gpu_temp_act {
                push(
                    &mut r,
                    Level::Act,
                    "machine",
                    format!("{} is at {temp:.0}°C", g.name),
                );
            } else if temp >= t.gpu_temp_warn {
                push(
                    &mut r,
                    Level::HeadsUp,
                    "machine",
                    format!("{} is at {temp:.0}°C", g.name),
                );
            }
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
