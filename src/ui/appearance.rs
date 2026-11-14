//! Focus ring, borders, shadows, colors, animations and the overview.

use super::widgets::{self, NumSpec, Target};
use super::{Ctx, Path, path, refs};
use crate::kdl::{self, Value};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let windows = widgets::group("Window decorations", "");
    windows.add(&outline(ctx, "focus-ring", "Focus ring", "Outline drawn around the focused window", true, "#7fc8ff"));
    windows.add(&outline(ctx, "border", "Border", "Outline drawn around every window, inside its size", false, "#ffc87f"));
    windows.add(&shadow(ctx));
    windows.add(&widgets::switch(
        ctx,
        "Prefer server-side decorations",
        "Ask apps to drop their own title bars (prefer-no-csd)",
        path(&["prefer-no-csd"]),
    ));
    page.add(&windows);

    let colors = widgets::group("Workspace", "");
    colors.add(&widgets::color(
        ctx,
        "Background color",
        "Shown behind windows when no wallpaper covers it",
        path(&["layout", "background-color"]),
        "#262626",
    ));
    let hint = widgets::block_expander(
        ctx,
        "Insert hint",
        "Preview where a window will land while dragging it",
        &["layout", "insert-hint"],
        true,
    );
    let c = ctx.clone();
    widgets::lazy_rows(&hint, move |hint| {
        hint.add_row(&widgets::color(&c, "Color", "", path(&["layout", "insert-hint", "color"]), "#ffc87f80"));
    });
    colors.add(&hint);
    page.add(&colors);

    let anim = widgets::group("Animations", "");
    anim.add(&widgets::off_switch(ctx, "Animations", "", path(&["animations", "off"])));
    anim.add(&widgets::spin(
        ctx,
        "Slowdown",
        "Multiplies every animation's duration; below 1 speeds them up",
        path(&["animations", "slowdown"]),
        NumSpec { min: 0.1, max: 10.0, step: 0.1, digits: 1, float: true, default: 1.0 },
    ));
    page.add(&anim);
    let each = widgets::group("Individual animations", "Turn off or retune a single animation");
    for (key, label, def) in ANIMATIONS {
        each.add(&animation(ctx, key, label, *def));
    }
    page.add(&each);

    let overview = widgets::group("Overview", "");
    overview.add(&widgets::slider(
        ctx,
        "Zoom",
        "How large workspaces appear in the overview",
        path(&["overview", "zoom"]),
        0.1,
        0.75,
        0.5,
    ));
    overview.add(&widgets::color(
        ctx,
        "Backdrop color",
        "",
        path(&["overview", "backdrop-color"]),
        "#262626",
    ));
    let ws = widgets::block_expander(ctx, "Workspace shadows", "", &["overview", "workspace-shadow"], true);
    let c = ctx.clone();
    widgets::lazy_rows(&ws, move |ws| {
        let ctx = &c;
        let wsp = |leaf: &str| path(&["overview", "workspace-shadow", leaf]);
        let px = |default: f64| NumSpec { min: 0.0, max: 500.0, step: 1.0, digits: 0, float: false, default };
        ws.add_row(&widgets::spin(ctx, "Softness", "Blur radius", wsp("softness"), px(40.0)));
        ws.add_row(&widgets::spin(ctx, "Spread", "", wsp("spread"), px(10.0)));
        offset_rows(ctx, ws, wsp("offset"), (0.0, 10.0));
        ws.add_row(&widgets::color(ctx, "Color", "", wsp("color"), "#00000050"));
    });
    overview.add(&ws);
    page.add(&overview);

    let notices = widgets::group("Notices", "");
    notices.add(&widgets::off_switch(
        ctx,
        "Show shortcuts at startup",
        "Display the important-hotkeys overlay when niri starts",
        path(&["hotkey-overlay", "skip-at-startup"]),
    ));
    notices.add(&widgets::switch(
        ctx,
        "Hide unbound actions in overlay",
        "Leave actions without a shortcut out of the hotkey overlay",
        path(&["hotkey-overlay", "hide-not-bound"]),
    ));
    notices.add(&widgets::off_switch(
        ctx,
        "Config error notification",
        "Tell you when niri fails to load the config",
        path(&["config-notification", "disable-failed"]),
    ));
    page.add(&notices);

    page
}

/// `focus-ring` / `border`: width plus active, inactive and urgent colors.
fn outline(ctx: &Ctx, name: &str, title: &str, subtitle: &str, default_on: bool, active: &str) -> adw::ExpanderRow {
    let row = widgets::block_expander(ctx, title, subtitle, &["layout", name], default_on);
    let (ctx, name, active) = (ctx.clone(), name.to_string(), active.to_string());
    widgets::lazy_rows(&row, move |row| {
        let (ctx, name, active) = (&ctx, name.as_str(), active.as_str());
        row.add_row(&widgets::spin(
            ctx,
            "Width",
            "",
            path(&["layout", name, "width"]),
            NumSpec { min: 0.0, max: 100.0, step: 1.0, digits: 0, float: false, default: 4.0 },
        ));
        let gradient_note = |kind: &str| {
            let has = ctx.store.borrow().get(&["layout", name, &format!("{kind}-gradient")]).is_some();
            if has { "The gradient below takes priority" } else { "" }
        };
        for (label, kind, default) in
            [("Active color", "active", active), ("Inactive color", "inactive", "#505050"), ("Urgent color", "urgent", "#9b0000")]
        {
            row.add_row(&widgets::color(
                ctx,
                label,
                gradient_note(kind),
                path(&["layout", name, &format!("{kind}-color")]),
                default,
            ));
        }
        for (label, kind, default) in [("Active gradient", "active", active), ("Inactive gradient", "inactive", "#505050")] {
            row.add_row(&gradient(ctx, label, path(&["layout", name, &format!("{kind}-gradient")]), default));
        }
    });
    row
}

/// `<kind>-gradient from=… to=… angle=… relative-to=…`; replaces the color.
fn gradient(ctx: &Ctx, title: &str, p: Path, base: &str) -> adw::ExpanderRow {
    let on = ctx.store.borrow().get(&refs(&p)).is_some();
    let row = adw::ExpanderRow::builder()
        .title(title)
        .subtitle("Use a gradient instead of a solid color")
        .show_enable_switch(true)
        .enable_expansion(on)
        .build();
    {
        let (ctx, p, base) = (ctx.clone(), p.clone(), base.to_string());
        widgets::lazy_rows(&row, move |row| {
            let prop = |k: &'static str| Target::Prop(p.clone(), k);
            row.add_row(&widgets::color(&ctx, "From", "", prop("from"), &base));
            row.add_row(&widgets::color(&ctx, "To", "", prop("to"), &base));
            row.add_row(&widgets::spin(
                &ctx,
                "Angle",
                "Degrees; 0 runs bottom to top, 90 left to right",
                prop("angle"),
                NumSpec { min: 0.0, max: 360.0, step: 15.0, digits: 0, float: false, default: 180.0 },
            ));
            row.add_row(&prop_switch(
                &ctx,
                "Span the whole view",
                "Stretch one gradient across the workspace instead of each window",
                p.clone(),
                "relative-to",
                ("window", "workspace-view"),
            ));
        });
    }
    let ctx = ctx.clone();
    let base = base.to_string();
    let guard = Rc::new(Cell::new(false));
    row.connect_enable_expansion_notify(move |row| {
        if guard.get() {
            return;
        }
        let on = row.enables_expansion();
        let name = p.last().cloned().unwrap_or_default();
        let text = on.then(|| format!("{name} from={} to={} angle=180", kdl::quote(&base), kdl::quote(&base)));
        let (p, row, guard) = (p.clone(), row.clone(), guard.clone());
        ctx.apply(move |s| s.set_node(&refs(&p), text), move |ok| {
            if !ok {
                guard.set(true);
                row.set_enable_expansion(!on);
                guard.set(false);
            }
        });
    });
    row
}

/// A string property with two values, the first being niri's default
/// (choosing it removes the property).
fn prop_switch(ctx: &Ctx, title: &str, subtitle: &str, p: Path, key: &'static str, values: (&'static str, &'static str)) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
    let current = ctx.store.borrow().get(&refs(&p)).and_then(|n| n.prop(key)).and_then(|v| v.as_str().map(String::from));
    row.set_active(current.as_deref() == Some(values.1));
    let ctx = ctx.clone();
    let guard = Rc::new(Cell::new(false));
    row.connect_active_notify(move |row| {
        if guard.get() {
            return;
        }
        let on = row.is_active();
        let v = on.then(|| Value::String(values.1.into()));
        let (p, row, guard) = (p.clone(), row.clone(), guard.clone());
        ctx.apply(move |s| s.set_props(&refs(&p), &[(key, v)]), move |ok| {
            if !ok {
                guard.set(true);
                row.set_active(!on);
                guard.set(false);
            }
        });
    });
    row
}

/// Horizontal and vertical offset rows for an `offset x=… y=…` node.
fn offset_rows(ctx: &Ctx, row: &adw::ExpanderRow, p: Path, default: (f64, f64)) {
    let num = |d: f64| NumSpec { min: -500.0, max: 500.0, step: 1.0, digits: 0, float: false, default: d };
    row.add_row(&widgets::spin(ctx, "Horizontal offset", "", Target::Prop(p.clone(), "x"), num(default.0)));
    row.add_row(&widgets::spin(ctx, "Vertical offset", "", Target::Prop(p, "y"), num(default.1)));
}

#[derive(Clone, Copy)]
enum Curve {
    /// damping-ratio, stiffness, epsilon
    Spring(f64, f64, f64),
    /// duration-ms, curve
    Easing(f64, &'static str),
}

const ANIMATIONS: &[(&str, &str, Curve)] = &[
    ("workspace-switch", "Workspace switch", Curve::Spring(1.0, 1000.0, 0.0001)),
    ("horizontal-view-movement", "Scrolling the view", Curve::Spring(1.0, 800.0, 0.0001)),
    ("window-open", "Window open", Curve::Easing(150.0, "ease-out-expo")),
    ("window-close", "Window close", Curve::Easing(150.0, "ease-out-quad")),
    ("window-movement", "Window movement", Curve::Spring(1.0, 800.0, 0.0001)),
    ("window-resize", "Window resize", Curve::Spring(1.0, 800.0, 0.0001)),
    ("overview-open-close", "Overview", Curve::Spring(1.0, 900.0, 0.0001)),
    ("recent-windows-close", "Window switcher close", Curve::Spring(1.0, 800.0, 0.001)),
    ("config-notification-open-close", "Config notification", Curve::Spring(0.6, 1000.0, 0.001)),
    ("exit-confirmation-open-close", "Exit confirmation", Curve::Spring(0.6, 500.0, 0.01)),
    ("screenshot-ui-open", "Screenshot UI", Curve::Easing(200.0, "ease-out-quad")),
];

const CURVES: &[(&str, Option<&str>)] = &[
    ("Default", None),
    ("Ease out (quadratic)", Some("ease-out-quad")),
    ("Ease out (cubic)", Some("ease-out-cubic")),
    ("Ease out (exponential)", Some("ease-out-expo")),
    ("Linear", Some("linear")),
];

/// One animation: on/off, and a spring or easing curve with its parameters.
fn animation(ctx: &Ctx, key: &str, label: &str, def: Curve) -> adw::ExpanderRow {
    let base = ["animations", key];
    let ap = |leaf: &str| path(&["animations", key, leaf]);
    let (enabled, kind) = {
        let s = ctx.store.borrow();
        let kind = if s.get(&[base[0], base[1], "spring"]).is_some() {
            1
        } else if s.get(&[base[0], base[1], "duration-ms"]).is_some() || s.get(&[base[0], base[1], "curve"]).is_some() {
            2
        } else {
            0
        };
        (!s.flag(&[base[0], base[1], "off"]), kind)
    };
    let row = adw::ExpanderRow::builder().title(label).show_enable_switch(true).enable_expansion(enabled).build();
    {
        let (ctx, off) = (ctx.clone(), ap("off"));
        let guard = Rc::new(Cell::new(false));
        row.connect_enable_expansion_notify(move |row| {
            if guard.get() {
                return;
            }
            let on = row.enables_expansion();
            let (off, row, guard) = (off.clone(), row.clone(), guard.clone());
            ctx.apply(move |s| s.set_flag(&refs(&off), !on), move |ok| {
                if !ok {
                    guard.set(true);
                    row.set_enable_expansion(!on);
                    guard.set(false);
                }
            });
        });
    }

    let ctx = ctx.clone();
    let key = key.to_string();
    widgets::lazy_rows(&row, move |row| {
        let (ctx, key) = (&ctx, key.as_str());
        let ap = |leaf: &str| path(&["animations", key, leaf]);
        let default_label = match def {
            Curve::Spring(..) => "Default (spring)",
            Curve::Easing(..) => "Default (easing)",
        };
        let kind_row = adw::ComboRow::builder()
            .title("Motion")
            .subtitle("Springs feel physical; easing runs for a fixed time")
            .model(&gtk::StringList::new(&[default_label, "Spring", "Easing"]))
            .selected(kind)
            .build();
        row.add_row(&kind_row);

        let (sd, ss, se) = match def {
            Curve::Spring(d, s, e) => (d, s, e),
            Curve::Easing(..) => (1.0, 800.0, 0.0001),
        };
        let (ed, ec) = match def {
            Curve::Easing(d, c) => (d, c),
            Curve::Spring(..) => (250.0, "ease-out-cubic"),
        };
        let sp = |k: &'static str| Target::Prop(ap("spring"), k);
        let damping = widgets::spin(
            ctx,
            "Damping ratio",
            "Below 1 bounces, 1 settles without bouncing",
            sp("damping-ratio"),
            NumSpec { min: 0.05, max: 10.0, step: 0.05, digits: 2, float: true, default: sd },
        );
        let stiffness = widgets::spin(
            ctx,
            "Stiffness",
            "Higher is faster",
            sp("stiffness"),
            NumSpec { min: 1.0, max: 100_000.0, step: 50.0, digits: 0, float: false, default: ss },
        );
        let epsilon = widgets::spin(
            ctx,
            "Epsilon",
            "How close to the end counts as finished",
            sp("epsilon"),
            NumSpec { min: 0.00001, max: 1.0, step: 0.0001, digits: 5, float: true, default: se },
        );
        let duration = widgets::spin(
            ctx,
            "Duration (ms)",
            "",
            ap("duration-ms"),
            NumSpec { min: 1.0, max: 10_000.0, step: 10.0, digits: 0, float: false, default: ed },
        );
        let curve = widgets::combo(ctx, "Curve", "", ap("curve"), CURVES);
        for r in [damping.upcast_ref::<gtk::Widget>(), stiffness.upcast_ref(), epsilon.upcast_ref(), duration.upcast_ref(), curve.upcast_ref()] {
            row.add_row(r);
        }
        let show = {
            let (damping, stiffness, epsilon, duration, curve) =
                (damping.clone(), stiffness.clone(), epsilon.clone(), duration.clone(), curve.clone());
            move |k: u32| {
                for r in [&damping, &stiffness, &epsilon] {
                    r.set_visible(k == 1);
                }
                duration.set_visible(k == 2);
                curve.set_visible(k == 2);
            }
        };
        show(kind);

        let ctx = ctx.clone();
        let key = key.to_string();
        let last = Rc::new(Cell::new(kind));
        let guard = Rc::new(Cell::new(false));
        kind_row.connect_selected_notify(move |kr| {
            if guard.get() {
                return;
            }
            let k = kr.selected();
            // Write what the rows show, so switching back and forth keeps values.
            let spring = (k == 1).then(|| {
                format!(
                    "spring damping-ratio={} stiffness={} epsilon={}",
                    kdl::format_float(damping.value()),
                    stiffness.value().round() as i64,
                    kdl::format_float(epsilon.value()),
                )
            });
            let dur = (k == 2).then(|| format!("duration-ms {}", duration.value().round() as i64));
            let crv = (k == 2).then(|| format!("curve {}", kdl::quote(ec)));
            let key = key.clone();
            let (kr, guard, last, show) = (kr.clone(), guard.clone(), last.clone(), show.clone());
            ctx.apply(
                move |s| {
                    s.set_nodes(&[
                        (&["animations", &key, "spring"], spring),
                        (&["animations", &key, "duration-ms"], dur),
                        (&["animations", &key, "curve"], crv),
                    ])
                },
                move |ok| {
                    if ok {
                        last.set(k);
                        show(k);
                    } else {
                        guard.set(true);
                        kr.set_selected(last.get());
                        guard.set(false);
                    }
                },
            );
        });
    });
    row
}

fn shadow(ctx: &Ctx) -> adw::ExpanderRow {
    let row = widgets::block_expander(ctx, "Shadow", "Drop shadow behind windows", &["layout", "shadow"], false);
    let ctx = ctx.clone();
    widgets::lazy_rows(&row, move |row| {
        let ctx = &ctx;
        let num = |default: f64| NumSpec { min: 0.0, max: 200.0, step: 1.0, digits: 0, float: false, default };
        row.add_row(&widgets::spin(ctx, "Softness", "Blur radius", path(&["layout", "shadow", "softness"]), num(30.0)));
        row.add_row(&widgets::spin(
            ctx,
            "Spread",
            "How far the shadow extends",
            path(&["layout", "shadow", "spread"]),
            num(5.0),
        ));
        offset_rows(ctx, &row, path(&["layout", "shadow", "offset"]), (0.0, 5.0));
        row.add_row(&widgets::color(ctx, "Color", "", path(&["layout", "shadow", "color"]), "#00000077"));
        row.add_row(&widgets::color(
            ctx,
            "Inactive color",
            "Defaults to the active color",
            path(&["layout", "shadow", "inactive-color"]),
            "#00000077",
        ));
        row.add_row(&widgets::bool_arg(
            ctx,
            "Draw behind window",
            "Fixes shadows showing through windows with rounded or translucent corners",
            path(&["layout", "shadow", "draw-behind-window"]),
            false,
        ));
    });
    row
}
