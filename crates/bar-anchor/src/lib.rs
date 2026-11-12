//! Opening a panel from the bar icon that was clicked.
//!
//! Waybar does not publish where its modules are, so the panel asks the
//! compositor instead: where the cursor is, which monitor it is on, and which
//! edge the bar surface sits on. [`anchor`] does that for whichever
//! compositor runs: [`hypr`] knows the cursor, so the panel opens at the icon;
//! [`sway`] and [`niri`] know only the monitor, so the panel opens on that
//! monitor's bar where a [`BarHint`] from the bar's config says the icon is.
//! [`place`] turns the answer into a panel rectangle and a scale origin, so
//! the open animation grows out of the icon.
//!
//! This crate started as Switchyard's anchoring code. The [`Anchor`] wire
//! format is the same as Switchyard's, so one sibling can hand an anchor to the
//! other and its panel opens from the same icon.

pub mod hypr;
pub mod niri;
mod place;
pub mod sway;

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

/// What the caller knows about the bar from the bar's own config. Only sway
/// and niri need it: Hyprland reports the bar's surface and the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BarHint {
    /// The edge the bar is on.
    pub edge: Option<Edge>,
    /// The bar's thickness in logical pixels.
    pub size: Option<f64>,
    /// Where along the bar the icon sits: 0 at the left (or top) end, 0.5
    /// in the middle, 1 at the right (or bottom) end.
    pub along: Option<f64>,
}

/// The compositor whose IPC answers here: `Hyprland`, `niri` or `sway`.
pub fn compositor() -> Option<&'static str> {
    if hypr::socket().is_some() {
        Some("Hyprland")
    } else if niri::socket().is_some() {
        Some("niri")
    } else if sway::socket().is_some() {
        Some("sway")
    } else {
        None
    }
}

/// Where the panel should open, from whichever compositor runs. `None` under
/// a compositor this crate cannot ask; the panel then opens centered.
pub fn anchor(namespace: &str, hint: BarHint, center: bool) -> Option<Anchor> {
    match compositor()? {
        "Hyprland" => hypr::anchor(namespace, center),
        "niri" => niri::anchor(hint, center),
        _ => sway::anchor(hint, center),
    }
}

/// A point on the bar, in monitor-local pixels, standing in for the click:
/// `along` of the way down the bar's usable length (from the left or the
/// top), the middle when unknown, and across the bar's middle.
pub(crate) fn along_bar(
    edge: Edge,
    width: f64,
    height: f64,
    reserved: &[f64; 4],
    along: Option<f64>,
) -> (f64, f64) {
    let f = along.unwrap_or(0.5).clamp(0.0, 1.0);
    match edge {
        Edge::Top | Edge::Bottom => {
            let x = reserved[0] + (width - reserved[0] - reserved[2]) * f;
            let y = if edge == Edge::Top {
                reserved[1] / 2.0
            } else {
                height - reserved[3] / 2.0
            };
            (x, y)
        }
        Edge::Left | Edge::Right => {
            let y = reserved[1] + (height - reserved[1] - reserved[3]) * f;
            let x = if edge == Edge::Left {
                reserved[0] / 2.0
            } else {
                width - reserved[2] / 2.0
            };
            (x, y)
        }
    }
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

    #[test]
    fn points_along_each_bar() {
        let r = [0.0, 0.0, 0.0, 30.0];
        assert_eq!(
            along_bar(Edge::Bottom, 1000.0, 800.0, &r, None),
            (500.0, 785.0)
        );
        assert_eq!(
            along_bar(Edge::Bottom, 1000.0, 800.0, &r, Some(0.0)),
            (0.0, 785.0)
        );
        let r = [0.0, 0.0, 40.0, 0.0];
        assert_eq!(
            along_bar(Edge::Right, 1000.0, 800.0, &r, Some(2.0)),
            (980.0, 800.0)
        );
        let r = [40.0, 24.0, 0.0, 0.0];
        assert_eq!(
            along_bar(Edge::Top, 1000.0, 800.0, &r, Some(1.0)),
            (1000.0, 12.0)
        );
    }
}
