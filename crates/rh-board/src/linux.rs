//! Startup: styles, the daemon subscription, and file watches that restyle
//! the drawer when the bar's stylesheet or Roundhouse's config changes.

use crate::drawer::{Drawer, Msg};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use rh_core::config::Config;
use rh_core::protocol::SubKind;
use std::cell::RefCell;
use std::time::Duration;

const DEFAULT_CSS: &str = include_str!("style.css");

thread_local! {
    static THEME: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static USER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static WATCHES: RefCell<Vec<gio::FileMonitor>> = const { RefCell::new(Vec::new()) };
}

fn provider(
    slot: &'static std::thread::LocalKey<RefCell<Option<gtk::CssProvider>>>,
    priority: u32,
) -> Option<gtk::CssProvider> {
    let display = gdk::Display::default()?;
    Some(slot.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| {
                let p = gtk::CssProvider::new();
                gtk::style_context_add_provider_for_display(&display, &p, priority);
                p
            })
            .clone()
    }))
}

/// Applies the theme layers in order: built-in, the bar's look, the user's
/// own stylesheet.
pub fn apply_styles() {
    let (cfg, _) = Config::load();
    if let Some(p) = provider(&THEME, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1) {
        let bar = rh_core::waybar::load_theme();
        p.load_from_string(&crate::theme::css(bar.as_ref(), &cfg));
    }
    if let Some(p) = provider(&USER, gtk::STYLE_PROVIDER_PRIORITY_USER) {
        let user = std::fs::read_to_string(rh_core::paths::style_file()).unwrap_or_default();
        p.load_from_string(&user);
    }
}

fn watch(path: std::path::PathBuf, on_change: impl Fn() + 'static) {
    let file = gio::File::for_path(&path);
    if let Ok(m) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        m.connect_changed(move |_, _, _, ev| {
            if matches!(
                ev,
                gio::FileMonitorEvent::ChangesDoneHint
                    | gio::FileMonitorEvent::Created
                    | gio::FileMonitorEvent::Deleted
            ) {
                on_change();
            }
        });
        WATCHES.with(|w| w.borrow_mut().push(m));
    }
}

/// Keeps a subscription to the daemon open, reconnecting forever.
fn connection(tx: async_channel::Sender<Msg>) {
    loop {
        if let Ok(events) = rh_core::protocol::subscribe(SubKind::Board) {
            for ev in events {
                match ev {
                    Ok(e) => {
                        if tx.send_blocking(Msg::Event(e)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

pub fn main() -> glib::ExitCode {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("rh-board {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    let app = gtk::Application::builder()
        .application_id("io.github.nathanielinman.Roundhouse")
        // The daemon decides when a drawer starts; a second one (a dev build)
        // must not hand itself to the first over D-Bus.
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let built = std::rc::Rc::new(std::cell::Cell::new(false));
    app.connect_activate(move |app| {
        if built.replace(true) {
            return;
        }
        if !gtk4_layer_shell::is_supported() {
            eprintln!("rh-board: this compositor does not support wlr-layer-shell");
        }
        if let Some(display) = gdk::Display::default() {
            let base = gtk::CssProvider::new();
            base.load_from_string(DEFAULT_CSS);
            gtk::style_context_add_provider_for_display(
                &display,
                &base,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        apply_styles();
        let (tx, rx) = async_channel::unbounded::<Msg>();
        let drawer = Drawer::new(app);
        for path in [
            rh_core::waybar::style_path(),
            rh_core::paths::style_file(),
            rh_core::paths::config_file(),
        ] {
            let tx = tx.clone();
            watch(path, move || {
                let _ = tx.send_blocking(Msg::Restyle);
            });
        }
        std::mem::forget(app.hold());
        std::thread::spawn(move || connection(tx));
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                drawer.handle(msg);
            }
        });
    });
    app.run_with_args::<&str>(&[])
}
