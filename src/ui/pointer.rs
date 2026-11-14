//! Mouse, touchpad, trackpoint and trackball pages. They share most options.

use super::widgets::{self, NumSpec};
use super::{Ctx, path};
use adw::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Device {
    Mouse,
    Touchpad,
    Trackpoint,
    Trackball,
}

impl Device {
    fn node(self) -> &'static str {
        match self {
            Device::Mouse => "mouse",
            Device::Touchpad => "touchpad",
            Device::Trackpoint => "trackpoint",
            Device::Trackball => "trackball",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Device::Mouse => "mice",
            Device::Touchpad => "touchpads",
            Device::Trackpoint => "trackpoints",
            Device::Trackball => "trackballs",
        }
    }
}

const ACCEL_PROFILES: &[(&str, Option<&str>)] = &[
    ("Default (adaptive)", None),
    ("Adaptive", Some("adaptive")),
    ("Flat (no acceleration)", Some("flat")),
];

pub fn page(ctx: &Ctx, dev: Device) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let d = dev.node();
    let p = |leaf: &str| path(&["input", d, leaf]);

    // ── General ──
    let general = widgets::group("General", "");
    general.add(&widgets::switch(
        ctx,
        "Disable",
        &format!("Ignore all {}", dev.noun()),
        p("off"),
    ));
    general.add(&widgets::switch(
        ctx,
        "Left-handed",
        "Swap the primary and secondary buttons",
        p("left-handed"),
    ));
    if dev == Device::Touchpad {
        general.add(&widgets::switch(
            ctx,
            "Disable when an external mouse is connected",
            "",
            p("disabled-on-external-mouse"),
        ));
    }
    page.add(&general);

    if dev == Device::Touchpad {
        let palm = widgets::group(
            "Palm Rejection",
            "Turn these off to keep the touchpad active while typing. libinput's built-in \
             palm detection (by touch size and pressure) can't be changed from niri.",
        );
        palm.add(&widgets::switch(
            ctx,
            "Disable while typing",
            "Ignore the touchpad briefly after key presses",
            p("dwt"),
        ));
        palm.add(&widgets::switch(
            ctx,
            "Disable while using trackpoint",
            "Ignore the touchpad while the trackpoint is in use",
            p("dwtp"),
        ));
        page.add(&palm);
    }

    // ── Pointer speed ──
    let speed = widgets::group("Pointer Speed", "");
    speed.add(&widgets::slider(ctx, "Speed", "Slow ← → Fast", p("accel-speed"), -1.0, 1.0, 0.0));
    speed.add(&widgets::combo(
        ctx,
        "Acceleration profile",
        "Flat gives a constant 1:1 response",
        p("accel-profile"),
        ACCEL_PROFILES,
    ));
    page.add(&speed);

    // ── Clicking / tapping ──
    if dev == Device::Touchpad {
        let tap = widgets::group("Tapping & Clicking", "");
        tap.add(&widgets::switch(ctx, "Tap to click", "Tap the touchpad instead of pressing it", p("tap")));
        tap.add(&widgets::combo(
            ctx,
            "Multi-finger tap",
            "What two- and three-finger taps do",
            p("tap-button-map"),
            &[
                ("Default", None),
                ("2 fingers: right, 3 fingers: middle", Some("left-right-middle")),
                ("2 fingers: middle, 3 fingers: right", Some("left-middle-right")),
            ],
        ));
        tap.add(&widgets::bool_arg(
            ctx,
            "Tap and drag",
            "Tap, then tap and hold to drag",
            p("drag"),
            true,
        ));
        tap.add(&widgets::switch(
            ctx,
            "Drag lock",
            "Lifting the finger briefly doesn't end a drag",
            p("drag-lock"),
        ));
        tap.add(&widgets::combo(
            ctx,
            "Click method",
            "How physical clicks become right/middle clicks",
            p("click-method"),
            &[
                ("Default", None),
                ("Button areas (bottom corners)", Some("button-areas")),
                ("Clickfinger (number of fingers)", Some("clickfinger")),
            ],
        ));
        tap.add(&widgets::switch(
            ctx,
            "Middle-click emulation",
            "Press left and right together for a middle click",
            p("middle-emulation"),
        ));
        page.add(&tap);
    } else {
        let buttons = widgets::group("Buttons", "");
        buttons.add(&widgets::switch(
            ctx,
            "Middle-click emulation",
            "Press left and right together for a middle click",
            p("middle-emulation"),
        ));
        page.add(&buttons);
    }

    // ── Scrolling ──
    let scroll = widgets::group("Scrolling", "");
    scroll.add(&widgets::switch(
        ctx,
        "Natural scrolling",
        "Content follows your fingers, like on a phone",
        p("natural-scroll"),
    ));
    scroll.add(&widgets::spin(
        ctx,
        "Scroll speed",
        "Multiplier for scroll distance (1.0 = normal)",
        p("scroll-factor"),
        NumSpec { min: 0.05, max: 10.0, step: 0.05, digits: 2, float: true, default: 1.0 },
    ));
    let methods: &[(&str, Option<&str>)] = match dev {
        Device::Touchpad => &[
            ("Default (two-finger)", None),
            ("Two-finger", Some("two-finger")),
            ("Edge", Some("edge")),
            ("On button down", Some("on-button-down")),
            ("Disabled", Some("no-scroll")),
        ],
        _ => &[
            ("Default", None),
            ("Wheel only", Some("no-scroll")),
            ("Hold button and move", Some("on-button-down")),
        ],
    };
    scroll.add(&widgets::combo(ctx, "Scroll method", "", p("scroll-method"), methods));
    if dev != Device::Touchpad {
        scroll.add(&widgets::spin(
            ctx,
            "Scroll button",
            "Button code used with “Hold button and move” (e.g. 273 = right, 274 = middle)",
            p("scroll-button"),
            NumSpec { min: 0.0, max: 1000.0, step: 1.0, digits: 0, float: false, default: 0.0 },
        ));
        scroll.add(&widgets::switch(
            ctx,
            "Scroll button lock",
            "Click the scroll button once to toggle scrolling instead of holding it",
            p("scroll-button-lock"),
        ));
    }
    page.add(&scroll);

    page
}

/// One "Input" page with a tab per pointer device.
pub fn input_page(ctx: &Ctx) -> gtk::Widget {
    let stack = adw::ViewStack::new();
    stack.set_vexpand(true);
    let tabs = [
        (Device::Touchpad, "Touchpad", "input-touchpad-symbolic"),
        (Device::Mouse, "Mouse", "input-mouse-symbolic"),
        (Device::Trackpoint, "Trackpoint", "input-mouse-symbolic"),
        (Device::Trackball, "Trackball", "input-mouse-symbolic"),
    ];
    // Only the first tab is built up front; the others on first view.
    let mut bins = Vec::new();
    for (i, (dev, title, icon)) in tabs.into_iter().enumerate() {
        let bin = adw::Bin::new();
        if i == 0 {
            bin.set_child(Some(&page(ctx, dev)));
        }
        stack.add_titled_with_icon(&bin, Some(dev.node()), title, icon);
        bins.push((dev, bin));
    }
    {
        let ctx = ctx.clone();
        stack.connect_visible_child_notify(move |stack| {
            let Some(child) = stack.visible_child() else { return };
            if let Some((dev, bin)) = bins.iter().find(|(_, b)| b.upcast_ref::<gtk::Widget>() == &child) {
                if bin.child().is_none() {
                    bin.set_child(Some(&page(&ctx, *dev)));
                }
            }
        });
    }
    let switcher = adw::InlineViewSwitcher::builder()
        .stack(&stack)
        .display_mode(adw::InlineViewSwitcherDisplayMode::Labels)
        .homogeneous(true)
        .build();
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .margin_top(12)
        .margin_start(12)
        .margin_end(12)
        .child(&switcher)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&clamp);
    root.append(&stack);
    root.upcast()
}
