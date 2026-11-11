//! Small building blocks: metric tiles with sparklines, meter rows, and a
//! row list that reuses its widgets between snapshots.

use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// The span a sparkline shows, in seconds: the last minute.
pub const WINDOW: f64 = 60.0;

/// Where each value sits across a sparkline, 0 (oldest) to 1 (now), placed by
/// when it was sampled. `times` and `values` end at the same sample. Values
/// older than the window are dropped, except the one just before it, so the
/// line runs in from the left edge.
pub fn place(values: &[f32], times: &[f64]) -> Vec<(f64, f32)> {
    let n = values.len().min(times.len());
    if n == 0 {
        return vec![];
    }
    let (values, times) = (&values[values.len() - n..], &times[times.len() - n..]);
    let last = times[n - 1];
    let first = times.iter().rposition(|t| last - t >= WINDOW).unwrap_or(0);
    (first..n)
        .map(|i| (1.0 - (last - times[i]) / WINDOW, values[i]))
        .collect()
}

/// Placed points and the value at the top of the chart.
type SparkData = Rc<RefCell<(Vec<(f64, f32)>, f32)>>;

/// A line chart with an area fill and an end dot, drawn in the widget's CSS
/// `color`, so the theme picks it. Points sit by time, not by index.
#[derive(Clone)]
pub struct Spark {
    pub area: gtk::DrawingArea,
    data: SparkData,
}

impl Spark {
    pub fn new(height: i32) -> Spark {
        let area = gtk::DrawingArea::new();
        area.set_content_height(height);
        area.set_hexpand(true);
        area.add_css_class("spark");
        let data: SparkData = Rc::new(RefCell::new((vec![], 100.0)));
        let d = data.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let (points, max) = &*d.borrow();
            if points.len() < 2 {
                return;
            }
            let c = a.color();
            let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
            let (w, h) = (w as f64, h as f64 - 2.0);
            let n = points.len();
            let pt = |i: usize| {
                let (at, v) = points[i];
                let x = at * w;
                let y = 1.0 + h - (v / max.max(1.0)).clamp(0.0, 1.0) as f64 * (h - 1.0);
                (x, y)
            };
            cr.move_to(pt(0).0, h + 2.0);
            for i in 0..n {
                let (x, y) = pt(i);
                cr.line_to(x, y);
            }
            cr.line_to(w, h + 2.0);
            cr.close_path();
            cr.set_source_rgba(r, g, b, 0.16);
            let _ = cr.fill();
            for i in 0..n {
                let (x, y) = pt(i);
                if i == 0 {
                    cr.move_to(x, y);
                } else {
                    cr.line_to(x, y);
                }
            }
            cr.set_source_rgba(r, g, b, 1.0);
            cr.set_line_width(1.4);
            let _ = cr.stroke();
            let (x, y) = pt(n - 1);
            cr.arc(x - 1.5, y, 2.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        });
        Spark { area, data }
    }

    /// `times` are the sample times shared by every series (`Machine::times`).
    pub fn set(&self, values: &[f32], times: &[f64], max: f32) {
        *self.data.borrow_mut() = (place(values, times), max);
        self.area.queue_draw();
    }
}

/// A metric tile: label and aside on top, a big number, and a sparkline.
pub struct Tile {
    pub root: gtk::Box,
    pub title: gtk::Label,
    pub aside: gtk::Label,
    pub value: gtk::Label,
    pub unit: gtk::Label,
    pub spark: Spark,
}

impl Tile {
    pub fn new(title: &str) -> Tile {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
        root.add_css_class("tile");
        root.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = label(title, &["t-label"]);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let aside = label("", &["t-label", "num"]);
        top.append(&title);
        top.append(&aside);
        let mid = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let value = label("-", &["t-val", "num"]);
        let unit = label("", &["t-unit", "num"]);
        unit.set_valign(gtk::Align::Baseline);
        value.set_valign(gtk::Align::Baseline);
        unit.set_ellipsize(gtk::pango::EllipsizeMode::End);
        mid.append(&value);
        mid.append(&unit);
        let spark = Spark::new(26);
        root.append(&top);
        root.append(&mid);
        root.append(&spark.area);
        Tile {
            root,
            title,
            aside,
            value,
            unit,
            spark,
        }
    }
}

/// What one row shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RowData {
    pub name: String,
    pub detail: String,
    pub right: String,
    /// Severity class for the right-hand text: "", "calm", "heads-up" or
    /// "act".
    pub right_class: &'static str,
    /// Meter fill 0..1; `None` hides the meter.
    pub meter: Option<f64>,
    pub meter_class: &'static str,
    /// A group heading (a disk above its filesystems).
    pub head: bool,
}

pub struct Row {
    root: gtk::Box,
    name: gtk::Label,
    detail: gtk::Label,
    right: gtk::Label,
    meter: gtk::ProgressBar,
    last: RefCell<Option<RowData>>,
}

impl Row {
    fn new() -> Row {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 3);
        root.add_css_class("row");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = label("", &[]);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let detail = label("", &["detail"]);
        detail.set_hexpand(true);
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let right = label("", &["right", "num"]);
        right.set_xalign(1.0);
        top.append(&name);
        top.append(&detail);
        top.append(&right);
        let meter = gtk::ProgressBar::new();
        meter.add_css_class("meter");
        root.append(&top);
        root.append(&meter);
        Row {
            root,
            name,
            detail,
            right,
            meter,
            last: RefCell::new(None),
        }
    }

    fn set(&self, d: &RowData) {
        if self.last.borrow().as_ref() == Some(d) {
            return;
        }
        self.name.set_label(&d.name);
        self.detail.set_label(&d.detail);
        self.right.set_label(&d.right);
        for c in ["calm", "heads-up", "act"] {
            self.right.remove_css_class(c);
        }
        if !d.right_class.is_empty() {
            self.right.add_css_class(d.right_class);
        }
        if d.head {
            self.root.add_css_class("head");
        } else {
            self.root.remove_css_class("head");
        }
        match d.meter {
            Some(f) => {
                self.meter.set_visible(true);
                self.meter.set_fraction(f.clamp(0.0, 1.0));
                for c in ["calm", "heads-up", "act"] {
                    self.meter.remove_css_class(c);
                }
                if !d.meter_class.is_empty() {
                    self.meter.add_css_class(d.meter_class);
                }
            }
            None => self.meter.set_visible(false),
        }
        *self.last.borrow_mut() = Some(d.clone());
    }
}

/// A vertical list of rows that grows and shrinks to fit, reusing widgets so
/// a refresh never flickers.
pub struct Rows {
    pub root: gtk::Box,
    rows: RefCell<Vec<Row>>,
}

impl Rows {
    pub fn new() -> Rows {
        Rows {
            root: gtk::Box::new(gtk::Orientation::Vertical, 0),
            rows: RefCell::new(vec![]),
        }
    }

    pub fn set(&self, items: &[RowData]) {
        let mut rows = self.rows.borrow_mut();
        while rows.len() < items.len() {
            let r = Row::new();
            self.root.append(&r.root);
            rows.push(r);
        }
        while rows.len() > items.len() {
            if let Some(r) = rows.pop() {
                self.root.remove(&r.root);
            }
        }
        for (r, d) in rows.iter().zip(items) {
            r.set(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparklines_follow_time_not_index() {
        // Five-second samples while closed, then one-second samples once the
        // drawer opens.
        let times = [0.0, 5.0, 10.0, 60.0, 65.0, 66.0, 67.0];
        let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let p = place(&values, &times);
        // The window is the last minute; 5.0 is just before it and stays so
        // the line enters from the left edge.
        assert_eq!(p.first(), Some(&(1.0 - 62.0 / 60.0, 2.0)));
        assert_eq!(p.last(), Some(&(1.0, 7.0)));
        // 65 s and 66 s are one second apart, not one slot.
        let (a, b) = (p[p.len() - 3].0, p[p.len() - 2].0);
        assert!((b - a - 1.0 / 60.0).abs() < 1e-9);
    }

    #[test]
    fn series_align_at_their_end() {
        // CPU skips the first sample, so it is one shorter than the times.
        let p = place(&[10.0, 20.0], &[0.0, 1.0, 2.0]);
        assert_eq!(p, vec![(1.0 - 1.0 / 60.0, 10.0), (1.0, 20.0)]);
    }
}
