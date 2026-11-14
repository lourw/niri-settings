//! Window rules: `window-rule { match …; open-floating true; … }`.
//!
//! The dialog edits the common properties; anything else in a rule (other
//! match criteria, nested blocks, less common properties) is kept verbatim.

use super::{Ctx, widgets};
use crate::config::Item;
use crate::kdl::{self, Node, Value};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const NAME: &str = "window-rule";

/// Yes/no properties shown as "Don't change / Yes / No".
const BOOLS: &[(&str, &str, &str)] = &[
    ("open-floating", "Open floating", ""),
    ("open-maximized", "Open maximized", "As a full-width column"),
    ("open-maximized-to-edges", "Open maximized to edges", "Fill the whole screen, without gaps"),
    ("open-fullscreen", "Open fullscreen", ""),
    ("open-focused", "Focus when opened", ""),
    ("clip-to-geometry", "Clip to geometry", "Cut the window to its rounded corners"),
    ("draw-border-with-background", "Draw border with background", "Fill the border behind the window"),
];
const STRINGS: &[(&str, &str)] = &[("open-on-workspace", "Open on workspace"), ("open-on-output", "Open on display")];
const SIZES: &[(&str, &str)] = &[("default-column-width", "Default width"), ("default-window-height", "Default height")];

#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub exclude: bool,
    pub app_id: String,
    pub title: String,
    /// Other criteria (`is-floating=true` …) as KDL property text.
    pub extra: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Proportion(f64),
    Fixed(i64),
    /// `{}`: let the window decide.
    Auto,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rule {
    pub conds: Vec<Cond>,
    pub bools: Vec<(String, bool)>,
    pub strings: Vec<(String, String)>,
    pub sizes: Vec<(String, Size)>,
    pub opacity: Option<f64>,
    pub radius: Option<f64>,
    pub block_out: Option<String>,
    /// Unrecognised children, as source text.
    pub other: Vec<String>,
}

fn arg_str(n: &Node) -> Option<String> {
    n.arg(0).and_then(Value::as_str).map(String::from)
}

impl Rule {
    pub fn parse(text: &str, node: &Node) -> Rule {
        let mut r = Rule::default();
        for c in node.children() {
            let simple = c.children.is_none();
            let name = c.name.as_str();
            let nargs = c.args().count();
            let nprops = c.entries.len() - nargs;
            let ok = match name {
                "match" | "exclude" if simple && nargs == 0 => {
                    let mut cond = Cond { exclude: name == "exclude", app_id: String::new(), title: String::new(), extra: vec![] };
                    for e in &c.entries {
                        let k = e.key.as_deref().unwrap_or_default();
                        match (k, &e.value) {
                            ("app-id", Value::String(s)) if cond.app_id.is_empty() => cond.app_id = s.clone(),
                            ("title", Value::String(s)) if cond.title.is_empty() => cond.title = s.clone(),
                            _ => cond.extra.push(format!("{}={}", kdl::ident(k), e.value.to_kdl())),
                        }
                    }
                    r.conds.push(cond);
                    true
                }
                n if simple && nprops == 0 && BOOLS.iter().any(|b| b.0 == n) => match nargs {
                    0 => {
                        r.bools.push((n.into(), true));
                        true
                    }
                    1 => c.arg(0).and_then(Value::as_bool).map(|b| r.bools.push((n.into(), b))).is_some(),
                    _ => false,
                },
                n if simple && nprops == 0 && nargs == 1 && STRINGS.iter().any(|s| s.0 == n) => {
                    arg_str(c).map(|s| r.strings.push((n.into(), s))).is_some()
                }
                n if SIZES.iter().any(|s| s.0 == n) && c.entries.is_empty() => {
                    let kids = c.children();
                    let size = match kids {
                        [] => Some(Size::Auto),
                        [k] if k.name == "proportion" => k.arg(0).and_then(Value::as_f64).map(Size::Proportion),
                        [k] if k.name == "fixed" => k.arg(0).and_then(Value::as_f64).map(|v| Size::Fixed(v as i64)),
                        _ => None,
                    };
                    size.map(|s| r.sizes.push((n.into(), s))).is_some()
                }
                "opacity" if simple && nargs == 1 && nprops == 0 => {
                    c.arg(0).and_then(Value::as_f64).map(|v| r.opacity = Some(v)).is_some()
                }
                "geometry-corner-radius" if simple && nargs == 1 && nprops == 0 => {
                    c.arg(0).and_then(Value::as_f64).map(|v| r.radius = Some(v)).is_some()
                }
                "block-out-from" if simple && nargs == 1 && nprops == 0 => {
                    arg_str(c).map(|v| r.block_out = Some(v)).is_some()
                }
                _ => false,
            };
            if !ok {
                r.other.push(text[c.span()].to_string());
            }
        }
        r
    }

    pub fn get_bool(&self, k: &str) -> Option<bool> {
        self.bools.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }
    pub fn get_string(&self, k: &str) -> Option<&str> {
        self.strings.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    pub fn get_size(&self, k: &str) -> Option<Size> {
        self.sizes.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }

    /// Node text. Lines after the first are prefixed with `indent`, the
    /// indentation of the line the rule starts on.
    pub fn to_kdl(&self, indent: &str) -> String {
        let mut lines: Vec<String> = Vec::new();
        for c in &self.conds {
            let mut l = String::from(if c.exclude { "exclude" } else { "match" });
            if !c.app_id.is_empty() {
                l.push_str(&format!(" app-id={}", kdl::regex_lit(&c.app_id)));
            }
            if !c.title.is_empty() {
                l.push_str(&format!(" title={}", kdl::regex_lit(&c.title)));
            }
            for e in &c.extra {
                l.push(' ');
                l.push_str(e);
            }
            if l != "match" && l != "exclude" {
                lines.push(l);
            }
        }
        for (k, _, _) in BOOLS {
            if let Some(v) = self.get_bool(k) {
                lines.push(format!("{k} {v}"));
            }
        }
        for (k, _) in STRINGS {
            if let Some(v) = self.get_string(k) {
                lines.push(format!("{k} {}", kdl::quote(v)));
            }
        }
        for (k, _) in SIZES {
            match self.get_size(k) {
                Some(Size::Proportion(p)) => lines.push(format!("{k} {{ proportion {}; }}", kdl::format_float(p))),
                Some(Size::Fixed(px)) => lines.push(format!("{k} {{ fixed {px}; }}")),
                Some(Size::Auto) => lines.push(format!("{k} {{}}")),
                None => {}
            }
        }
        if let Some(o) = self.opacity {
            lines.push(format!("opacity {}", kdl::format_float(o)));
        }
        if let Some(r) = self.radius {
            lines.push(format!("geometry-corner-radius {}", kdl::format_float(r)));
        }
        if let Some(b) = &self.block_out {
            lines.push(format!("block-out-from {}", kdl::quote(b)));
        }
        let mut out = format!("{NAME} {{\n");
        for l in &lines {
            out.push_str(&format!("{indent}    {l}\n"));
        }
        // Verbatim children keep their own continuation-line indentation.
        for o in &self.other {
            out.push_str(&format!("{indent}    {o}\n"));
        }
        out.push_str(indent);
        out.push('}');
        out
    }

    pub fn title(&self) -> String {
        let parts: Vec<String> = self
            .conds
            .iter()
            .filter(|c| !c.exclude)
            .map(|c| match (c.app_id.is_empty(), c.title.is_empty()) {
                (false, true) => c.app_id.clone(),
                (true, false) => format!("Title: {}", c.title),
                (false, false) => format!("{} — {}", c.app_id, c.title),
                (true, true) => c.extra.join(" "),
            })
            .collect();
        if parts.is_empty() { "All windows".into() } else { parts.join(", ") }
    }

    pub fn summary(&self) -> String {
        let mut s: Vec<String> = Vec::new();
        for (k, label, _) in BOOLS {
            match self.get_bool(k) {
                Some(true) => s.push(label.to_string()),
                Some(false) => s.push(format!("{label}: no")),
                None => {}
            }
        }
        for (k, label) in STRINGS {
            if let Some(v) = self.get_string(k) {
                s.push(format!("{label} {v}"));
            }
        }
        for (k, label) in SIZES {
            match self.get_size(k) {
                Some(Size::Proportion(p)) => s.push(format!("{label} {:.0}%", p * 100.0)),
                Some(Size::Fixed(px)) => s.push(format!("{label} {px}px")),
                Some(Size::Auto) => s.push(format!("{label}: app decides")),
                None => {}
            }
        }
        if let Some(o) = self.opacity {
            s.push(format!("Opacity {:.0}%", o * 100.0));
        }
        if let Some(r) = self.radius {
            s.push(format!("Corner radius {r}"));
        }
        if let Some(b) = &self.block_out {
            s.push(format!("Hidden from {b}"));
        }
        for o in &self.other {
            s.push(o.split([' ', '{', '\n']).next().unwrap_or("").to_string());
        }
        let ex = self.conds.iter().filter(|c| c.exclude).count();
        if ex > 0 {
            s.push(format!("{ex} exclusion{}", if ex == 1 { "" } else { "s" }));
        }
        s.join(" · ")
    }
}

/// Parse a rule from its own source text (spans then index into it).
fn parse_item(item: &Item) -> Rule {
    match kdl::Document::parse(item.src.clone()) {
        Ok(doc) if !doc.nodes.is_empty() => Rule::parse(&doc.text, &doc.nodes[0]),
        _ => Rule { other: vec![], ..Default::default() },
    }
}

/// Anchored, escaped regex matching exactly `s`.
fn exact_regex(s: &str) -> String {
    let mut out = String::from("^");
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('$');
    out
}

/// (app-id, title) of open windows, from `niri msg -j windows`.
fn open_windows() -> Vec<(String, String)> {
    let Ok(out) = std::process::Command::new("niri").args(["msg", "-j", "windows"]).output() else { return vec![] };
    let Ok(serde_json::Value::Array(ws)) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return vec![];
    };
    let mut v: Vec<(String, String)> = ws
        .iter()
        .map(|w| {
            let s = |k: &str| w.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            (s("app_id"), s("title"))
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

struct State {
    ctx: Ctx,
    page: adw::PreferencesPage,
    group: RefCell<Option<adw::PreferencesGroup>>,
    /// Rule index to show expanded after the next rebuild.
    reopen: Cell<Option<usize>>,
}

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let st = Rc::new(State { ctx: ctx.clone(), page: page.clone(), group: RefCell::new(None), reopen: Cell::new(None) });
    st.rebuild();
    page
}

impl State {
    fn rebuild(self: &Rc<Self>) {
        if let Some(g) = self.group.borrow_mut().take() {
            self.page.remove(&g);
        }
        let g = widgets::group(
            "Window rules",
            "Settings for windows that match. Rules apply in order, and later rules override earlier ones.",
        );
        let reopen = self.reopen.take();
        let items = self.ctx.store.borrow().items(NAME);
        for (i, item) in items.into_iter().enumerate() {
            g.add(&self.rule_row(Some(item), reopen == Some(i)));
        }
        let add = adw::ButtonRow::builder().title("Add Rule…").start_icon_name("list-add-symbolic").build();
        {
            let (st, g2) = (self.clone(), g.clone());
            add.connect_activated(move |add| {
                // An unsaved rule, opened for editing, above the Add button.
                let row = st.rule_row(None, true);
                g2.remove(add);
                g2.add(&row);
                g2.add(add);
            });
        }
        g.add(&add);
        self.page.add(&g);
        *self.group.borrow_mut() = Some(g);
    }

    /// A rule as an expander; its form is built on first expand.
    fn rule_row(self: &Rc<Self>, existing: Option<Item>, expanded: bool) -> adw::ExpanderRow {
        let rule = existing.as_ref().map(parse_item).unwrap_or_default();
        let (title, summary) = if existing.is_some() {
            (rule.title(), rule.summary())
        } else {
            ("New rule".to_string(), "Not saved yet".to_string())
        };
        let row = adw::ExpanderRow::builder()
            .title(glib::markup_escape_text(&title))
            .subtitle(glib::markup_escape_text(&summary))
            .subtitle_lines(2)
            .build();
        let st = self.clone();
        widgets::lazy_rows(&row, move |row| st.fill_form(row, existing, rule));
        row.set_expanded(expanded);
        row
    }

    fn fill_form(self: &Rc<Self>, row: &adw::ExpanderRow, existing: Option<Item>, rule: Rule) {
        let save = gtk::Button::builder().label("Save").css_classes(["suggested-action"]).valign(gtk::Align::Center).build();
        let discard = gtk::Button::builder().label("Discard").valign(gtk::Align::Center).build();
        let is_new = existing.is_none();
        save.set_sensitive(is_new);
        discard.set_sensitive(is_new);
        // Any edit enables Save/Discard.
        let dirty: Rc<dyn Fn()> = {
            let (save, discard) = (save.clone(), discard.clone());
            Rc::new(move || {
                save.set_sensitive(true);
                discard.set_sensitive(true);
            })
        };

        // ── Conditions ──
        let cond_rows: Rc<RefCell<Vec<CondRow>>> = Rc::default();
        let conds_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = gtk::Label::builder()
            .label("Applies to windows matching these regular expressions; with none, to every window")
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .css_classes(["dim-label"])
            .build();
        let pick = gtk::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add condition")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        header.append(&label);
        header.append(&pick);
        conds_box.append(&header);
        let add_cond = {
            let (cond_rows, conds_box, dirty) = (cond_rows.clone(), conds_box.clone(), dirty.clone());
            Rc::new(move |c: Cond, user: bool| {
                let r = CondRow::new(&c);
                conds_box.append(&r.list);
                let (cond_rows2, conds_box2, list, d) = (cond_rows.clone(), conds_box.clone(), r.list.clone(), dirty.clone());
                r.remove.connect_clicked(move |_| {
                    conds_box2.remove(&list);
                    cond_rows2.borrow_mut().retain(|r| r.list != list);
                    d();
                });
                let d = dirty.clone();
                r.kind.connect_selected_notify(move |_| d());
                for e in [&r.app_id, &r.title] {
                    let d = dirty.clone();
                    e.connect_changed(move |_| d());
                }
                cond_rows.borrow_mut().push(r);
                if user {
                    dirty();
                }
            })
        };
        for c in &rule.conds {
            add_cond(c.clone(), false);
        }
        {
            let add_cond = add_cond.clone();
            pick.set_popover(Some(&cond_popover(&pick, Rc::new(move |c| add_cond(c, true)))));
        }
        row.add_row(&conds_box);

        // ── Properties ──
        let mut bool_rows = Vec::new();
        for (k, label, sub) in BOOLS.iter().filter(|b| b.0.starts_with("open-")) {
            let r = tri_row(label, sub, rule.get_bool(k));
            row.add_row(&r);
            bool_rows.push((k.to_string(), r));
        }
        let mut string_rows = Vec::new();
        for (k, label) in STRINGS {
            let r = adw::EntryRow::builder().title(*label).text(rule.get_string(k).unwrap_or("")).build();
            row.add_row(&r);
            string_rows.push((k.to_string(), r));
        }
        let mut size_rows = Vec::new();
        for (k, label) in SIZES {
            let (combo, spin) = size_rows_for(label, rule.get_size(k));
            row.add_row(&combo);
            row.add_row(&spin);
            size_rows.push((k.to_string(), combo, spin));
        }
        let opacity = adw::SpinRow::with_range(0.0, 100.0, 5.0);
        opacity.set_title("Opacity");
        opacity.set_subtitle("Percent; 100 leaves the window as it is");
        opacity.set_value(rule.opacity.map(|o| o * 100.0).unwrap_or(100.0));
        row.add_row(&opacity);
        let radius = adw::SpinRow::with_range(0.0, 100.0, 1.0);
        radius.set_title("Corner radius");
        radius.set_subtitle("For borders, focus ring and shadow; 0 leaves it unset");
        radius.set_value(rule.radius.unwrap_or(0.0));
        row.add_row(&radius);
        for (k, label, sub) in BOOLS.iter().filter(|b| !b.0.starts_with("open-")) {
            let r = tri_row(label, sub, rule.get_bool(k));
            row.add_row(&r);
            bool_rows.push((k.to_string(), r));
        }
        let block_opts = [("Don't hide", None), ("Screencasts", Some("screencast")), ("All screen capture", Some("screen-capture"))];
        let block = adw::ComboRow::builder()
            .title("Hide from")
            .subtitle("Show a black box instead when the screen is shared or captured")
            .model(&gtk::StringList::new(&block_opts.iter().map(|o| o.0).collect::<Vec<_>>()))
            .build();
        widgets::unclip(&block);
        block.set_selected(block_opts.iter().position(|o| o.1 == rule.block_out.as_deref()).unwrap_or(0) as u32);
        row.add_row(&block);
        for o in &rule.other {
            let first = o.lines().next().unwrap_or("");
            let more = if o.lines().count() > 1 { " …" } else { "" };
            let r = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&format!("{first}{more}")))
                .subtitle("Kept as written; edit it in the config file")
                .css_classes(["monospace"])
                .build();
            r.set_tooltip_text(Some(o));
            row.add_row(&r);
        }

        // Mark dirty on any change.
        for (_, r) in &bool_rows {
            let d = dirty.clone();
            r.connect_selected_notify(move |_| d());
        }
        for (_, r) in &string_rows {
            let d = dirty.clone();
            r.connect_changed(move |_| d());
        }
        for (_, c, sp) in &size_rows {
            let d = dirty.clone();
            c.connect_selected_notify(move |_| d());
            let d = dirty.clone();
            sp.connect_value_notify(move |_| d());
        }
        for sp in [&opacity, &radius] {
            let d = dirty.clone();
            sp.connect_value_notify(move |_| d());
        }
        {
            let d = dirty.clone();
            block.connect_selected_notify(move |_| d());
        }

        // ── Actions ──
        let actions = gtk::Box::builder()
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let remove = gtk::Button::builder()
            .label(if is_new { "Cancel" } else { "Remove Rule" })
            .css_classes(["destructive-action"])
            .hexpand(true)
            .halign(gtk::Align::Start)
            .build();
        actions.append(&remove);
        if !is_new {
            actions.append(&discard);
        }
        actions.append(&save);
        row.add_row(&actions);

        {
            let (st, existing, row) = (self.clone(), existing.clone(), row.clone());
            remove.connect_clicked(move |_| match existing.clone() {
                None => {
                    if let Some(g) = st.group.borrow().as_ref() {
                        g.remove(&row);
                    }
                }
                Some(item) => {
                    let st2 = st.clone();
                    st.ctx.apply(move |s| s.edit_item(&item, None), move |ok| {
                        if ok {
                            st2.rebuild();
                        }
                    });
                }
            });
        }
        {
            // Discard: rebuild the list, keeping this rule open with saved values.
            let (st, existing) = (self.clone(), existing.clone());
            discard.connect_clicked(move |_| {
                if let Some(item) = &existing {
                    st.reopen.set(Some(st.index_of(item)));
                }
                st.rebuild();
            });
        }

        let st = self.clone();
        let other = rule.other.clone();
        save.connect_clicked(move |_| {
            let mut r = Rule { other: other.clone(), ..Default::default() };
            r.conds = cond_rows
                .borrow()
                .iter()
                .map(CondRow::read)
                .filter(|c| c.app_id.len() + c.title.len() + c.extra.len() > 0)
                .collect();
            for (k, row) in &bool_rows {
                match row.selected() {
                    1 => r.bools.push((k.clone(), true)),
                    2 => r.bools.push((k.clone(), false)),
                    _ => {}
                }
            }
            for (k, row) in &string_rows {
                let t = row.text().trim().to_string();
                if !t.is_empty() {
                    r.strings.push((k.clone(), t));
                }
            }
            for (k, combo, spin) in &size_rows {
                match combo.selected() {
                    1 => r.sizes.push((k.clone(), Size::Proportion(spin.value() / 100.0))),
                    2 => r.sizes.push((k.clone(), Size::Fixed(spin.value() as i64))),
                    3 => r.sizes.push((k.clone(), Size::Auto)),
                    _ => {}
                }
            }
            r.opacity = (opacity.value() < 100.0).then(|| (opacity.value()).round() / 100.0);
            r.radius = (radius.value() > 0.0).then(|| radius.value());
            r.block_out = block_opts[block.selected() as usize].1.map(String::from);

            let existing = existing.clone();
            let indent = existing.as_ref().map(|i| i.indent.clone()).unwrap_or_default();
            let text = r.to_kdl(&indent);
            // Keep the saved rule open after the list is rebuilt.
            let reopen = match &existing {
                Some(item) => st.index_of(item),
                None => st.ctx.store.borrow().items(NAME).len(),
            };
            let st2 = st.clone();
            let done = move |ok: bool| {
                if ok {
                    st2.reopen.set(Some(reopen));
                    st2.rebuild();
                }
            };
            match existing {
                Some(item) => st.ctx.apply(move |s| s.edit_item(&item, Some(text)), done),
                // add_item indents every line itself.
                None => st.ctx.apply(move |s| s.add_item(&text), done),
            }
        });
    }

    /// Position of `item` among all rules (for reopening it after a rebuild).
    fn index_of(&self, item: &Item) -> usize {
        self.ctx
            .store
            .borrow()
            .items(NAME)
            .iter()
            .position(|i| i.file == item.file && i.index == item.index)
            .unwrap_or(usize::MAX)
    }
}

/// One match/exclude condition in a rule's form.
struct CondRow {
    list: gtk::ListBox,
    kind: adw::ComboRow,
    app_id: adw::EntryRow,
    title: adw::EntryRow,
    remove: gtk::Button,
    extra: Vec<String>,
}

impl CondRow {
    fn new(c: &Cond) -> Self {
        let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
        let kind = adw::ComboRow::builder()
            .title("Condition")
            .model(&gtk::StringList::new(&["Match", "Exclude"]))
            .build();
        widgets::unclip(&kind);
        kind.set_selected(c.exclude as u32);
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Remove condition")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        kind.add_suffix(&remove);
        let app_id = adw::EntryRow::builder().title("App ID").text(&c.app_id).build();
        let title = adw::EntryRow::builder().title("Window title").text(&c.title).build();
        list.append(&kind);
        list.append(&app_id);
        list.append(&title);
        if !c.extra.is_empty() {
            let row = adw::ActionRow::builder()
                .title("Also requires")
                .subtitle(glib::markup_escape_text(&c.extra.join(" ")))
                .build();
            list.append(&row);
        }
        CondRow { list, kind, app_id, title, remove, extra: c.extra.clone() }
    }

    fn read(&self) -> Cond {
        Cond {
            exclude: self.kind.selected() == 1,
            app_id: self.app_id.text().trim().to_string(),
            title: self.title.text().trim().to_string(),
            extra: self.extra.clone(),
        }
    }
}

/// Popover offering an empty condition or one for an open window.
fn cond_popover(button: &gtk::MenuButton, add: Rc<dyn Fn(Cond)>) -> gtk::Popover {
    let pop = gtk::Popover::new();
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["navigation-sidebar"]).build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .max_content_height(360)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    pop.set_child(Some(&scroller));
    let refill = {
        let (list, pop, add) = (list.clone(), pop.clone(), add.clone());
        move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let append = {
                let (list, pop, add) = (list.clone(), pop.clone(), add.clone());
                move |label: String, cond: Cond| {
                    let l = gtk::Label::builder()
                        .label(&label)
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .max_width_chars(50)
                        .build();
                    let row = gtk::ListBoxRow::builder().child(&l).build();
                    let (pop, add) = (pop.clone(), add.clone());
                    let gesture = gtk::GestureClick::new();
                    gesture.connect_released(move |_, _, _, _| {
                        pop.popdown();
                        add(cond.clone());
                    });
                    row.add_controller(gesture);
                    list.append(&row);
                }
            };
            append("Empty condition".into(), Cond { exclude: false, app_id: String::new(), title: String::new(), extra: vec![] });
            // Ask niri for open windows off the UI thread.
            glib::spawn_future_local(async move {
                let windows = gtk::gio::spawn_blocking(open_windows).await.unwrap_or_default();
                for (app, title) in windows {
                    let label = if title.is_empty() { app.clone() } else { format!("{app} — {title}") };
                    append(label, Cond { exclude: false, app_id: exact_regex(&app), title: String::new(), extra: vec![] });
                }
            });
        }
    };
    let _ = button;
    pop.connect_show(move |_| refill());
    pop
}

fn tri_row(label: &str, sub: &str, v: Option<bool>) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(label)
        .subtitle(sub)
        .model(&gtk::StringList::new(&["Don't change", "Yes", "No"]))
        .build();
    widgets::unclip(&row);
    row.set_selected(match v {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    });
    row
}

fn size_rows_for(label: &str, v: Option<Size>) -> (adw::ComboRow, adw::SpinRow) {
    let combo = adw::ComboRow::builder()
        .title(label)
        .model(&gtk::StringList::new(&["Don't change", "Percent of screen", "Fixed pixels", "Let the app decide"]))
        .build();
    widgets::unclip(&combo);
    let spin = adw::SpinRow::with_range(1.0, 10000.0, 1.0);
    let (sel, val) = match v {
        None => (0, 50.0),
        Some(Size::Proportion(p)) => (1, (p * 100.0).round()),
        Some(Size::Fixed(px)) => (2, px as f64),
        Some(Size::Auto) => (3, 50.0),
    };
    combo.set_selected(sel);
    spin.set_value(val);
    let sync = {
        let spin = spin.clone();
        move |c: &adw::ComboRow| {
            let s = c.selected();
            spin.set_visible(s == 1 || s == 2);
            spin.set_title(if s == 1 { "Percent" } else { "Pixels" });
            if s == 1 && spin.value() > 100.0 {
                spin.set_value(50.0);
            }
        }
    };
    sync(&combo);
    combo.connect_selected_notify(sync);
    (combo, spin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kdl::Document;

    fn parse(text: &str) -> Rule {
        let doc = Document::parse(text).unwrap();
        Rule::parse(&doc.text, &doc.nodes[0])
    }

    #[test]
    fn round_trips_known_and_other() {
        let text = r##"window-rule {
    match app-id="steam" title=r#"^notif_\d+$"# is-floating=true
    exclude title="Steam"
    open-floating
    default-column-width { fixed 1080; }
    default-window-height { proportion 0.5; }
    opacity 0.9
    background-effect {
        blur true
    }
    default-floating-position x=10 y=10 relative-to="bottom-right"
}"##;
        let r = parse(text);
        assert_eq!(r.conds.len(), 2);
        assert_eq!(r.conds[0].title, r"^notif_\d+$");
        assert_eq!(r.conds[0].extra, vec!["is-floating=true"]);
        assert_eq!(r.get_bool("open-floating"), Some(true));
        assert_eq!(r.get_size("default-column-width"), Some(Size::Fixed(1080)));
        assert_eq!(r.other.len(), 2);
        let out = r.to_kdl("");
        assert!(out.contains(r##"title=r#"^notif_\d+$"#"##), "{out}");
        assert!(out.contains("    background-effect {\n        blur true\n    }"), "{out}");
        // Re-parsing what we wrote gives the same rule.
        assert_eq!(parse(&out), r);
    }

    /// Every rule in the real config survives a save through the form model.
    #[test]
    fn real_rules_round_trip() {
        let Some((mut s, dir)) = crate::config::test_copy_of_real_config("test-rules") else { return };
        let n = s.items(NAME).len();
        for i in 0..n {
            let item = s.items(NAME)[i].clone();
            let rule = parse_item(&item);
            s.edit_item(&item, Some(rule.to_kdl(&item.indent))).unwrap();
            assert_eq!(parse_item(&s.items(NAME)[i]), rule);
        }
        let mut r = Rule::default();
        r.conds.push(Cond { exclude: false, app_id: exact_regex("foo.bar"), title: String::new(), extra: vec![] });
        r.bools.push(("open-floating".into(), true));
        r.sizes.push(("default-column-width".into(), Size::Proportion(0.5)));
        r.block_out = Some("screencast".into());
        s.add_item(&r.to_kdl("")).unwrap();
        assert_eq!(parse_item(s.items(NAME).last().unwrap()), r);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exact_regex_escapes() {
        assert_eq!(exact_regex("dev.zed.Zed"), r"^dev\.zed\.Zed$");
    }
}
