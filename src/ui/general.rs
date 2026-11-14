//! Focus behaviour, modifier key, desktop toggles, cursor theme.

use super::widgets::{self, NumSpec};
use super::{Ctx, path, refs};
use crate::config::home;
use crate::kdl::{self, Value};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    let focus = widgets::group("Focus", "");
    focus.add(&focus_follows_mouse(ctx));
    focus.add(&widgets::switch(
        ctx,
        "Warp mouse to focus",
        "Move the pointer to newly focused windows",
        path(&["input", "warp-mouse-to-focus"]),
    ));
    focus.add(&widgets::switch(
        ctx,
        "Workspace back and forth",
        "Switching to the current workspace goes to the previous one",
        path(&["input", "workspace-auto-back-and-forth"]),
    ));
    page.add(&focus);

    let keys = widgets::group("Keys", "");
    let mods: &[(&str, Option<&str>)] = &[
        ("Default (Super)", None),
        ("Super", Some("Super")),
        ("Alt", Some("Alt")),
        ("Ctrl", Some("Ctrl")),
        ("Shift", Some("Shift")),
        ("Mod3", Some("Mod3")),
        ("Mod5 (AltGr)", Some("Mod5")),
        ("ISO Level 3 Shift", Some("ISO_Level3_Shift")),
        ("ISO Level 5 Shift", Some("ISO_Level5_Shift")),
    ];
    keys.add(&widgets::combo(
        ctx,
        "Mod key",
        "The key that “Mod” refers to in shortcuts",
        path(&["input", "mod-key"]),
        mods,
    ));
    keys.add(&widgets::switch(
        ctx,
        "Ignore power button",
        "Let another program (e.g. logind) handle the power key",
        path(&["input", "disable-power-key-handling"]),
    ));
    page.add(&keys);

    let desktop = widgets::group("Desktop", "");
    desktop.add(&widgets::off_switch(
        ctx,
        "Hot corner",
        "Open the overview by pushing the pointer into the top-left corner",
        path(&["gestures", "hot-corners", "off"]),
    ));
    desktop.add(&widgets::off_switch(
        ctx,
        "Middle-click paste",
        "Keep the primary selection (select text, paste with middle click)",
        path(&["clipboard", "disable-primary"]),
    ));
    page.add(&desktop);

    let cursor = widgets::group("Cursor", "");
    cursor.add(&cursor_theme(ctx));
    cursor.add(&widgets::spin(
        ctx,
        "Size",
        "",
        path(&["cursor", "xcursor-size"]),
        NumSpec { min: 8.0, max: 256.0, step: 1.0, digits: 0, float: false, default: 24.0 },
    ));
    cursor.add(&widgets::switch(
        ctx,
        "Hide while typing",
        "",
        path(&["cursor", "hide-when-typing"]),
    ));
    cursor.add(&widgets::spin(
        ctx,
        "Hide after inactivity (ms)",
        "0 keeps the cursor visible",
        path(&["cursor", "hide-after-inactive-ms"]),
        NumSpec { min: 0.0, max: 600_000.0, step: 500.0, digits: 0, float: false, default: 0.0 },
    ));
    page.add(&cursor);

    page
}

/// `focus-follows-mouse` with its optional `max-scroll-amount="N%"` property.
fn focus_follows_mouse(ctx: &Ctx) -> adw::ExpanderRow {
    let p = path(&["input", "focus-follows-mouse"]);
    let (enabled, max_scroll) = {
        let s = ctx.store.borrow();
        let n = s.get(&refs(&p));
        let amount = n
            .and_then(|n| n.prop("max-scroll-amount"))
            .and_then(Value::as_str)
            .and_then(|s| s.trim_end_matches('%').parse::<f64>().ok());
        (n.is_some(), amount)
    };
    let row = adw::ExpanderRow::builder()
        .title("Focus follows mouse")
        .subtitle("Focus windows when the pointer moves over them")
        .show_enable_switch(true)
        .enable_expansion(enabled)
        .build();

    let limit = adw::SpinRow::with_range(0.0, 100.0, 5.0);
    limit.set_title("Max scroll amount (%)");
    limit.set_subtitle("Don't focus windows that would scroll the view more than this; 100 = no limit");
    limit.set_value(max_scroll.unwrap_or(100.0));
    row.add_row(&limit);

    let write = {
        let ctx = ctx.clone();
        let p = p.clone();
        let limit = limit.clone();
        move |on: bool, done: Box<dyn FnOnce(bool)>| {
            let text = on.then(|| {
                let v = limit.value().round();
                if v >= 100.0 {
                    "focus-follows-mouse".to_string()
                } else {
                    format!("focus-follows-mouse max-scroll-amount={}", kdl::quote(&format!("{v}%")))
                }
            });
            let p = p.clone();
            ctx.apply(move |s| s.set_node(&refs(&p), text), done);
        }
    };

    let guard = Rc::new(Cell::new(false));
    {
        let write = write.clone();
        let guard = guard.clone();
        row.connect_enable_expansion_notify(move |row| {
            if guard.get() {
                return;
            }
            let on = row.enables_expansion();
            let (row, guard) = (row.clone(), guard.clone());
            write(
                on,
                Box::new(move |ok| {
                    if !ok {
                        guard.set(true);
                        row.set_enable_expansion(!on);
                        guard.set(false);
                    }
                }),
            );
        });
    }
    let deb = widgets::Debouncer::default();
    let row2 = row.clone();
    limit.connect_value_notify(move |_| {
        if !row2.enables_expansion() {
            return;
        }
        let write = write.clone();
        deb.run(400, move || write(true, Box::new(|_| {})));
    });
    row
}

fn cursor_themes() -> Vec<String> {
    let mut dirs = vec![home().join(".icons"), home().join(".local/share/icons")];
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':').filter(|d| !d.is_empty()) {
        dirs.push(std::path::Path::new(d).join("icons"));
    }
    let mut themes: Vec<String> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|e| e.path().join("cursors").is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    themes.sort_by_key(|t| t.to_lowercase());
    themes.dedup();
    themes
}

fn cursor_theme(ctx: &Ctx) -> adw::ComboRow {
    let themes = cursor_themes();
    let mut options: Vec<(&str, Option<&str>)> = vec![("Default", None)];
    options.extend(themes.iter().map(|t| (t.as_str(), Some(t.as_str()))));
    let row = widgets::combo(ctx, "Theme", "", path(&["cursor", "xcursor-theme"]), &options);
    row.set_enable_search(true);
    row.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    row.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
    row
}
