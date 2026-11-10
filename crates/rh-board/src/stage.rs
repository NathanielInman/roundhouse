//! `Stage` (from Switchyard): fills the layer surface and places the panel
//! next to the bar with [`bar_anchor::place`]. A click on the empty area
//! around the panel closes the drawer.

use bar_anchor::{place, Anchor, Rect, SIDE_MARGIN};
use gtk::glib;
use gtk::graphene::Point;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

type OnPlaced = Box<dyn Fn((f64, f64))>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Stage {
        pub anchor: RefCell<Option<Anchor>>,
        pub panel_width: Cell<f64>,
        pub max_height: Cell<f64>,
        pub gap: Cell<f64>,
        pub rect: Cell<Rect>,
        pub origin: Cell<(f64, f64)>,
        pub on_outside: RefCell<Option<Box<dyn Fn()>>>,
        pub on_placed: RefCell<Option<OnPlaced>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Stage {
        const NAME: &'static str = "RhStage";
        type Type = super::Stage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Stage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let click = gtk::GestureClick::new();
            click.set_button(0);
            let weak = obj.downgrade();
            click.connect_pressed(move |g, _, x, y| {
                let Some(stage) = weak.upgrade() else { return };
                if !stage.imp().rect.get().contains(x, y) {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    if let Some(f) = stage.imp().on_outside.borrow().as_ref() {
                        f();
                    }
                }
            });
            obj.add_controller(click);
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Stage {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let obj = self.obj();
            let Some(child) = obj.first_child() else {
                return;
            };
            let (sw, sh) = (width as f64, height as f64);
            let gap = self.gap.get();
            let want_w = self.panel_width.get().min(sw - 2.0 * SIDE_MARGIN);
            let (min_w, _, _, _) = child.measure(gtk::Orientation::Horizontal, -1);
            let pw = want_w.max(min_w as f64);
            let (min_h, nat_h, _, _) = child.measure(gtk::Orientation::Vertical, pw as i32);
            let max_h = (sh * self.max_height.get()).max(min_h as f64);
            let ph = (nat_h as f64).min(max_h).max(min_h as f64);
            let anchor = self.anchor.borrow().clone();
            let (rect, origin) = place(anchor.as_ref(), sw, sh, pw, ph, gap);
            self.rect.set(rect);
            self.origin.set(origin);
            let t = gtk::gsk::Transform::new()
                .translate(&Point::new(rect.x.round() as f32, rect.y.round() as f32));
            child.allocate(rect.w.round() as i32, rect.h.round() as i32, -1, Some(t));
            if let Some(f) = self.on_placed.borrow().as_ref() {
                f(origin);
            }
        }
    }
}

glib::wrapper! {
    pub struct Stage(ObjectSubclass<imp::Stage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Stage {
    pub fn new(child: &impl IsA<gtk::Widget>, width: f64, max_height: f64, gap: f64) -> Stage {
        let s: Stage = glib::Object::new();
        child.set_parent(&s);
        s.imp().panel_width.set(width);
        s.imp().max_height.set(max_height);
        s.imp().gap.set(gap);
        s
    }

    pub fn set_anchor(&self, a: Option<Anchor>) {
        *self.imp().anchor.borrow_mut() = a;
        self.queue_allocate();
    }

    pub fn set_geometry(&self, width: f64, max_height: f64, gap: f64) {
        self.imp().panel_width.set(width);
        self.imp().max_height.set(max_height);
        self.imp().gap.set(gap);
        self.queue_resize();
    }

    pub fn connect_outside(&self, f: impl Fn() + 'static) {
        *self.imp().on_outside.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_placed(&self, f: impl Fn((f64, f64)) + 'static) {
        *self.imp().on_placed.borrow_mut() = Some(Box::new(f));
    }
}
