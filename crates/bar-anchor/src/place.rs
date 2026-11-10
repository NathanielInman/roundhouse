//! Placement math: where the panel goes next to the bar, horizontally
//! centered on the click and clamped to the screen, and the point the open
//! animation scales from.

use crate::{Anchor, Edge};

/// Distance the panel keeps from the screen's side edges.
pub const SIDE_MARGIN: f64 = 10.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// Where the panel goes, given the surface size and the anchor. Returns the
/// panel rectangle and the scale origin in panel coordinates.
pub fn place(
    anchor: Option<&Anchor>,
    sw: f64,
    sh: f64,
    pw: f64,
    ph: f64,
    gap: f64,
) -> (Rect, (f64, f64)) {
    let pw = pw.min(sw - 2.0 * SIDE_MARGIN).max(1.0);
    let ph = ph.min(sh - 2.0 * gap).max(1.0);
    let clamp_x = |x: f64| x.clamp(SIDE_MARGIN, (sw - pw - SIDE_MARGIN).max(SIDE_MARGIN));
    let clamp_y = |y: f64| y.clamp(gap, (sh - ph - gap).max(gap));
    let a = match anchor {
        Some(a) if !a.center => a,
        _ => {
            let x = (sw - pw) / 2.0;
            let y = ((sh - ph) / 2.0 * 0.85).max(gap);
            return (Rect { x, y, w: pw, h: ph }, (pw / 2.0, ph / 2.0));
        }
    };
    // The surface starts after the space other surfaces reserve.
    let lx = a.x - a.reserved[0];
    let ly = a.y - a.reserved[1];
    match a.edge {
        Edge::Bottom => {
            let x = clamp_x(lx - pw / 2.0);
            let y = sh - gap - ph;
            (
                Rect { x, y, w: pw, h: ph },
                ((lx - x).clamp(0.0, pw), ph + gap),
            )
        }
        Edge::Top => {
            let x = clamp_x(lx - pw / 2.0);
            let y = gap;
            (Rect { x, y, w: pw, h: ph }, ((lx - x).clamp(0.0, pw), -gap))
        }
        Edge::Left => {
            let y = clamp_y(ly - ph / 2.0);
            let x = gap;
            (Rect { x, y, w: pw, h: ph }, (-gap, (ly - y).clamp(0.0, ph)))
        }
        Edge::Right => {
            let y = clamp_y(ly - ph / 2.0);
            let x = sw - gap - pw;
            (
                Rect { x, y, w: pw, h: ph },
                (pw + gap, (ly - y).clamp(0.0, ph)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(edge: Edge, x: f64, y: f64) -> Anchor {
        Anchor {
            edge,
            x,
            y,
            reserved: [0.0, 0.0, 0.0, 24.0],
            width: 2560.0,
            height: 1440.0,
            ..Default::default()
        }
    }

    #[test]
    fn bottom_right_click_clamps() {
        let (r, o) = place(
            Some(&anchor(Edge::Bottom, 2500.0, 1428.0)),
            2560.0,
            1416.0,
            640.0,
            400.0,
            8.0,
        );
        assert_eq!(r.x, 2560.0 - 640.0 - SIDE_MARGIN);
        assert_eq!(r.y, 1416.0 - 8.0 - 400.0);
        assert_eq!(o.1, 408.0);
        assert!(o.0 > 500.0 && o.0 <= 640.0);
    }

    #[test]
    fn bottom_middle_centers() {
        let (r, o) = place(
            Some(&anchor(Edge::Bottom, 1280.0, 1428.0)),
            2560.0,
            1416.0,
            640.0,
            400.0,
            8.0,
        );
        assert_eq!(r.x, 960.0);
        assert_eq!(o.0, 320.0);
    }

    #[test]
    fn top_and_center() {
        let mut a = anchor(Edge::Top, 100.0, 10.0);
        a.reserved = [0.0, 30.0, 0.0, 0.0];
        let (r, o) = place(Some(&a), 2560.0, 1410.0, 640.0, 400.0, 8.0);
        assert_eq!(r.y, 8.0);
        assert_eq!(r.x, SIDE_MARGIN);
        assert_eq!(o.1, -8.0);
        let (r, _) = place(None, 1000.0, 800.0, 600.0, 400.0, 8.0);
        assert_eq!(r.x, 200.0);
    }

    #[test]
    fn tiny_screen_shrinks() {
        let (r, _) = place(None, 400.0, 300.0, 640.0, 500.0, 8.0);
        assert_eq!(r.w, 380.0);
        assert_eq!(r.h, 284.0);
    }
}
