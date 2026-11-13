//! The daemon socket: newline-delimited JSON, one request per line
//! (`{"cmd":"toggle","anchor":{...}}`), and events to subscribers
//! (`{"event":"show",...}`).

use crate::model::Snapshot;
use crate::Anchor;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SubKind {
    /// Snapshots and show/hide events, for the drawer.
    Board,
    /// Waybar JSON lines, already formatted.
    Bar,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Ping,
    Snapshot,
    Subscribe {
        kind: SubKind,
    },
    Toggle {
        #[serde(default)]
        anchor: Option<Anchor>,
        #[serde(default)]
        tab: Option<String>,
    },
    Show {
        #[serde(default)]
        anchor: Option<Anchor>,
        #[serde(default)]
        tab: Option<String>,
    },
    Hide,
    /// The drawer reports whether it is on screen, which sets how often the
    /// daemon samples.
    BoardVisible {
        visible: bool,
    },
    /// Starts (`true`) or ends (`false`) a recording session; without `on`
    /// it toggles. The reply carries the snapshot taken after the change.
    Session {
        #[serde(default)]
        on: Option<bool>,
    },
    /// Stops the daemon and the drawer, for `rh restart` and for replacing a
    /// daemon whose binary was upgraded.
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Box<Snapshot>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// In a `ping` reply: the daemon's binary was replaced by an upgrade, so
    /// it is running old code and should be restarted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

impl Reply {
    pub fn ok() -> Reply {
        Reply {
            ok: true,
            ..Default::default()
        }
    }

    pub fn err(e: impl ToString) -> Reply {
        Reply {
            ok: false,
            error: Some(e.to_string()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Snapshot {
        snapshot: Box<Snapshot>,
    },
    Show {
        #[serde(default)]
        anchor: Option<Anchor>,
        #[serde(default)]
        tab: Option<String>,
    },
    Toggle {
        #[serde(default)]
        anchor: Option<Anchor>,
        #[serde(default)]
        tab: Option<String>,
    },
    Hide,
    /// The daemon is going away for good (a restart or an upgrade): the
    /// drawer exits, and the daemon starts the new one when it is needed.
    Quit,
}

/// The line a waybar `custom` module with `"return-type": "json"` reads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BarLine {
    pub text: String,
    pub class: String,
    pub alt: String,
    pub tooltip: String,
}

#[cfg(unix)]
mod client {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// Sends one request and reads one reply.
    pub fn request(req: &Request) -> Result<Reply> {
        request_within(req, std::time::Duration::from_secs(5))
    }

    /// Like [`request`], giving up after `timeout` instead of five seconds.
    pub fn request_within(req: &Request, timeout: std::time::Duration) -> Result<Reply> {
        let mut s = crate::transport::connect()?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        s.write_all(line.as_bytes())?;
        let mut reader = BufReader::new(s);
        let mut out = String::new();
        reader.read_line(&mut out)?;
        let reply: Reply =
            serde_json::from_str(&out).map_err(|e| anyhow!("bad reply from daemon: {e}"))?;
        Ok(reply)
    }

    /// Subscribes and yields raw lines until the daemon goes away.
    pub fn subscribe_raw(kind: SubKind) -> Result<impl Iterator<Item = std::io::Result<String>>> {
        let mut s = crate::transport::connect()?;
        let mut line = serde_json::to_string(&Request::Subscribe { kind })?;
        line.push('\n');
        s.write_all(line.as_bytes())?;
        Ok(BufReader::new(s).lines())
    }

    /// Subscribes and yields parsed events.
    pub fn subscribe(kind: SubKind) -> Result<impl Iterator<Item = Result<Event>>> {
        Ok(subscribe_raw(kind)?.map(|l| {
            let l = l?;
            serde_json::from_str::<Event>(&l).map_err(|e| anyhow!("bad event: {e}"))
        }))
    }
}

#[cfg(unix)]
pub use client::*;

#[cfg(not(unix))]
pub fn request(_req: &Request) -> Result<Reply> {
    Err(anyhow!(
        "the Roundhouse daemon does not run on this platform yet"
    ))
}

#[cfg(not(unix))]
pub fn request_within(req: &Request, _timeout: std::time::Duration) -> Result<Reply> {
    request(req)
}

#[cfg(not(unix))]
pub fn subscribe_raw(_kind: SubKind) -> Result<std::iter::Empty<std::io::Result<String>>> {
    Err(anyhow!(
        "the Roundhouse daemon does not run on this platform yet"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shapes() {
        let r: Request = serde_json::from_str(r#"{"cmd":"toggle","tab":"storage"}"#).unwrap();
        assert_eq!(
            r,
            Request::Toggle {
                anchor: None,
                tab: Some("storage".into())
            }
        );
        let s = serde_json::to_string(&Request::Subscribe { kind: SubKind::Bar }).unwrap();
        assert_eq!(s, r#"{"cmd":"subscribe","kind":"bar"}"#);
        let s = serde_json::to_string(&Request::BoardVisible { visible: true }).unwrap();
        assert_eq!(s, r#"{"cmd":"board_visible","visible":true}"#);
        let s = serde_json::to_string(&Request::Quit).unwrap();
        assert_eq!(s, r#"{"cmd":"quit"}"#);
    }

    #[test]
    fn event_shapes() {
        let e = serde_json::to_string(&Event::Hide).unwrap();
        assert_eq!(e, r#"{"event":"hide"}"#);
        let e = serde_json::to_string(&Event::Quit).unwrap();
        assert_eq!(e, r#"{"event":"quit"}"#);
    }

    #[test]
    fn stale_shows_only_when_set() {
        let r = serde_json::to_string(&Reply::ok()).unwrap();
        assert_eq!(r, r#"{"ok":true}"#);
        let stale = Reply {
            stale: true,
            ..Reply::ok()
        };
        let r = serde_json::to_string(&stale).unwrap();
        assert_eq!(r, r#"{"ok":true,"stale":true}"#);
        // A reply from an older daemon has no `stale`.
        let r: Reply = serde_json::from_str(r#"{"ok":true,"version":"0.3.0"}"#).unwrap();
        assert!(!r.stale);
    }
}
