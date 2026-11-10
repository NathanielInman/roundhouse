//! The drawer: header with the lamp, tabs, and one page per tab.

use crate::anim::{Motion, Spring};
use crate::stage::Stage;
use crate::widgets::{label, RowData, Rows, Tile};
use bar_anchor::Anchor;
use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge as LsEdge, KeyboardMode, Layer, LayerShell};
use rh_core::config::Config;
use rh_core::model::{DiskKind, Level, Snapshot};
use rh_core::protocol::{Event, Request};
use rh_core::{duration, human};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub enum Msg {
    Event(Event),
    /// A watched stylesheet or the config changed.
    Restyle,
}

const TABS: [(&str, &str); 3] = [
    ("machine", "Machine"),
    ("storage", "Storage"),
    ("switchyard", "Switchyard"),
];

struct MachinePage {
    root: gtk::Box,
    verdict: gtk::Box,
    reasons: gtk::Box,
    reasons_sig: RefCell<String>,
    cpu: Tile,
    gpu: Tile,
    mem: Tile,
    psi: Tile,
    tiles: Vec<Spring>,
    apps: Rows,
    apps_card: gtk::Box,
}

struct StoragePage {
    root: gtk::Box,
    rows: Rows,
}

struct YardPage {
    root: gtk::Box,
    rows: Rows,
}

pub struct Drawer {
    /// Itself, for handlers created while rendering.
    me: RefCell<std::rc::Weak<Drawer>>,
    window: gtk::ApplicationWindow,
    stage: Stage,
    spring: Spring,
    visible: Cell<bool>,
    anchor: RefCell<Option<Anchor>>,
    snap: RefCell<Option<Snapshot>>,
    cfg: RefCell<Config>,
    lamp: gtk::Label,
    host: gtk::Label,
    sub: gtk::Label,
    chip: gtk::Label,
    notice: gtk::Label,
    tabs: Vec<(String, gtk::Button)>,
    tab: RefCell<String>,
    stack: gtk::Stack,
    machine: MachinePage,
    storage: StoragePage,
    yard: YardPage,
    foot: gtk::Label,
}

fn send_quiet(req: Request) {
    std::thread::spawn(move || {
        let _ = rh_core::protocol::request(&req);
    });
}

fn find_monitor(name: &str) -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    let list = display.monitors();
    (0..list.n_items())
        .filter_map(|i| list.item(i).and_downcast::<gdk::Monitor>())
        .find(|m| m.connector().is_some_and(|c| c == name))
}

fn card(title: &str, aside: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.add_css_class("card");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let t = label(title, &["t-label"]);
    t.set_hexpand(true);
    top.append(&t);
    top.append(&label(aside, &["t-label"]));
    b.append(&top);
    b
}

fn level_class(pct: f32, warn: f32, act: f32) -> &'static str {
    if pct >= act {
        "act"
    } else if pct >= warn {
        "heads-up"
    } else {
        ""
    }
}

fn tilde(p: &str) -> String {
    let home = rh_core::paths::home();
    let h = home.to_string_lossy();
    match p.strip_prefix(h.as_ref()) {
        Some("") => "~".into(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

impl Drawer {
    pub fn new(app: &gtk::Application) -> Rc<Drawer> {
        let (cfg, _) = Config::load();
        let window = gtk::ApplicationWindow::new(app);
        window.add_css_class("roundhouse");
        window.set_decorated(false);
        window.set_title(Some("Roundhouse"));
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("roundhouse"));
        for e in [LsEdge::Top, LsEdge::Bottom, LsEdge::Left, LsEdge::Right] {
            window.set_anchor(e, true);
        }
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        // ---- header
        let lamp = label("\u{25cf}", &["lamp", "calm"]);
        let host = label("", &["host"]);
        let sub = label("", &["sub"]);
        let who = gtk::Box::new(gtk::Orientation::Vertical, 0);
        who.append(&host);
        who.append(&sub);
        who.set_hexpand(true);
        let chip = label("calm", &["chip", "calm"]);
        chip.set_valign(gtk::Align::Center);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.append(&lamp);
        header.append(&who);
        header.append(&chip);

        // ---- tabs
        let tabs_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        tabs_box.add_css_class("tabs");
        tabs_box.set_homogeneous(true);
        let mut tabs = vec![];
        let order: Vec<String> = cfg
            .drawer
            .tabs
            .iter()
            .filter(|t| TABS.iter().any(|(id, _)| id == t))
            .cloned()
            .collect();
        for id in &order {
            let title = TABS
                .iter()
                .find(|(i, _)| i == id)
                .map(|(_, t)| *t)
                .unwrap_or(id);
            let b = gtk::Button::with_label(title);
            b.set_focus_on_click(false);
            tabs_box.append(&b);
            tabs.push((id.clone(), b));
        }

        let notice = label("", &["notice"]);
        notice.set_wrap(true);
        notice.set_visible(false);

        // ---- machine page
        let verdict = gtk::Box::new(gtk::Orientation::Vertical, 2);
        verdict.add_css_class("verdict");
        let reasons = gtk::Box::new(gtk::Orientation::Vertical, 0);
        verdict.append(&reasons);
        verdict.set_visible(false);
        let (cpu, gpu, mem, psi) = (
            Tile::new("CPU"),
            Tile::new("GPU"),
            Tile::new("MEMORY"),
            Tile::new("PRESSURE"),
        );
        psi.spark.area.set_visible(false);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(8);
        grid.set_row_spacing(8);
        grid.set_column_homogeneous(true);
        let mut tiles = vec![];
        for (i, t) in [&cpu, &gpu, &mem, &psi].iter().enumerate() {
            let s = Spring::new(&t.root, Motion::TILE);
            grid.attach(&s, (i % 2) as i32, (i / 2) as i32, 1, 1);
            tiles.push(s);
        }
        let apps_card = card("MEMORY BY APP", "NOT BY PID");
        let apps = Rows::new();
        apps_card.append(&apps.root);
        let mroot = gtk::Box::new(gtk::Orientation::Vertical, 8);
        mroot.append(&verdict);
        mroot.append(&grid);
        mroot.append(&apps_card);
        let machine = MachinePage {
            root: mroot,
            verdict,
            reasons,
            reasons_sig: RefCell::new(String::new()),
            cpu,
            gpu,
            mem,
            psi,
            tiles,
            apps,
            apps_card,
        };

        // ---- storage page
        let srows = Rows::new();
        let scard = card("DISKS AND SHARES", "");
        scard.append(&srows.root);
        let sroot = gtk::Box::new(gtk::Orientation::Vertical, 8);
        sroot.append(&scard);
        let storage = StoragePage {
            root: sroot,
            rows: srows,
        };

        // ---- switchyard page
        let yrows = Rows::new();
        let ycard = card("SWITCHYARD", "SIBLING");
        ycard.append(&yrows.root);
        let handoff = gtk::Button::with_label("Open the Switchyard board");
        handoff.add_css_class("handoff");
        let yroot = gtk::Box::new(gtk::Orientation::Vertical, 8);
        yroot.append(&ycard);
        yroot.append(&handoff);
        let yard = YardPage {
            root: yroot,
            rows: yrows,
        };

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(220);
        stack.set_vhomogeneous(false);
        stack.set_interpolate_size(true);
        for (id, page) in [
            ("machine", &machine.root),
            ("storage", &storage.root),
            ("switchyard", &yard.root),
        ] {
            stack.add_named(page, Some(id));
        }
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&stack));
        scroller.set_vexpand(true);

        let foot = label("", &["foot"]);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 10);
        panel.add_css_class("panel");
        panel.append(&header);
        panel.append(&tabs_box);
        panel.append(&notice);
        panel.append(&scroller);
        panel.append(&foot);

        let spring = Spring::new(&panel, Motion::PANEL);
        let stage = Stage::new(
            &spring,
            cfg.drawer.width,
            cfg.drawer.max_height,
            cfg.drawer.gap,
        );
        window.set_child(Some(&stage));

        let first = order.first().cloned().unwrap_or_else(|| "machine".into());
        let d = Rc::new(Drawer {
            me: RefCell::new(std::rc::Weak::new()),
            window,
            stage,
            spring,
            visible: Cell::new(false),
            anchor: RefCell::new(None),
            snap: RefCell::new(None),
            cfg: RefCell::new(cfg),
            lamp,
            host,
            sub,
            chip,
            notice,
            tabs,
            tab: RefCell::new(first),
            stack,
            machine,
            storage,
            yard,
            foot,
        });

        *d.me.borrow_mut() = Rc::downgrade(&d);
        {
            let w = Rc::downgrade(&d);
            d.stage.connect_outside(move || {
                if let Some(d) = w.upgrade() {
                    d.hide();
                }
            });
            let s = d.spring.clone();
            d.stage.connect_placed(move |(x, y)| s.set_origin(x, y));
        }
        for (id, b) in &d.tabs {
            let w = Rc::downgrade(&d);
            let id = id.clone();
            b.connect_clicked(move |_| {
                if let Some(d) = w.upgrade() {
                    d.select(&id);
                }
            });
        }
        {
            let w = Rc::downgrade(&d);
            handoff.connect_clicked(move |_| {
                let Some(d) = w.upgrade() else { return };
                let anchor = d.anchor.borrow().clone();
                d.hide();
                std::thread::spawn(move || {
                    rh_core::sibling::switchyard_show(anchor.as_ref());
                });
            });
        }
        {
            let keys = gtk::EventControllerKey::new();
            let w = Rc::downgrade(&d);
            keys.connect_key_pressed(move |_, key, _, _| {
                let Some(d) = w.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                d.key(key)
            });
            d.window.add_controller(keys);
        }
        d.select_quiet();
        d
    }

    pub fn handle(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::Event(Event::Snapshot { snapshot }) => {
                *self.snap.borrow_mut() = Some(*snapshot);
                if self.visible.get() {
                    self.render();
                }
            }
            Msg::Event(Event::Show { anchor, tab }) => self.show(anchor, tab),
            Msg::Event(Event::Toggle { anchor, tab }) => {
                if self.visible.get() && tab.as_ref().is_none_or(|t| *t == *self.tab.borrow()) {
                    self.hide();
                } else {
                    self.show(anchor, tab);
                }
            }
            Msg::Event(Event::Hide) => self.hide(),
            Msg::Restyle => {
                crate::linux::apply_styles();
                let (cfg, _) = Config::load();
                self.stage
                    .set_geometry(cfg.drawer.width, cfg.drawer.max_height, cfg.drawer.gap);
                *self.cfg.borrow_mut() = cfg;
            }
        }
    }

    fn key(self: &Rc<Self>, key: gdk::Key) -> glib::Propagation {
        if key == gdk::Key::Escape {
            self.hide();
            return glib::Propagation::Stop;
        }
        let visible: Vec<String> = self
            .tabs
            .iter()
            .filter(|(_, b)| b.is_visible())
            .map(|(id, _)| id.clone())
            .collect();
        if let Some(n) = key.to_unicode().and_then(|c| c.to_digit(10)) {
            if let Some(id) = visible.get((n as usize).wrapping_sub(1)) {
                self.select(id);
                return glib::Propagation::Stop;
            }
        }
        let step: isize = match key {
            gdk::Key::Tab | gdk::Key::Right | gdk::Key::l => 1,
            gdk::Key::ISO_Left_Tab | gdk::Key::Left | gdk::Key::h => -1,
            _ => return glib::Propagation::Proceed,
        };
        let cur = visible
            .iter()
            .position(|t| *t == *self.tab.borrow())
            .unwrap_or(0) as isize;
        let n = visible.len() as isize;
        if n > 0 {
            let next = ((cur + step) % n + n) % n;
            self.select(&visible[next as usize]);
        }
        glib::Propagation::Stop
    }

    fn select_quiet(&self) {
        let tab = self.tab.borrow().clone();
        for (id, b) in &self.tabs {
            if *id == tab {
                b.add_css_class("active");
            } else {
                b.remove_css_class("active");
            }
        }
        self.stack.set_visible_child_name(&tab);
    }

    fn select(self: &Rc<Self>, id: &str) {
        if !self.tabs.iter().any(|(t, b)| t == id && b.is_visible()) {
            return;
        }
        *self.tab.borrow_mut() = id.to_string();
        self.select_quiet();
        self.render();
    }

    fn show(self: &Rc<Self>, anchor: Option<Anchor>, tab: Option<String>) {
        if let Some(name) = anchor.as_ref().and_then(|a| a.monitor.clone()) {
            if let Some(m) = find_monitor(&name) {
                self.window.set_monitor(Some(&m));
            }
        }
        *self.anchor.borrow_mut() = anchor.clone();
        self.stage.set_anchor(anchor);
        let was_visible = self.visible.replace(true);
        self.render();
        if let Some(t) = tab {
            if self.tabs.iter().any(|(id, _)| *id == t) {
                *self.tab.borrow_mut() = t;
                self.select_quiet();
            }
        }
        self.window.set_visible(true);
        self.window.present();
        if !was_visible {
            self.spring.play_in(0);
            if *self.tab.borrow() == "machine" {
                for (i, t) in self.machine.tiles.iter().enumerate() {
                    t.play_in(90 + i as u32 * 45);
                }
            }
            send_quiet(Request::BoardVisible { visible: true });
        }
    }

    fn hide(self: &Rc<Self>) {
        if !self.visible.replace(false) {
            return;
        }
        let w = Rc::downgrade(self);
        self.spring.play_out(move || {
            if let Some(d) = w.upgrade() {
                if !d.visible.get() {
                    d.window.set_visible(false);
                }
            }
        });
        send_quiet(Request::BoardVisible { visible: false });
    }

    // ------------------------------------------------------------- render

    fn render(&self) {
        let snap = self.snap.borrow();
        let Some(s) = snap.as_ref() else {
            self.host.set_label("Roundhouse");
            self.sub.set_label("waiting for the daemon");
            return;
        };
        let cfg = self.cfg.borrow();

        self.host.set_label(&s.host.name.to_uppercase());
        let kernel = s.host.kernel.split('-').next().unwrap_or(&s.host.kernel);
        self.sub
            .set_label(&format!("up {} · linux {kernel}", duration(s.host.uptime)));
        let level = s.lamp.level.as_str();
        for c in ["calm", "heads-up", "act"] {
            self.lamp.remove_css_class(c);
            self.chip.remove_css_class(c);
        }
        self.lamp.add_css_class(level);
        self.chip.add_css_class(level);
        let n = s.lamp.reasons.len();
        self.chip.set_label(&match s.lamp.level {
            Level::Calm => "calm".to_string(),
            Level::HeadsUp => format!("{n} heads-up"),
            Level::Act => "needs you".to_string(),
        });
        match &s.config_error {
            Some(e) => {
                self.notice
                    .set_label(&format!("Config problem, using defaults: {e}"));
                self.notice.set_visible(true);
            }
            None => self.notice.set_visible(false),
        }

        // Sibling tabs come and go with their sibling.
        let yard_on = s.siblings.switchyard.is_some();
        for (id, b) in &self.tabs {
            if id == "switchyard" {
                b.set_visible(yard_on);
            }
        }
        if !yard_on && *self.tab.borrow() == "switchyard" {
            *self.tab.borrow_mut() = "machine".into();
            self.select_quiet();
        }

        self.render_machine(s);
        self.render_storage(s, &cfg);
        self.render_yard(s);
        let n_tabs = self.tabs.iter().filter(|(_, b)| b.is_visible()).count();
        self.foot
            .set_label(&format!("1-{n_tabs} tabs · Esc close · {}", s.host.os));
    }

    fn render_machine(&self, s: &Snapshot) {
        let m = &s.machine;
        let p = &self.machine;

        // Verdict: the reasons behind the lamp, each one a link to its tab.
        let sig: String = s
            .lamp
            .reasons
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if *p.reasons_sig.borrow() != sig {
            while let Some(c) = p.reasons.first_child() {
                p.reasons.remove(&c);
            }
            for r in &s.lamp.reasons {
                let b = gtk::Button::with_label(&r.text);
                if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                    l.set_xalign(0.0);
                    l.set_wrap(true);
                }
                let tab = r.tab.clone();
                let w = self.me.borrow().clone();
                b.connect_clicked(move |_| {
                    if let Some(d) = w.upgrade() {
                        d.select(&tab);
                    }
                });
                p.reasons.append(&b);
            }
            *p.reasons_sig.borrow_mut() = sig;
        }
        for c in ["heads-up", "act"] {
            p.verdict.remove_css_class(c);
        }
        if s.lamp.level != Level::Calm {
            p.verdict.add_css_class(s.lamp.level.as_str());
        }
        p.verdict.set_visible(!s.lamp.reasons.is_empty());

        p.cpu
            .title
            .set_label(&format!("CPU · {}", m.cpu.model.to_uppercase()));
        p.cpu
            .aside
            .set_label(&m.cpu.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default());
        p.cpu.value.set_label(&format!("{:.0}", m.cpu.usage));
        p.cpu.unit.set_label(&match m.cpu.mhz {
            Some(mhz) => format!("% · {:.1} GHz", mhz / 1000.0),
            None => "%".into(),
        });
        p.cpu.spark.set(&m.cpu.history, 100.0);

        match m.gpus.first() {
            Some(g) => {
                p.tiles[1].set_visible(true);
                p.gpu
                    .title
                    .set_label(&format!("GPU · {}", g.name.to_uppercase()));
                p.gpu
                    .aside
                    .set_label(&g.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default());
                p.gpu.value.set_label(
                    &g.usage
                        .map(|u| format!("{u:.0}"))
                        .unwrap_or_else(|| "-".into()),
                );
                let mut unit = "%".to_string();
                if let (Some(u), Some(t)) = (g.vram_used, g.vram_total) {
                    unit.push_str(&format!(" · {} / {}", human(u), human(t)));
                }
                p.gpu.unit.set_label(&unit);
                p.gpu.spark.set(&g.history, 100.0);
            }
            None => p.tiles[1].set_visible(false),
        }

        let mem = &m.memory;
        p.mem.aside.set_label(&if mem.swap_total > 0 {
            format!("SWAP {}", human(mem.swap_used))
        } else {
            String::new()
        });
        p.mem.value.set_label(&human(mem.used()).replace(' ', ""));
        p.mem.unit.set_label(&format!("/ {}", human(mem.total)));
        p.mem.spark.set(&mem.history, 100.0);

        let psi = |x: Option<rh_core::model::Psi>| {
            x.map(|p| format!("{:.1}", p.some10))
                .unwrap_or_else(|| "-".into())
        };
        p.psi.aside.set_label("SOME · 10 S");
        p.psi.value.set_label(&psi(m.pressure.memory));
        p.psi.unit.set_label(&format!(
            "mem · cpu {} · io {}",
            psi(m.pressure.cpu),
            psi(m.pressure.io)
        ));

        let total = mem.total.max(1) as f64;
        let rows: Vec<RowData> = s
            .apps
            .iter()
            .take(6)
            .map(|a| {
                let (right, meter, class) = match a.cap {
                    Some(cap) => {
                        let pct = a.mem as f32 / cap.max(1) as f32 * 100.0;
                        (
                            format!("{} / {}", human(a.mem), human(cap)),
                            a.mem as f64 / cap.max(1) as f64,
                            level_class(pct, 75.0, 90.0),
                        )
                    }
                    None => (human(a.mem), a.mem as f64 / total, ""),
                };
                RowData {
                    name: a.name.clone(),
                    detail: a.detail.clone(),
                    right,
                    meter: Some(meter),
                    meter_class: if a.cap.is_some() && class.is_empty() {
                        "calm"
                    } else {
                        class
                    },
                    ..Default::default()
                }
            })
            .collect();
        p.apps_card.set_visible(!rows.is_empty());
        p.apps.set(&rows);
    }

    fn render_storage(&self, s: &Snapshot, cfg: &Config) {
        let t = &cfg.lamp;
        let mut rows = vec![];
        for d in &s.disks {
            let name = match (d.kind, &d.model) {
                (DiskKind::Network, _) => {
                    let src = d.name.trim_start_matches("//");
                    match src.split_once('/') {
                        Some((host, share)) => format!("{host} · {share}"),
                        None => src.to_string(),
                    }
                }
                (DiskKind::OnDemand, _) => d.name.clone(),
                (_, Some(m)) => m.clone(),
                (_, None) => d.name.clone(),
            };
            let detail = match d.kind {
                DiskKind::Local => d.name.clone(),
                DiskKind::Network => "network share".into(),
                DiskKind::OnDemand => "automount".into(),
            };
            let (right, right_class) = match (d.kind, d.temp) {
                (DiskKind::OnDemand, _) => ("not mounted, not woken".to_string(), ""),
                (_, Some(temp)) => (
                    format!("{temp:.0}°C"),
                    level_class(temp, t.drive_temp_warn, t.drive_temp_act),
                ),
                _ => (String::new(), ""),
            };
            rows.push(RowData {
                name,
                detail,
                right,
                right_class,
                head: true,
                ..Default::default()
            });
            if d.kind == DiskKind::OnDemand {
                continue;
            }
            for f in &d.filesystems {
                let more = if f.mounts.len() > 1 {
                    format!(" +{}", f.mounts.len() - 1)
                } else {
                    String::new()
                };
                let pct = f.used_pct();
                let (right, right_class) = if f.stale {
                    ("not answering".to_string(), "heads-up")
                } else {
                    (
                        format!("{} / {} · {pct:.0}%", human(f.used), human(f.total)),
                        "",
                    )
                };
                rows.push(RowData {
                    name: format!("{}{more}", tilde(&f.mounts[0])),
                    detail: f.fstype.clone(),
                    right,
                    right_class,
                    meter: Some(pct as f64 / 100.0),
                    meter_class: level_class(pct, t.disk_warn, t.disk_act),
                    head: false,
                });
            }
        }
        self.storage.rows.set(&rows);
    }

    fn render_yard(&self, s: &Snapshot) {
        let Some(y) = &s.siblings.switchyard else {
            self.yard.rows.set(&[]);
            return;
        };
        let names = |ids: &[String]| {
            ids.iter()
                .map(|i| y.label(i).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut rows = vec![RowData {
            name: "Main line".into(),
            detail: if y.waiting_memory {
                "waiting for free memory".into()
            } else {
                String::new()
            },
            right: y
                .current
                .as_deref()
                .map(|c| format!("building {}", y.label(c)))
                .unwrap_or_else(|| "free".into()),
            right_class: if y.waiting_memory { "heads-up" } else { "" },
            ..Default::default()
        }];
        if !y.queue.is_empty() {
            rows.push(RowData {
                name: "Queued".into(),
                right: names(&y.queue),
                ..Default::default()
            });
        }
        if !y.building.is_empty() {
            rows.push(RowData {
                name: "Building".into(),
                right: names(&y.building),
                ..Default::default()
            });
        }
        if !y.running.is_empty() {
            rows.push(RowData {
                name: "Running".into(),
                right: names(&y.running),
                ..Default::default()
            });
        }
        if !y.failed.is_empty() {
            rows.push(RowData {
                name: "Failed".into(),
                right: names(&y.failed),
                right_class: "act",
                ..Default::default()
            });
        }
        rows.push(RowData {
            name: "Tasks".into(),
            detail: y.phase.clone(),
            right: y.tasks.to_string(),
            ..Default::default()
        });
        self.yard.rows.set(&rows);
    }
}
