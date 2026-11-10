//! Siblings: Switchyard and Roundhouse know about each other when both run,
//! and neither needs the other. A sibling that is missing, stopped or too old
//! to answer simply unlocks nothing.
//!
//! Roundhouse reads Switchyard's snapshot from its socket, picking only the
//! fields it shows, so a newer Switchyard with more fields still works. It
//! never links Switchyard's crates.

use crate::model::SwitchyardView;
use crate::Anchor;
use serde_json::Value;
use std::time::Duration;

/// How long a sibling gets to answer before it counts as absent.
const TIMEOUT: Duration = Duration::from_millis(300);

/// Pulls the parts of a Switchyard snapshot reply the drawer uses.
pub fn parse_switchyard(reply: &str) -> Option<SwitchyardView> {
    let v: Value = serde_json::from_str(reply).ok()?;
    if v.get("ok").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let snap = v.get("snapshot")?;
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let tasks = snap
        .get("tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let labels = tasks
        .iter()
        .filter_map(|t| {
            let id = t.get("id")?.as_str()?.to_string();
            let label = t
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            Some((id, label))
        })
        .collect();
    let agg = snap.get("aggregate");
    let ml = snap.get("mainline");
    Some(SwitchyardView {
        phase: agg
            .and_then(|a| a.get("phase"))
            .and_then(Value::as_str)
            .unwrap_or("idle")
            .to_string(),
        tasks: tasks.len(),
        current: ml
            .and_then(|m| m.get("current"))
            .and_then(Value::as_str)
            .map(String::from),
        queue: strings(ml.and_then(|m| m.get("queue"))),
        waiting_memory: ml
            .and_then(|m| m.get("waiting_memory"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        building: strings(agg.and_then(|a| a.get("building"))),
        running: strings(agg.and_then(|a| a.get("running"))),
        failed: strings(agg.and_then(|a| a.get("failed"))),
        labels,
    })
}

#[cfg(unix)]
fn ask(line: &str) -> Option<String> {
    use std::io::{BufRead, BufReader, Write};
    let path = crate::paths::switchyard_socket();
    if !path.exists() {
        return None;
    }
    let mut s = crate::transport::connect_to(&path, Some(TIMEOUT)).ok()?;
    s.write_all(line.as_bytes()).ok()?;
    s.write_all(b"\n").ok()?;
    let mut out = String::new();
    BufReader::new(s).read_line(&mut out).ok()?;
    Some(out)
}

#[cfg(not(unix))]
fn ask(_line: &str) -> Option<String> {
    let _ = TIMEOUT;
    None
}

/// Switchyard's state, or `None` when it is not running.
pub fn switchyard() -> Option<SwitchyardView> {
    parse_switchyard(&ask(r#"{"cmd":"snapshot"}"#)?)
}

/// Asks Switchyard to open its board from the same icon. Returns false when
/// Switchyard is not there to ask.
pub fn switchyard_show(anchor: Option<&Anchor>) -> bool {
    let req = serde_json::json!({ "cmd": "show", "anchor": anchor });
    ask(&req.to_string()).is_some_and(|r| r.contains("\"ok\":true"))
}

/// Asks Switchyard to close its board, so only one panel is open at a time.
/// Does nothing when Switchyard is not running.
pub fn switchyard_hide() {
    let _ = ask(r#"{"cmd":"hide"}"#);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_switchyard_snapshot() {
        let reply = r#"{"ok":true,"snapshot":{
          "tasks":[{"id":"launchpad","label":"LaunchPad","phase":"building","future_field":1},{"id":"plos-deploy","phase":"queued"}],
          "mainline":{"current":"launchpad","queue":["plos-deploy"],"waiting_memory":true,"mem_available":1},
          "aggregate":{"phase":"building","failed":[],"building":["launchpad"],"queued":["plos-deploy"],"running":[],"ok":[]},
          "now":1.0}}"#;
        let v = parse_switchyard(reply).unwrap();
        assert_eq!(v.phase, "building");
        assert_eq!(v.tasks, 2);
        assert_eq!(v.current.as_deref(), Some("launchpad"));
        assert_eq!(v.queue, vec!["plos-deploy".to_string()]);
        assert!(v.waiting_memory);
        assert_eq!(v.label("launchpad"), "LaunchPad");
        assert_eq!(v.label("plos-deploy"), "plos-deploy");
    }

    #[test]
    fn tolerates_missing_pieces() {
        let v = parse_switchyard(r#"{"ok":true,"snapshot":{}}"#).unwrap();
        assert_eq!(v.phase, "idle");
        assert!(parse_switchyard(r#"{"ok":false,"error":"nope"}"#).is_none());
        assert!(parse_switchyard("garbage").is_none());
    }
}
