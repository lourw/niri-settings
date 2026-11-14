//! Focus ring, borders, shadows, colors, animations and the overview.

use super::widgets::{self, NumSpec};
use super::{Ctx, path};
use adw::prelude::*;

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
    hint.add_row(&widgets::color(ctx, "Color", "", path(&["layout", "insert-hint", "color"]), "#ffc87f80"));
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
    overview.add(&widgets::off_switch(
        ctx,
        "Workspace shadows",
        "",
        path(&["overview", "workspace-shadow", "off"]),
    ));
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
    row.add_row(&widgets::spin(
        ctx,
        "Width",
        "",
        path(&["layout", name, "width"]),
        NumSpec { min: 0.0, max: 100.0, step: 1.0, digits: 0, float: false, default: 4.0 },
    ));
    let gradient_note = |kind: &str| {
        let has = ctx.store.borrow().get(&["layout", name, &format!("{kind}-gradient")]).is_some();
        if has { "A gradient in your config takes priority" } else { "" }
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
    row
}

fn shadow(ctx: &Ctx) -> adw::ExpanderRow {
    let row = widgets::block_expander(ctx, "Shadow", "Drop shadow behind windows", &["layout", "shadow"], false);
    let num = |default: f64| NumSpec { min: 0.0, max: 200.0, step: 1.0, digits: 0, float: false, default };
    row.add_row(&widgets::spin(ctx, "Softness", "Blur radius", path(&["layout", "shadow", "softness"]), num(30.0)));
    row.add_row(&widgets::spin(
        ctx,
        "Spread",
        "How far the shadow extends",
        path(&["layout", "shadow", "spread"]),
        num(5.0),
    ));
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
    row
}
