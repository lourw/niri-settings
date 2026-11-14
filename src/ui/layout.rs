//! Column layout, gaps, struts and tabs (the `layout` section).

use super::widgets::{self, NumSpec};
use super::{Ctx, path};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Common column/window sizes as (label, KDL proportion).
const FRACTIONS: &[(&str, &str)] = &[
    ("¼", "0.25"),
    ("⅓", "0.33333"),
    ("½", "0.5"),
    ("⅔", "0.66667"),
    ("¾", "0.75"),
    ("Full", "1.0"),
];
/// niri's built-in presets when none are configured.
const DEFAULT_PRESETS: &[&str] = &["0.33333", "0.5", "0.66667"];

fn fraction_index(v: f64) -> Option<usize> {
    FRACTIONS.iter().position(|(_, f)| (f.parse::<f64>().unwrap() - v).abs() < 0.002)
}

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let columns = widgets::group("Columns", "");
    columns.add(&default_column_width(ctx));
    columns.add(&preset_list(
        ctx,
        "Preset column widths",
        "Sizes cycled through by “switch preset column width”",
        "preset-column-widths",
    ));
    columns.add(&preset_list(
        ctx,
        "Preset window heights",
        "Sizes cycled through by “switch preset window height”",
        "preset-window-heights",
    ));
    columns.add(&widgets::enum_combo(
        ctx,
        "Center focused column",
        "",
        path(&["layout", "center-focused-column"]),
        &[("Never", "never"), ("Always", "always"), ("When it doesn't fit", "on-overflow")],
    ));
    columns.add(&widgets::switch(
        ctx,
        "Center a lone column",
        "Keep a single column centered on the workspace",
        path(&["layout", "always-center-single-column"]),
    ));
    columns.add(&widgets::enum_combo(
        ctx,
        "New column display",
        "How newly created columns show their windows",
        path(&["layout", "default-column-display"]),
        &[("Normal", "normal"), ("Tabbed", "tabbed")],
    ));
    page.add(&columns);

    let spacing = widgets::group("Spacing", "");
    spacing.add(&widgets::spin(
        ctx,
        "Gaps",
        "Space between windows and around the edges, in logical pixels",
        path(&["layout", "gaps"]),
        NumSpec { min: 0.0, max: 200.0, step: 1.0, digits: 0, float: false, default: 16.0 },
    ));
    let struts = adw::ExpanderRow::builder()
        .title("Struts")
        .subtitle("Extra space reserved at the screen edges, added to the gaps")
        .build();
    for (label, side) in [("Left", "left"), ("Right", "right"), ("Top", "top"), ("Bottom", "bottom")] {
        struts.add_row(&widgets::spin(
            ctx,
            label,
            "",
            path(&["layout", "struts", side]),
            NumSpec { min: -500.0, max: 2000.0, step: 1.0, digits: 0, float: false, default: 0.0 },
        ));
    }
    spacing.add(&struts);
    page.add(&spacing);

    let workspaces = widgets::group("Workspaces", "");
    workspaces.add(&widgets::switch(
        ctx,
        "Empty workspace above first",
        "Always keep an empty workspace at the top, as well as at the bottom",
        path(&["layout", "empty-workspace-above-first"]),
    ));
    page.add(&workspaces);

    page.add(&tabs(ctx));
    page
}

fn default_column_width(ctx: &Ctx) -> adw::ComboRow {
    let p = ["layout", "default-column-width"];
    let (exists, kids) = {
        let s = ctx.store.borrow();
        (s.get(&p).is_some(), s.children_src(&p))
    };
    let mut options: Vec<(String, Option<String>)> = vec![
        ("Default (½)".into(), None),
        ("Let the window decide".into(), Some("default-column-width {}".into())),
    ];
    for (label, f) in FRACTIONS {
        options.push((label.to_string(), Some(format!("default-column-width {{ proportion {f}; }}"))));
    }
    let selected = match kids.as_slice() {
        _ if !exists => 0,
        [] => 1,
        [(n, _)] if n.name == "proportion" => {
            n.arg(0).and_then(|v| v.as_f64()).and_then(fraction_index).map(|i| i + 2).unwrap_or(usize::MAX)
        }
        _ => usize::MAX,
    };
    let selected = if selected == usize::MAX {
        // Something else (e.g. `fixed 800`): keep it as a selectable option.
        let src: Vec<&str> = kids.iter().map(|(_, s)| s.as_str()).collect();
        options.push((src.join(", "), Some(format!("default-column-width {{ {}; }}", src.join("; ")))));
        options.len() - 1
    } else {
        selected
    };
    widgets::node_combo(
        ctx,
        "New column width",
        "Width of newly opened windows",
        path(&p),
        options,
        selected,
    )
}

/// Checkbox list of common proportions for `preset-column-widths` /
/// `preset-window-heights`. Entries it doesn't recognise (e.g. `fixed 800`)
/// are kept as they are.
fn preset_list(ctx: &Ctx, title: &str, subtitle: &str, name: &'static str) -> adw::ExpanderRow {
    let p = ["layout", name];
    let (exists, kids) = {
        let s = ctx.store.borrow();
        (s.get(&p).is_some(), s.children_src(&p))
    };
    let mut checked = vec![false; FRACTIONS.len()];
    let mut extras = Vec::new();
    if exists {
        for (n, src) in &kids {
            match n.arg(0).and_then(|v| v.as_f64()).and_then(fraction_index) {
                Some(i) if n.name == "proportion" => checked[i] = true,
                _ => extras.push(src.clone()),
            }
        }
    } else {
        for d in DEFAULT_PRESETS {
            checked[FRACTIONS.iter().position(|(_, f)| f == d).unwrap()] = true;
        }
    }

    let row = adw::ExpanderRow::builder().title(title).subtitle(subtitle).build();
    let switches: Rc<RefCell<Vec<adw::SwitchRow>>> = Rc::default();
    let guard = Rc::new(Cell::new(false));
    let extras = Rc::new(extras);
    for (i, (label, _)) in FRACTIONS.iter().enumerate() {
        let sw = adw::SwitchRow::builder().title(*label).active(checked[i]).build();
        row.add_row(&sw);
        switches.borrow_mut().push(sw.clone());
        let (ctx, switches, guard, extras) = (ctx.clone(), switches.clone(), guard.clone(), extras.clone());
        sw.connect_active_notify(move |sw| {
            if guard.get() {
                return;
            }
            let on = sw.is_active();
            let mut entries: Vec<String> = switches
                .borrow()
                .iter()
                .zip(FRACTIONS)
                .filter(|(s, _)| s.is_active())
                .map(|(_, (_, f))| format!("proportion {f}"))
                .collect();
            entries.extend(extras.iter().cloned());
            let revert = {
                let (sw, guard) = (sw.clone(), guard.clone());
                move || {
                    guard.set(true);
                    sw.set_active(!on);
                    guard.set(false);
                }
            };
            if entries.is_empty() {
                revert();
                ctx.toast("Keep at least one preset");
                return;
            }
            let text = format!("{name} {{ {}; }}", entries.join("; "));
            ctx.apply(move |s| s.set_node(&["layout", name], Some(text)), move |ok| {
                if !ok {
                    revert();
                }
            });
        });
    }
    for e in extras.iter() {
        row.add_row(&adw::ActionRow::builder().title(glib_escape(e)).subtitle("Kept from your config").build());
    }
    row
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

fn tabs(ctx: &Ctx) -> adw::PreferencesGroup {
    let g = widgets::group("Tabbed columns", "");
    let ind = widgets::block_expander(
        ctx,
        "Tab indicator",
        "Show a bar of tabs beside tabbed columns",
        &["layout", "tab-indicator"],
        true,
    );
    ind.add_row(&widgets::enum_combo(
        ctx,
        "Position",
        "",
        path(&["layout", "tab-indicator", "position"]),
        &[("Left", "left"), ("Right", "right"), ("Top", "top"), ("Bottom", "bottom")],
    ));
    ind.add_row(&widgets::switch(
        ctx,
        "Hide for a single tab",
        "",
        path(&["layout", "tab-indicator", "hide-when-single-tab"]),
    ));
    ind.add_row(&widgets::switch(
        ctx,
        "Place within column",
        "Draw the indicator inside the column instead of in the gap",
        path(&["layout", "tab-indicator", "place-within-column"]),
    ));
    let num = |default: f64, min: f64| NumSpec { min, max: 200.0, step: 1.0, digits: 0, float: false, default };
    ind.add_row(&widgets::spin(ctx, "Width", "", path(&["layout", "tab-indicator", "width"]), num(4.0, 0.0)));
    ind.add_row(&widgets::spin(
        ctx,
        "Gap to window",
        "",
        path(&["layout", "tab-indicator", "gap"]),
        num(5.0, -200.0),
    ));
    ind.add_row(&widgets::spin(
        ctx,
        "Gap between tabs",
        "",
        path(&["layout", "tab-indicator", "gaps-between-tabs"]),
        num(0.0, 0.0),
    ));
    ind.add_row(&widgets::spin(
        ctx,
        "Corner radius",
        "",
        path(&["layout", "tab-indicator", "corner-radius"]),
        num(0.0, 0.0),
    ));
    ind.add_row(&widgets::spin(
        ctx,
        "Length",
        "Fraction of the column the whole bar of tabs spans",
        widgets::Target::Prop(path(&["layout", "tab-indicator", "length"]), "total-proportion"),
        NumSpec { min: 0.05, max: 1.0, step: 0.05, digits: 2, float: true, default: 0.5 },
    ));
    for (label, kind, default) in
        [("Active color", "active", "#7fc8ff"), ("Inactive color", "inactive", "#505050"), ("Urgent color", "urgent", "#9b0000")]
    {
        ind.add_row(&widgets::color(
            ctx,
            label,
            "Defaults to the focus ring color",
            path(&["layout", "tab-indicator", &format!("{kind}-color")]),
            default,
        ));
    }
    g.add(&ind);
    g
}
