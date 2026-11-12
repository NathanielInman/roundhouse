//! Asks sway which monitor the click was on, by talking to its IPC socket
//! (`$SWAYSOCK`) directly, without `swaymsg`.
//!
//! Sway's IPC has no cursor position and does not list layer surfaces. With
//! sway's default `focus_follows_mouse yes`, moving onto the bar focuses
//! that output, so the focused output is the one that was clicked. The bar's
//! edge and size come from the space it takes from the visible workspace;
//! where along the bar the icon sits comes from the caller's [`BarHint`].

use crate::{Anchor, BarHint, Edge};
use anyhow::{anyhow, Result};
use serde::Deserialize;
#[cfg(unix)]
use std::io::{Read, Write};
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

/// `GET_WORKSPACES` in sway-ipc(7).
pub const GET_WORKSPACES: u32 = 1;
/// `GET_OUTPUTS` in sway-ipc(7).
pub const GET_OUTPUTS: u32 = 3;

const MAGIC: &[u8; 6] = b"i3-ipc";

pub fn socket() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("SWAYSOCK")?);
    p.exists().then_some(p)
}

#[cfg(not(unix))]
pub fn query(_kind: u32) -> Result<String> {
    Err(anyhow!("sway only runs on Linux"))
}

/// Sends one message with no payload and returns the JSON reply.
#[cfg(unix)]
pub fn query(kind: u32) -> Result<String> {
    let path = socket().ok_or_else(|| anyhow!("not running under sway"))?;
    let mut s = std::os::unix::net::UnixStream::connect(path)?;
    s.set_read_timeout(Some(Duration::from_millis(800)))?;
    let mut msg = Vec::with_capacity(14);
    msg.extend_from_slice(MAGIC);
    msg.extend_from_slice(&0u32.to_ne_bytes());
    msg.extend_from_slice(&kind.to_ne_bytes());
    s.write_all(&msg)?;
    let mut head = [0u8; 14];
    s.read_exact(&mut head)?;
    if &head[..6] != MAGIC {
        return Err(anyhow!("sway sent a reply without the i3-ipc header"));
    }
    let len = u32::from_ne_bytes([head[6], head[7], head[8], head[9]]) as usize;
    let mut body = vec![0u8; len];
    s.read_exact(&mut body)?;
    Ok(String::from_utf8(body)?)
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
pub struct SwayRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Output {
    pub name: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub focused: bool,
    /// Logical pixels: scale and rotation are already applied.
    pub rect: SwayRect,
    #[serde(default)]
    pub current_workspace: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub name: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub focused: bool,
    /// The output's usable area (what layer surfaces leave) shrunk by gaps.
    pub rect: SwayRect,
}

/// Works out the anchor from raw `GET_OUTPUTS` and `GET_WORKSPACES`
/// replies. Split out so it can be tested against recorded output.
pub fn compute_anchor(
    outputs_json: &str,
    workspaces_json: &str,
    hint: BarHint,
    center: bool,
) -> Result<Anchor> {
    let outputs: Vec<Output> = serde_json::from_str(outputs_json)?;
    let workspaces: Vec<Workspace> = serde_json::from_str(workspaces_json).unwrap_or_default();
    let active = || outputs.iter().filter(|o| o.active);
    let out = active()
        .find(|o| o.focused)
        .or_else(|| {
            let ws = workspaces.iter().find(|w| w.focused)?;
            active().find(|o| o.name == ws.output)
        })
        .or_else(|| active().next())
        .ok_or_else(|| anyhow!("no active outputs"))?;
    let o = out.rect;
    let ws = workspaces
        .iter()
        .find(|w| w.output == out.name && Some(&w.name) == out.current_workspace.as_ref());
    let reserved = match ws {
        Some(w) => {
            let r = w.rect;
            let insets = [
                r.x - o.x,
                r.y - o.y,
                (o.x + o.width) - (r.x + r.width),
                (o.y + o.height) - (r.y + r.height),
            ];
            // Each inset is a bar plus that side's gap. The IPC has no gaps,
            // so the thinnest side (no bar, gap only) stands in for them.
            let gap = insets.iter().cloned().fold(f64::INFINITY, f64::min);
            insets.map(|i| (i - gap).max(0.0))
        }
        None => [0.0; 4],
    };
    let edge = hint.edge.unwrap_or_else(|| widest(&reserved, Edge::Top));
    let (x, y) = crate::along_bar(edge, o.width, o.height, &reserved, hint.along);
    Ok(Anchor {
        monitor: Some(out.name.clone()),
        edge,
        x,
        y,
        width: o.width,
        height: o.height,
        reserved,
        center,
        physical: false,
    })
}

/// The side that reserves the most space, or `fallback` when none does.
fn widest(reserved: &[f64; 4], fallback: Edge) -> Edge {
    let idx = (0..4)
        .max_by(|a, b| reserved[*a].partial_cmp(&reserved[*b]).unwrap())
        .unwrap_or(1);
    if reserved[idx] <= 0.0 {
        fallback
    } else {
        [Edge::Left, Edge::Top, Edge::Right, Edge::Bottom][idx]
    }
}

/// Asks sway for the current anchor. `None` outside sway.
pub fn anchor(hint: BarHint, center: bool) -> Option<Anchor> {
    socket()?;
    let outputs = query(GET_OUTPUTS).ok()?;
    let workspaces = query(GET_WORKSPACES).unwrap_or_default();
    compute_anchor(&outputs, &workspaces, hint, center).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // sway 1.12 nested in Hyprland with two outputs, `gaps outer 6`,
    // `gaps inner 4` and a 30 px waybar at the bottom of each; WL-2 focused.
    const OUTPUTS: &str = include_str!("../fixtures/sway-outputs.json");
    const WORKSPACES: &str = include_str!("../fixtures/sway-workspaces.json");

    #[test]
    fn focused_output_and_bottom_bar() {
        let a = compute_anchor(OUTPUTS, WORKSPACES, BarHint::default(), false).unwrap();
        assert_eq!(a.monitor.as_deref(), Some("WL-2"));
        assert_eq!(a.edge, Edge::Bottom);
        assert_eq!(a.reserved, [0.0, 0.0, 0.0, 30.0]);
        assert_eq!((a.width, a.height), (1262.0, 1386.0));
        assert_eq!((a.x, a.y), (631.0, 1371.0));
        assert!(!a.physical);
    }

    #[test]
    fn hint_places_the_icon_and_wins_on_edge() {
        let hint = BarHint {
            edge: Some(Edge::Bottom),
            along: Some(1.0),
            ..BarHint::default()
        };
        let a = compute_anchor(OUTPUTS, WORKSPACES, hint, false).unwrap();
        assert_eq!(a.x, 1262.0);
        let hint = BarHint {
            edge: Some(Edge::Top),
            ..BarHint::default()
        };
        let a = compute_anchor(OUTPUTS, WORKSPACES, hint, true).unwrap();
        assert_eq!(a.edge, Edge::Top);
        assert!(a.center);
    }

    #[test]
    fn falls_back_to_the_focused_workspace() {
        let outputs = r#"[
          {"name":"DP-1","active":true,"rect":{"x":0,"y":0,"width":1920,"height":1080},"current_workspace":"1"},
          {"name":"DP-2","active":true,"rect":{"x":1920,"y":0,"width":1280,"height":720},"current_workspace":"2"},
          {"name":"HDMI-A-1","active":false,"focused":true,"rect":{"x":0,"y":0,"width":0,"height":0}}
        ]"#;
        let workspaces = r#"[
          {"name":"1","output":"DP-1","focused":false,"rect":{"x":0,"y":0,"width":1920,"height":1080}},
          {"name":"2","output":"DP-2","focused":true,"rect":{"x":1952,"y":0,"width":1248,"height":690}}
        ]"#;
        let a = compute_anchor(outputs, workspaces, BarHint::default(), false).unwrap();
        assert_eq!(a.monitor.as_deref(), Some("DP-2"));
        assert_eq!(a.reserved, [32.0, 0.0, 0.0, 30.0]);
        assert_eq!(a.edge, Edge::Left);
        assert_eq!((a.x, a.y), (16.0, 345.0));
    }

    #[test]
    fn no_bar_reserves_nothing() {
        let outputs = r#"[{"name":"eDP-1","active":true,"focused":true,"rect":{"x":0,"y":0,"width":1440,"height":900},"current_workspace":"1"}]"#;
        let workspaces = r#"[{"name":"1","output":"eDP-1","focused":true,"rect":{"x":10,"y":10,"width":1420,"height":880}}]"#;
        let a = compute_anchor(outputs, workspaces, BarHint::default(), false).unwrap();
        assert_eq!(a.reserved, [0.0; 4]);
        assert_eq!(a.edge, Edge::Top);
        assert!(compute_anchor("[]", "[]", BarHint::default(), false).is_err());
    }
}
