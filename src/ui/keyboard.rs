//! Keyboard: layouts, xkb options, repeat, numlock.

use super::widgets::{self, NumSpec};
use super::{Ctx, path};
use crate::kdl::Value;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use crate::config::ConfigStore;

/// Parsed `/usr/share/X11/xkb/rules/evdev.lst`.
#[derive(Default)]
pub struct XkbDb {
    pub layouts: Vec<(String, String)>,          // (layout, description)
    pub variants: Vec<(String, String, String)>, // (layout, variant, description)
    pub options: Vec<(String, String)>,          // (option, description)
}

impl XkbDb {
    pub fn load() -> Self {
        let mut db = XkbDb::default();
        let candidates = [
            "/usr/share/X11/xkb/rules/evdev.lst",
            "/usr/share/xkeyboard-config-2/rules/evdev.lst",
            "/usr/share/X11/xkb/rules/base.lst",
        ];
        let Some(text) = candidates.iter().find_map(|p| std::fs::read_to_string(p).ok()) else {
            return db;
        };
        let mut section = "";
        for line in text.lines() {
            if let Some(s) = line.strip_prefix("! ") {
                section = match s.trim() {
                    "layout" => "layout",
                    "variant" => "variant",
                    "option" => "option",
                    _ => "",
                };
                continue;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (name, desc) = match line.split_once(char::is_whitespace) {
                Some((n, d)) => (n.to_string(), d.trim().to_string()),
                None => continue,
            };
            match section {
                "layout" => db.layouts.push((name, desc)),
                "variant" => {
                    if let Some((layout, d)) = desc.split_once(": ") {
                        db.variants.push((layout.to_string(), name, d.to_string()));
                    }
                }
                "option" => db.options.push((name, desc)),
                _ => {}
            }
        }
        db
    }

    pub fn describe(&self, layout: &str, variant: &str) -> String {
        if !variant.is_empty() {
            if let Some((_, _, d)) = self.variants.iter().find(|(l, v, _)| l == layout && v == variant) {
                return d.clone();
            }
            return format!("{layout} ({variant})");
        }
        self.layouts
            .iter()
            .find(|(l, _)| l == layout)
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| layout.to_string())
    }

    fn option_desc(&self, opt: &str) -> String {
        self.options
            .iter()
            .find(|(o, _)| o == opt)
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| opt.to_string())
    }
}

const XKB: [&str; 3] = ["input", "keyboard", "xkb"];

fn xkb_path(leaf: &str) -> Vec<&str> {
    let mut v = XKB.to_vec();
    v.push(leaf);
    v
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).collect()
}

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    // Parsed once per run: the list is static and takes a while to parse.
    thread_local! {
        static DB: Rc<XkbDb> = Rc::new(XkbDb::load());
    }
    let db = DB.with(Rc::clone);

    // ── Layouts ──
    let layouts_group = widgets::group(
        "Layouts",
        "The first layout is the default. Use the layout switch shortcut below to cycle between them.",
    );
    let uses_file = ctx.store.borrow().string(&xkb_path("file")).is_some();
    if uses_file {
        layouts_group.set_description(Some(
            "Your config loads a custom keymap file (xkb file), so layout settings here are ignored by niri.",
        ));
    }
    let layouts_box = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    layouts_group.add(&layouts_box);
    let rebuild: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let ctx = ctx.clone();
        let db = db.clone();
        let layouts_box = layouts_box.clone();
        let rebuild_ref = rebuild.clone();
        *rebuild.borrow_mut() = Some(Box::new(move || {
            layouts_box.remove_all();
            let list = current_layouts(&ctx);
            let n = list.len();
            for (i, (layout, variant)) in list.iter().enumerate() {
                let row = adw::ActionRow::builder()
                    .title(glib_escape(&db.describe(layout, variant)))
                    .subtitle(if variant.is_empty() { layout.clone() } else { format!("{layout} · {variant}") })
                    .build();
                if i > 0 {
                    let up = icon_button("go-up-symbolic", "Move up");
                    let ctx2 = ctx.clone();
                    let rb = rebuild_ref.clone();
                    up.connect_clicked(move |_| {
                        let rb = rb.clone();
                        edit_layouts(&ctx2, move |l| if i < l.len() { l.swap(i, i - 1) }, move || {
                            if let Some(f) = rb.borrow().as_ref() { f() }
                        });
                    });
                    row.add_suffix(&up);
                }
                if n > 1 {
                    let rm = icon_button("user-trash-symbolic", "Remove layout");
                    let ctx2 = ctx.clone();
                    let rb = rebuild_ref.clone();
                    rm.connect_clicked(move |_| {
                        let rb = rb.clone();
                        edit_layouts(&ctx2, move |l| if i < l.len() { l.remove(i); }, move || {
                            if let Some(f) = rb.borrow().as_ref() { f() }
                        });
                    });
                    row.add_suffix(&rm);
                }
                layouts_box.append(&row);
            }
            if list.is_empty() {
                let row = adw::ActionRow::builder()
                    .title("System default")
                    .subtitle("No layout set; niri uses the XKB defaults (usually US English)")
                    .build();
                layouts_box.append(&row);
            }
            // Inline search over every layout and variant.
            let (choices, items) = layout_choices(&db);
            let ctx2 = ctx.clone();
            let rb = rebuild_ref.clone();
            let on_pick = Rc::new(move |i: usize| {
                let (layout, variant) = choices[i].clone();
                let rb = rb.clone();
                edit_layouts(
                    &ctx2,
                    move |l| {
                        if !l.iter().any(|(a, b)| *a == layout && *b == variant) {
                            l.push((layout, variant));
                        }
                    },
                    move || {
                        if let Some(f) = rb.borrow().as_ref() { f() }
                    },
                );
            });
            let add = widgets::search_picker("Add Layout…", "list-add-symbolic", items, on_pick);
            layouts_box.append(&add);
        }));
    }
    if let Some(f) = rebuild.borrow().as_ref() {
        f();
    }
    page.add(&layouts_group);

    // ── Special keys (xkb options) ──
    let opts_group = widgets::group("Special Keys", "");
    let caps: Vec<String> = db
        .options
        .iter()
        .map(|(o, _)| o.clone())
        .filter(|o| o.starts_with("caps:") || o == "ctrl:nocaps" || o == "ctrl:swapcaps")
        .collect();
    let compose: Vec<String> = db.options.iter().map(|(o, _)| o.clone()).filter(|o| o.starts_with("compose:")).collect();
    let grp: Vec<String> = db.options.iter().map(|(o, _)| o.clone()).filter(|o| o.starts_with("grp:")).collect();
    let other = adw::EntryRow::builder().title("Other XKB options (comma-separated)").show_apply_button(true).build();

    let families: Arc<Vec<Vec<String>>> = Arc::new(vec![caps, compose, grp]);
    let titles = [
        ("Caps Lock behavior", "Default"),
        ("Compose key", "None"),
        ("Switch layout shortcut", "None"),
    ];
    let current_opts = current_options(ctx);
    for (fi, (title, none_label)) in titles.iter().enumerate() {
        let fam = &families[fi];
        let mut labels = vec![none_label.to_string()];
        labels.extend(fam.iter().map(|o| db.option_desc(o)));
        let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let row = adw::ComboRow::builder().title(*title).model(&model).build();
        widgets::unclip(&row);
        row.set_enable_search(true);
        row.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
        row.set_expression(Some(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<gtk::Expression>,
            "string",
        )));
        let sel = current_opts.iter().find_map(|o| fam.iter().position(|f| f == o)).map(|i| i + 1).unwrap_or(0);
        row.set_selected(sel as u32);
        let ctx2 = ctx.clone();
        let families = families.clone();
        row.connect_selected_notify(move |row| {
            let families = families.clone();
            let sel = row.selected() as usize;
            edit_options(&ctx2, move |opts| {
                let fam = &families[fi];
                opts.retain(|o| !fam.contains(o));
                if let Some(o) = sel.checked_sub(1).and_then(|i| fam.get(i)) {
                    opts.push(o.clone());
                }
            });
        });
        opts_group.add(&row);
    }
    let known = |o: &String, fams: &Vec<Vec<String>>| fams.iter().any(|f| f.contains(o));
    other.set_text(
        &current_opts.iter().filter(|o| !known(o, &families)).cloned().collect::<Vec<_>>().join(","),
    );
    {
        let ctx2 = ctx.clone();
        let families = families.clone();
        other.connect_apply(move |row| {
            let families = families.clone();
            let extra: Vec<String> =
                row.text().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            edit_options(&ctx2, move |opts| {
                opts.retain(|o| known(o, &families));
                opts.extend(extra);
            });
        });
    }
    opts_group.add(&other);
    page.add(&opts_group);

    // ── Typing ──
    let typing = widgets::group("Typing", "");
    typing.add(&widgets::spin(
        ctx,
        "Repeat delay (ms)",
        "How long to hold a key before it starts repeating",
        path(&["input", "keyboard", "repeat-delay"]),
        NumSpec { min: 100.0, max: 2000.0, step: 25.0, digits: 0, float: false, default: 600.0 },
    ));
    typing.add(&widgets::spin(
        ctx,
        "Repeat rate (per second)",
        "How fast a held key repeats",
        path(&["input", "keyboard", "repeat-rate"]),
        NumSpec { min: 1.0, max: 100.0, step: 1.0, digits: 0, float: false, default: 25.0 },
    ));
    typing.add(&widgets::switch(
        ctx,
        "Num Lock on startup",
        "",
        path(&["input", "keyboard", "numlock"]),
    ));
    typing.add(&widgets::combo(
        ctx,
        "Remember layout",
        "Whether the active layout is shared or tracked per window",
        path(&["input", "keyboard", "track-layout"]),
        &[("Default (global)", None), ("Global", Some("global")), ("Per window", Some("window"))],
    ));
    page.add(&typing);

    page
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build()
}

fn layouts_of(s: &ConfigStore) -> Vec<(String, String)> {
    let layouts = s.string(&xkb_path("layout")).unwrap_or_default();
    if layouts.trim().is_empty() {
        return Vec::new();
    }
    let variants = s.string(&xkb_path("variant")).unwrap_or_default();
    let vs = split_list(&variants);
    split_list(&layouts)
        .into_iter()
        .enumerate()
        .map(|(i, l)| (l, vs.get(i).cloned().unwrap_or_default()))
        .collect()
}

fn current_layouts(ctx: &Ctx) -> Vec<(String, String)> {
    layouts_of(&ctx.store.borrow())
}

/// Read-modify-write of the layout list, done on the worker so quick
/// successive clicks always see the latest list.
fn edit_layouts(
    ctx: &Ctx,
    f: impl FnOnce(&mut Vec<(String, String)>) + Send + 'static,
    done: impl FnOnce() + 'static,
) {
    ctx.apply(
        move |s| {
            s.reload()?;
            let mut list = layouts_of(s);
            f(&mut list);
            let layout = list.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(",");
            let variant = list.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(",");
            let has_variant = list.iter().any(|(_, v)| !v.is_empty());
            let layout_node = (!layout.is_empty()).then(|| format!("layout {}", Value::String(layout).to_kdl()));
            let variant_node = has_variant.then(|| format!("variant {}", Value::String(variant).to_kdl()));
            s.set_nodes(&[(&xkb_path("layout"), layout_node), (&xkb_path("variant"), variant_node)])
        },
        move |_| done(),
    );
}

fn options_of(s: &ConfigStore) -> Vec<String> {
    s.string(&xkb_path("options"))
        .map(|o| split_list(&o).into_iter().filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

fn current_options(ctx: &Ctx) -> Vec<String> {
    options_of(&ctx.store.borrow())
}

fn edit_options(ctx: &Ctx, f: impl FnOnce(&mut Vec<String>) + Send + 'static) {
    ctx.apply_quiet(move |s| {
        s.reload()?;
        let mut opts = options_of(s);
        f(&mut opts);
        let joined = opts.join(",");
        s.set_value(&xkb_path("options"), (!joined.is_empty()).then(|| Value::String(joined)))
    });
}

/// Every layout and variant as (layout, variant) plus picker entries,
/// sorted by description.
fn layout_choices(db: &XkbDb) -> (Vec<(String, String)>, Rc<Vec<widgets::PickItem>>) {
    let mut entries: Vec<(String, String, String)> = Vec::new();
    for (l, d) in &db.layouts {
        entries.push((l.clone(), String::new(), d.clone()));
    }
    for (l, v, d) in &db.variants {
        entries.push((l.clone(), v.clone(), d.clone()));
    }
    entries.sort_by_cached_key(|e| e.2.to_lowercase());
    let items = entries
        .iter()
        .map(|(l, v, d)| widgets::PickItem {
            title: d.clone(),
            subtitle: if v.is_empty() { l.clone() } else { format!("{l} · {v}") },
            icon: None,
            hay: format!("{d} {l} {v}").to_lowercase(),
        })
        .collect();
    (entries.into_iter().map(|(l, v, _)| (l, v)).collect(), Rc::new(items))
}
