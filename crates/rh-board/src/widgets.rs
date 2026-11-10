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

/// A line chart with an area fill and an end dot, drawn in the widget's CSS
/// `color`, so the theme picks it.
#[derive(Clone)]
pub struct Spark {
    pub area: gtk::DrawingArea,
    data: Rc<RefCell<(Vec<f32>, f32)>>,
}

impl Spark {
    pub fn new(height: i32) -> Spark {
        let area = gtk::DrawingArea::new();
        area.set_content_height(height);
        area.set_hexpand(true);
        area.add_css_class("spark");
        let data: Rc<RefCell<(Vec<f32>, f32)>> = Rc::new(RefCell::new((vec![], 100.0)));
        let d = data.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let (values, max) = &*d.borrow();
            if values.len() < 2 {
                return;
            }
            let c = a.color();
            let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
            let (w, h) = (w as f64, h as f64 - 2.0);
            let n = values.len();
            let pt = |i: usize| {
                let x = i as f64 / (n - 1) as f64 * w;
                let y = 1.0 + h - (values[i] / max.max(1.0)).clamp(0.0, 1.0) as f64 * (h - 1.0);
                (x, y)
            };
            cr.move_to(0.0, h + 2.0);
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

    pub fn set(&self, values: &[f32], max: f32) {
        *self.data.borrow_mut() = (values.to_vec(), max);
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
    /// Severity class for the right-hand text: "", "heads-up" or "act".
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
        for c in ["heads-up", "act"] {
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
