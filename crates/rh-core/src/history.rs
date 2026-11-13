//! The lamp's history on disk: one JSON line each time what the lamp says
//! changes, in `$ROUNDHOUSE_STATE_DIR/history.jsonl`. It survives restarts,
//! so "what happened while I was away" has an answer, and it is what the
//! Timeline tab will read.
//!
//! The file is capped: past 1 MiB it moves to `history.1.jsonl` (replacing
//! the one before) and a new file starts. A line is a few hundred bytes, so
//! that is thousands of changes.

use crate::model::{Level, Snapshot};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const CAP: u64 = 1 << 20;

/// One change of the lamp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unix seconds.
    pub t: f64,
    pub level: Level,
    pub reasons: Vec<EntryReason>,
    #[serde(default)]
    pub session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntryReason {
    pub key: String,
    pub level: Level,
    pub text: String,
}

pub fn file() -> PathBuf {
    crate::paths::state_dir().join("history.jsonl")
}

fn rotated(path: &Path) -> PathBuf {
    path.with_file_name("history.1.jsonl")
}

/// Writes a line when the lamp's state changes: its level, which rules are
/// lit at which level, or the session. A reason's numbers changing
/// ("87°C" to "88°C") is not a change.
#[derive(Default)]
pub struct Recorder {
    last: Option<String>,
    path: Option<PathBuf>,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder::default()
    }

    /// Records into `path` instead of the state directory (tests).
    pub fn at(path: PathBuf) -> Recorder {
        Recorder {
            last: None,
            path: Some(path),
        }
    }

    pub fn record(&mut self, s: &Snapshot) {
        let sig = format!(
            "{}|{}|{}",
            s.lamp.level.as_str(),
            s.lamp
                .reasons
                .iter()
                .map(|r| format!("{}={}", r.key, r.level.as_str()))
                .collect::<Vec<_>>()
                .join(","),
            s.session.is_some()
        );
        if self.last.as_deref() == Some(sig.as_str()) {
            return;
        }
        // The first sample after a start is recorded only if it is not calm:
        // a restart is not an event.
        let first = self.last.is_none();
        self.last = Some(sig);
        if first && s.lamp.level == Level::Calm && s.session.is_none() {
            return;
        }
        let e = Entry {
            t: s.now,
            level: s.lamp.level,
            reasons: s
                .lamp
                .reasons
                .iter()
                .map(|r| EntryReason {
                    key: r.key.clone(),
                    level: r.level,
                    text: r.text.clone(),
                })
                .collect(),
            session: s.session.is_some(),
        };
        let path = self.path.clone().unwrap_or_else(file);
        let _ = append(&path, &e);
    }
}

fn append(path: &Path, e: &Entry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > CAP) {
        std::fs::rename(path, rotated(path))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut line = serde_json::to_string(e).map_err(std::io::Error::other)?;
    line.push('\n');
    f.write_all(line.as_bytes())
}

/// Every entry at or after `since` (Unix seconds), oldest first, from the
/// rotated file and the current one. Lines that do not parse are skipped.
pub fn read(path: &Path, since: f64) -> Vec<Entry> {
    [rotated(path), path.to_path_buf()]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
                .collect::<Vec<_>>()
        })
        .filter(|e| e.t >= since)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Reason;

    fn snap(t: f64, level: Level, text: &str) -> Snapshot {
        let mut s = Snapshot {
            now: t,
            ..Default::default()
        };
        s.lamp.level = level;
        if level != Level::Calm {
            s.lamp.reasons.push(Reason {
                level,
                text: text.into(),
                tab: "machine".into(),
                key: "cpu.temp".into(),
            });
        }
        s
    }

    #[test]
    fn records_changes_only() {
        let dir = std::env::temp_dir().join(format!("rh-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.jsonl");
        let mut r = Recorder::at(path.clone());
        r.record(&snap(1.0, Level::Calm, ""));
        r.record(&snap(2.0, Level::HeadsUp, "CPU is at 87°C (limit 95°C)"));
        r.record(&snap(3.0, Level::HeadsUp, "CPU is at 88°C (limit 95°C)"));
        r.record(&snap(4.0, Level::Calm, ""));
        let all = read(&path, 0.0);
        // The calm start is not an event; the second reading is the same
        // state with another number.
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].reasons[0].text, "CPU is at 87°C (limit 95°C)");
        assert_eq!(all[1].level, Level::Calm);
        assert_eq!(read(&path, 3.5).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_past_the_cap() {
        let dir = std::env::temp_dir().join(format!("rh-history-rot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        std::fs::write(&path, vec![b'\n'; (CAP + 1) as usize]).unwrap();
        let mut r = Recorder::at(path.clone());
        r.record(&snap(
            5.0,
            Level::Act,
            "CPU is slowing itself down for heat",
        ));
        assert!(rotated(&path).exists());
        assert_eq!(read(&path, 0.0).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
