//! Asks niri which monitor the click was on, by talking to its IPC socket
//! (`$NIRI_SOCKET`) directly, without `niri msg`.
//!
//! niri's IPC has no cursor position and gives layer surfaces no geometry.
//! A click focuses the output under the cursor before the bar sees it, so
//! the focused output is the one that was clicked. The bar's edge, size and
//! where along it the icon sits come from the caller's [`BarHint`].

use crate::{Anchor, BarHint, Edge};
use anyhow::{anyhow, Result};
use serde::Deserialize;
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

pub fn socket() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("NIRI_SOCKET")?);
    p.exists().then_some(p)
}

#[cfg(not(unix))]
pub fn query(_request: &str) -> Result<String> {
    Err(anyhow!("niri only runs on Linux"))
}

/// Sends one request with no arguments (`FocusedOutput`) and returns the
/// reply line.
#[cfg(unix)]
pub fn query(request: &str) -> Result<String> {
    let path = socket().ok_or_else(|| anyhow!("not running under niri"))?;
    let mut s = std::os::unix::net::UnixStream::connect(path)?;
    s.set_read_timeout(Some(Duration::from_millis(800)))?;
    s.write_all(format!("{}\n", serde_json::to_string(request)?).as_bytes())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    Ok(line)
}

#[derive(Debug, Clone, Deserialize)]
struct Logical {
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct Output {
    name: String,
    /// `None` while the output is off.
    logical: Option<Logical>,
}

#[derive(Debug, Deserialize)]
enum Reply {
    Ok(FocusedOutput),
    Err(String),
}

#[derive(Debug, Deserialize)]
enum FocusedOutput {
    FocusedOutput(Option<Output>),
}

/// Works out the anchor from a raw `FocusedOutput` reply. Split out so it
/// can be tested against recorded output.
pub fn compute_anchor(focused_json: &str, hint: BarHint, center: bool) -> Result<Anchor> {
    let out = match serde_json::from_str::<Reply>(focused_json)? {
        Reply::Ok(FocusedOutput::FocusedOutput(o)) => o.ok_or_else(|| anyhow!("no output"))?,
        Reply::Err(e) => return Err(anyhow!("niri: {e}")),
    };
    let size = out.logical.ok_or_else(|| anyhow!("{} is off", out.name))?;
    // Waybar's default position is the top.
    let edge = hint.edge.unwrap_or(Edge::Top);
    let mut reserved = [0.0; 4];
    let side = match edge {
        Edge::Left => 0,
        Edge::Top => 1,
        Edge::Right => 2,
        Edge::Bottom => 3,
    };
    reserved[side] = hint.size.unwrap_or(0.0);
    let (x, y) = crate::along_bar(edge, size.width, size.height, &reserved, hint.along);
    Ok(Anchor {
        monitor: Some(out.name),
        edge,
        x,
        y,
        width: size.width,
        height: size.height,
        reserved,
        center,
        physical: false,
    })
}

/// Asks niri for the current anchor. `None` outside niri.
pub fn anchor(hint: BarHint, center: bool) -> Option<Anchor> {
    socket()?;
    let focused = query("FocusedOutput").ok()?;
    compute_anchor(&focused, hint, center).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // niri 26.04 nested in Hyprland (one `winit` output, flipped).
    const FOCUSED: &str = include_str!("../fixtures/niri-focused-output.json");

    #[test]
    fn focused_output_with_a_hint() {
        let hint = BarHint {
            edge: Some(Edge::Top),
            size: Some(28.0),
            along: Some(1.0),
        };
        let a = compute_anchor(FOCUSED, hint, false).unwrap();
        assert_eq!(a.monitor.as_deref(), Some("winit"));
        assert_eq!(a.edge, Edge::Top);
        assert_eq!((a.width, a.height), (1262.0, 1386.0));
        assert_eq!(a.reserved, [0.0, 28.0, 0.0, 0.0]);
        assert_eq!((a.x, a.y), (1262.0, 14.0));
    }

    #[test]
    fn without_a_hint_the_bar_is_on_top_and_the_icon_in_the_middle() {
        let a = compute_anchor(FOCUSED, BarHint::default(), true).unwrap();
        assert_eq!(a.edge, Edge::Top);
        assert_eq!(a.x, 631.0);
        assert!(a.center);
    }

    #[test]
    fn errors_and_no_output() {
        assert!(compute_anchor(
            r#"{"Err":"error parsing request"}"#,
            BarHint::default(),
            false
        )
        .is_err());
        assert!(compute_anchor(
            r#"{"Ok":{"FocusedOutput":null}}"#,
            BarHint::default(),
            false
        )
        .is_err());
        let off = r#"{"Ok":{"FocusedOutput":{"name":"DP-1","logical":null}}}"#;
        assert!(compute_anchor(off, BarHint::default(), false).is_err());
    }
}
