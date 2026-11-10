//! Opening a panel from the bar icon that was clicked.
//!
//! Waybar does not publish where its modules are, so the panel asks the
//! compositor instead: where the cursor is, which monitor it is on, and which
//! edge the bar surface sits on. [`hypr::anchor`] does that for Hyprland, and
//! [`place`] turns the answer into a panel rectangle and a scale origin, so the
//! open animation grows out of the icon.
//!
//! This crate started as Switchyard's anchoring code. The [`Anchor`] wire
//! format is the same as Switchyard's, so one sibling can hand an anchor to the
//! other and its panel opens from the same icon.

pub mod hypr;
mod place;

pub use place::{place, Rect, SIDE_MARGIN};
use serde::{Deserialize, Serialize};

/// The screen edge the bar is on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }
}

/// Where a panel should open, in monitor-local logical pixels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Anchor {
    pub monitor: Option<String>,
    pub edge: Edge,
    /// Click position relative to the monitor's top-left corner.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Space other surfaces reserve: left, top, right, bottom.
    pub reserved: [f64; 4],
    /// Open centered on the monitor instead of at the click.
    pub center: bool,
    /// Coordinates are global physical pixels (tray icons) rather than
    /// monitor-local logical ones (Hyprland).
    #[serde(default)]
    pub physical: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_matches_switchyard() {
        let json = r#"{"monitor":"DP-2","edge":"bottom","x":2310,"y":1428,"width":2560,"height":1440,"reserved":[0,0,0,24],"center":false}"#;
        let a: Anchor = serde_json::from_str(json).unwrap();
        assert_eq!(a.edge, Edge::Bottom);
        assert!(!a.physical);
        let back = serde_json::to_value(&a).unwrap();
        assert_eq!(back["edge"], "bottom");
        assert_eq!(back["reserved"][3], 24.0);
    }
}
