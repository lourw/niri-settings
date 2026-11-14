//! Keyboard shortcut editor for the `binds {}` section.

use super::Ctx;
use crate::kdl::{self, Document, Value};
use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Arg {
    None,
    /// `spawn-sh "command line"`
    Shell,
    /// `spawn "prog" "arg"...`
    Argv,
    /// Raw KDL arguments, with a hint.
    Raw(&'static str),
}

const ACTIONS: &[(&str, Arg)] = &[
    ("spawn-sh", Arg::Shell),
    ("spawn", Arg::Argv),
    ("close-window", Arg::None),
    ("fullscreen-window", Arg::None),
    ("toggle-windowed-fullscreen", Arg::None),
    ("toggle-window-floating", Arg::None),
    ("switch-focus-between-floating-and-tiling", Arg::None),
    ("maximize-column", Arg::None),
    ("maximize-window-to-edges", Arg::None),
    ("expand-column-to-available-width", Arg::None),
    ("center-column", Arg::None),
    ("center-window", Arg::None),
    ("center-visible-columns", Arg::None),
    ("toggle-column-tabbed-display", Arg::None),
    ("focus-column-left", Arg::None),
    ("focus-column-right", Arg::None),
    ("focus-column-first", Arg::None),
    ("focus-column-last", Arg::None),
    ("focus-column-left-or-last", Arg::None),
    ("focus-column-right-or-first", Arg::None),
    ("focus-window-up", Arg::None),
    ("focus-window-down", Arg::None),
    ("focus-window-or-workspace-up", Arg::None),
    ("focus-window-or-workspace-down", Arg::None),
    ("focus-window-previous", Arg::None),
    ("move-column-left", Arg::None),
    ("move-column-right", Arg::None),
    ("move-column-to-first", Arg::None),
    ("move-column-to-last", Arg::None),
    ("move-window-up", Arg::None),
    ("move-window-down", Arg::None),
    ("move-window-up-or-to-workspace-up", Arg::None),
    ("move-window-down-or-to-workspace-down", Arg::None),
    ("consume-or-expel-window-left", Arg::None),
    ("consume-or-expel-window-right", Arg::None),
    ("consume-window-into-column", Arg::None),
    ("expel-window-from-column", Arg::None),
    ("swap-window-left", Arg::None),
    ("swap-window-right", Arg::None),
    ("switch-preset-column-width", Arg::None),
    ("switch-preset-column-width-back", Arg::None),
    ("switch-preset-window-height", Arg::None),
    ("switch-preset-window-height-back", Arg::None),
    ("reset-window-height", Arg::None),
    ("set-column-width", Arg::Raw("e.g. \"50%\", \"+10%\", \"-100\"")),
    ("set-window-width", Arg::Raw("e.g. \"50%\", \"+10%\"")),
    ("set-window-height", Arg::Raw("e.g. \"50%\", \"+10%\"")),
    ("focus-workspace", Arg::Raw("workspace index or \"name\"")),
    ("focus-workspace-up", Arg::None),
    ("focus-workspace-down", Arg::None),
    ("focus-workspace-previous", Arg::None),
    ("move-column-to-workspace", Arg::Raw("workspace index or \"name\"")),
    ("move-column-to-workspace-up", Arg::None),
    ("move-column-to-workspace-down", Arg::None),
    ("move-window-to-workspace", Arg::Raw("workspace index or \"name\"")),
    ("move-window-to-workspace-up", Arg::None),
    ("move-window-to-workspace-down", Arg::None),
    ("move-workspace-up", Arg::None),
    ("move-workspace-down", Arg::None),
    ("focus-monitor-left", Arg::None),
    ("focus-monitor-right", Arg::None),
    ("focus-monitor-up", Arg::None),
    ("focus-monitor-down", Arg::None),
    ("focus-monitor-next", Arg::None),
    ("focus-monitor-previous", Arg::None),
    ("move-column-to-monitor-left", Arg::None),
    ("move-column-to-monitor-right", Arg::None),
    ("move-column-to-monitor-up", Arg::None),
    ("move-column-to-monitor-down", Arg::None),
    ("move-window-to-monitor-left", Arg::None),
    ("move-window-to-monitor-right", Arg::None),
    ("move-workspace-to-monitor-left", Arg::None),
    ("move-workspace-to-monitor-right", Arg::None),
    ("toggle-overview", Arg::None),
    ("show-hotkey-overlay", Arg::None),
    ("screenshot", Arg::None),
    ("screenshot-screen", Arg::None),
    ("screenshot-window", Arg::None),
    ("switch-layout", Arg::Raw("\"next\", \"prev\" or an index")),
    ("toggle-keyboard-shortcuts-inhibit", Arg::None),
    ("power-off-monitors", Arg::None),
    ("power-on-monitors", Arg::None),
    ("quit", Arg::None),
];

const MANAGED_PROPS: &[&str] =
    &["hotkey-overlay-title", "allow-when-locked", "repeat", "cooldown-ms", "allow-inhibiting"];

#[derive(Clone, Debug)]
struct Bind {
    key: String,
    props: Vec<(String, Value)>,
    action: String,
    section: String,
}

impl Bind {
    fn prop(&self, k: &str) -> Option<&Value> {
        self.props.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v)
    }
    fn title(&self) -> String {
        if let Some(t) = self.prop("hotkey-overlay-title").and_then(Value::as_str) {
            return t.to_string();
        }
        describe_action(&self.action)
    }
}

fn humanize(name: &str) -> String {
    let s = name.replace('-', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

fn describe_action(action: &str) -> String {
    let Ok(doc) = Document::parse(action) else { return action.to_string() };
    let Some(n) = doc.nodes.first() else { return action.to_string() };
    let args: Vec<String> = n
        .args()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            v => v.to_kdl(),
        })
        .collect();
    match n.name.as_str() {
        "spawn" | "spawn-sh" => format!("Run {}", args.join(" ")),
        _ if args.is_empty() => humanize(&n.name),
        _ => format!("{} {}", humanize(&n.name), args.join(" ")),
    }
}

/// Section heading from a comment like `// ─── Applications ───`.
fn heading(line: &str) -> Option<String> {
    let t = line.trim().strip_prefix("//")?;
    if !(t.contains("──") || t.contains("---") || t.contains("===")) {
        return None;
    }
    let h = t.trim_matches(|c: char| c.is_whitespace() || matches!(c, '─' | '-' | '=' | '#' | '━'));
    (!h.is_empty()).then(|| h.to_string())
}

struct BindsData {
    binds: Vec<Bind>,
    align: usize,
}

fn collect(ctx: &Ctx) -> BindsData {
    let store = ctx.store.borrow();
    let mut binds = Vec::new();
    let mut aligns = Vec::new();
    for f in &store.files {
        let text = &f.doc.text;
        for bnode in f.doc.find_all("binds") {
            let Some(ch) = &bnode.children else { continue };
            let mut section = String::new();
            let mut cursor = ch.open + 1;
            for c in &ch.nodes {
                for line in text[cursor..c.start].lines() {
                    if let Some(h) = heading(line) {
                        section = h;
                    }
                }
                cursor = c.end;
                let action = c
                    .children()
                    .first()
                    .map(|a| text[a.start..a.end].to_string())
                    .unwrap_or_default();
                let props = c
                    .entries
                    .iter()
                    .filter_map(|e| e.key.clone().map(|k| (k, e.value.clone())))
                    .collect();
                // Alignment: where the first thing after the key starts.
                let raw_name_end = c.start + kdl::ident(&c.name).len();
                if raw_name_end <= c.end && text.is_char_boundary(raw_name_end) {
                    let after = &text[raw_name_end..c.end];
                    let pad = after.len() - after.trim_start().len();
                    if pad > 1 {
                        aligns.push(raw_name_end - c.start + pad);
                    }
                }
                binds.push(Bind { key: c.name.clone(), props, action, section: section.clone() });
            }
        }
    }
    aligns.sort();
    let align = aligns.get(aligns.len() / 2).copied().unwrap_or(0);
    BindsData { binds, align }
}

/// Normalized form for conflict detection.
fn normalize_key(key: &str) -> (BTreeSet<String>, String) {
    let parts: Vec<String> = key.split('+').map(|p| p.trim().to_lowercase()).collect();
    let (last, mods) = parts.split_last().map(|(l, m)| (l.clone(), m.to_vec())).unwrap_or_default();
    let mods = mods
        .into_iter()
        .map(|m| match m.as_str() {
            "control" => "ctrl".to_string(),
            "win" => "super".to_string(),
            "mod1" => "alt".to_string(),
            "mod4" => "super".to_string(),
            _ => m,
        })
        .collect();
    (mods, last)
}

pub fn page(ctx: &Ctx) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let bar = gtk::Box::builder()
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .build();
    let clamp = adw::Clamp::builder().maximum_size(800).child(&bar).build();
    let search = gtk::SearchEntry::builder().placeholder_text("Search shortcuts").hexpand(true).build();
    let add = gtk::Button::builder()
        .label("Add Shortcut")
        .css_classes(["suggested-action"])
        .build();
    bar.append(&search);
    bar.append(&add);
    root.append(&clamp);

    let prefs = adw::PreferencesPage::new();
    prefs.set_vexpand(true);
    root.append(&prefs);
    let new_group = adw::PreferencesGroup::builder().title("New shortcut").visible(false).build();
    prefs.add(&new_group);

    let state = Rc::new(PageState {
        ctx: ctx.clone(),
        prefs,
        search: search.clone(),
        groups: RefCell::new(Vec::new()),
        new_group,
        reopen: RefCell::new(None),
        new_row: RefCell::new(None),
        generation: Cell::new(0),
    });
    state.rebuild();

    {
        let state = state.clone();
        search.connect_search_changed(move |_| state.filter());
    }
    {
        let state = state.clone();
        add.connect_clicked(move |_| state.add_new());
    }
    root.upcast()
}

struct PageState {
    ctx: Ctx,
    prefs: adw::PreferencesPage,
    search: gtk::SearchEntry,
    groups: RefCell<Vec<(adw::PreferencesGroup, Vec<(adw::ExpanderRow, String)>)>>,
    /// Holds an unsaved new shortcut, above the rest of the list.
    new_group: adw::PreferencesGroup,
    /// Key of a shortcut to show expanded after the next rebuild.
    reopen: RefCell<Option<String>>,
    new_row: RefCell<Option<adw::ExpanderRow>>,
    /// Bumped on every rebuild so a still-running chunked fill stops.
    generation: Cell<u64>,
}

impl PageState {
    fn rebuild(self: &Rc<Self>) {
        for (g, _) in self.groups.borrow_mut().drain(..) {
            self.prefs.remove(&g);
        }
        let data = collect(&self.ctx);
        if data.binds.is_empty() {
            let g = adw::PreferencesGroup::builder()
                .title("No shortcuts")
                .description("Your config has no binds section yet. Add a shortcut to create one.")
                .build();
            self.prefs.add(&g);
            self.groups.borrow_mut().push((g, Vec::new()));
            return;
        }
        // Rows are created in chunks: the first screenful right away, the
        // rest while the main loop is idle, so opening the page never stalls.
        const CHUNK: usize = 15;
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let binds = Rc::new(data.binds);
        let next = Rc::new(Cell::new(0usize));
        let last_section: Rc<RefCell<Option<String>>> = Rc::default();
        let step = {
            let st = self.clone();
            move || -> glib::ControlFlow {
                if st.generation.get() != generation {
                    return glib::ControlFlow::Break;
                }
                let start = next.get();
                let end = (start + CHUNK).min(binds.len());
                super::timed(
                    || format!("shortcut rows {start}..{end}"),
                    || {
                        for b in &binds[start..end] {
                            st.add_bind_row(b, &mut last_section.borrow_mut());
                        }
                        st.filter();
                    },
                );
                next.set(end);
                if end == binds.len() { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
            }
        };
        if step() == glib::ControlFlow::Continue {
            glib::idle_add_local(step);
        }
    }

    /// Append one bind, starting a new group when its section changes.
    fn add_bind_row(self: &Rc<Self>, b: &Bind, last_section: &mut Option<String>) {
        if last_section.as_ref() != Some(&b.section) {
            let title = if b.section.is_empty() { "Shortcuts".to_string() } else { b.section.clone() };
            let g = adw::PreferencesGroup::builder().title(glib::markup_escape_text(&title)).build();
            self.prefs.add(&g);
            self.groups.borrow_mut().push((g, Vec::new()));
            *last_section = Some(b.section.clone());
        }
        let row = adw::ExpanderRow::builder().title(glib::markup_escape_text(&b.title())).build();
        if b.prop("hotkey-overlay-title").is_some() {
            row.set_subtitle(&glib::markup_escape_text(&b.action));
            row.set_subtitle_lines(1);
        }
        row.add_suffix(&keycaps(&b.key));
        let st = self.clone();
        let bind = b.clone();
        super::widgets::lazy_rows(&row, move |row| edit_form(&st, row, Some(bind)));
        if self.reopen.borrow().as_deref() == Some(b.key.as_str()) {
            self.reopen.replace(None);
            row.set_expanded(true);
        }
        let hay = format!("{} {} {}", b.key, b.title(), b.action).to_lowercase();
        let mut groups = self.groups.borrow_mut();
        let g = groups.last_mut().unwrap();
        g.0.add(&row);
        g.1.push((row, hay));
    }

    /// Show an unsaved shortcut, opened for editing, at the top.
    fn add_new(self: &Rc<Self>) {
        if let Some(row) = self.new_row.borrow().clone() {
            row.set_expanded(true);
            return;
        }
        let row = adw::ExpanderRow::builder().title("New shortcut").subtitle("Not saved yet").build();
        let st = self.clone();
        super::widgets::lazy_rows(&row, move |row| edit_form(&st, row, None));
        self.new_group.add(&row);
        self.new_group.set_visible(true);
        row.set_expanded(true);
        *self.new_row.borrow_mut() = Some(row);
    }

    /// Remove the unsaved new shortcut, if any.
    fn close_new(&self) {
        if let Some(row) = self.new_row.borrow_mut().take() {
            self.new_group.remove(&row);
        }
        self.new_group.set_visible(false);
    }

    fn filter(&self) {
        let q = self.search.text().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        for (g, rows) in self.groups.borrow().iter() {
            let mut any = rows.is_empty();
            for (row, hay) in rows {
                let vis = words.iter().all(|w| hay.contains(w));
                row.set_visible(vis);
                any |= vis;
            }
            g.set_visible(any);
        }
    }
}

pub fn keycaps(key: &str) -> gtk::Box {
    let b = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).build();
    for part in key.split('+') {
        let label = gtk::Label::builder().label(part).css_classes(["keycap"]).build();
        b.append(&label);
    }
    b
}

/// Turn a GDK key event into niri's key syntax, e.g. `Mod+Shift+Q`.
fn key_from_event(
    display: &gdk::Display,
    keyval: gdk::Key,
    keycode: u32,
    state: gdk::ModifierType,
    mod_key: &str,
) -> Option<String> {
    let name = keyval.name()?.to_string();
    let is_modifier = name.starts_with("Shift_")
        || name.starts_with("Control_")
        || name.starts_with("Alt_")
        || name.starts_with("Super_")
        || name.starts_with("Meta_")
        || name.starts_with("Hyper_")
        || name.starts_with("ISO_Level")
        || name == "Caps_Lock"
        || name == "Num_Lock";
    if is_modifier {
        return None;
    }
    // niri matches the unshifted keysym, so look up level 0 of the key.
    let base = display
        .map_keycode(keycode)
        .and_then(|entries| {
            entries
                .into_iter()
                .find(|(k, _)| k.group() == 0 && k.level() == 0)
                .and_then(|(_, key)| key.name())
        })
        .map(|n| n.to_string())
        .unwrap_or(name);
    let base = if base.chars().count() == 1 { base.to_uppercase() } else { base };
    // Whichever modifier niri treats as "Mod" (input.mod-key) is written as Mod.
    let held = [
        (gdk::ModifierType::SUPER_MASK, "Super"),
        (gdk::ModifierType::CONTROL_MASK, "Ctrl"),
        (gdk::ModifierType::ALT_MASK, "Alt"),
        (gdk::ModifierType::SHIFT_MASK, "Shift"),
    ];
    let mut parts = Vec::new();
    if held.iter().any(|(m, n)| state.contains(*m) && n.eq_ignore_ascii_case(mod_key)) {
        parts.push("Mod");
    }
    for (m, n) in held {
        if state.contains(m) && !n.eq_ignore_ascii_case(mod_key) {
            parts.push(n);
        }
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&base);
    Some(s)
}

fn quote_args(argv: &[String]) -> String {
    argv.iter().map(|a| kdl::quote(a)).collect::<Vec<_>>().join(" ")
}

/// The editing form inside a shortcut's expander (None = a new shortcut).
fn edit_form(state: &Rc<PageState>, row: &adw::ExpanderRow, existing: Option<Bind>) {
    let ctx = &state.ctx;
    let is_new = existing.is_none();
    let save = gtk::Button::builder().label("Save").css_classes(["suggested-action"]).build();
    let discard = gtk::Button::with_label("Discard");
    save.set_sensitive(is_new);
    discard.set_sensitive(is_new);
    let dirty: Rc<dyn Fn()> = {
        let (save, discard) = (save.clone(), discard.clone());
        Rc::new(move || {
            save.set_sensitive(true);
            discard.set_sensitive(true);
        })
    };

    // ── Keys ──
    let keys = adw::EntryRow::builder().title("Keys").build();
    keys.set_tooltip_text(Some("Record a key combination, or type one (e.g. Mod+Shift+Return, Mod+WheelScrollDown)"));
    let record = gtk::Button::builder()
        .icon_name("media-record-symbolic")
        .tooltip_text("Record shortcut")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    keys.add_suffix(&record);
    let conflict = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["error", "caption"])
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .visible(false)
        .build();
    row.add_row(&keys);
    row.add_row(&conflict);

    // ── Action ──
    let mut labels: Vec<String> = ACTIONS
        .iter()
        .map(|(n, a)| match a {
            Arg::Shell => "Run a shell command (spawn-sh)".to_string(),
            Arg::Argv => "Run a program (spawn)".to_string(),
            _ => format!("{} ({n})", humanize(n)),
        })
        .collect();
    labels.push("Custom action (raw KDL)".into());
    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let action_row = adw::ComboRow::builder().title("Action").model(&model).enable_search(true).build();
    super::widgets::unclip(&action_row);
    action_row.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    action_row.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
    let arg_row = adw::EntryRow::builder().title("Command").build();
    row.add_row(&action_row);
    row.add_row(&arg_row);

    // ── Options ──
    let title_row = adw::EntryRow::builder().title("Title in hotkey overlay (optional)").build();
    let locked = adw::SwitchRow::builder().title("Works on the lock screen").build();
    let repeat = adw::SwitchRow::builder().title("Repeat while held").active(true).build();
    let inhibit = adw::SwitchRow::builder()
        .title("Can be inhibited by apps")
        .subtitle("Fullscreen apps and remote desktops may capture this shortcut")
        .active(true)
        .build();
    let cooldown = adw::SpinRow::with_range(0.0, 10_000.0, 10.0);
    cooldown.set_title("Cooldown (ms)");
    cooldown.set_subtitle("Minimum time between triggers; useful for scroll wheel binds");
    row.add_row(&title_row);
    row.add_row(&locked);
    row.add_row(&repeat);
    row.add_row(&inhibit);
    row.add_row(&cooldown);

    // Fill from the existing bind.
    let custom_idx = ACTIONS.len() as u32;
    let mut initial_sel = 0u32;
    if let Some(b) = &existing {
        keys.set_text(&b.key);
        if let Some(t) = b.prop("hotkey-overlay-title").and_then(Value::as_str) {
            title_row.set_text(t);
        }
        locked.set_active(b.prop("allow-when-locked").and_then(Value::as_bool).unwrap_or(false));
        repeat.set_active(b.prop("repeat").and_then(Value::as_bool).unwrap_or(true));
        inhibit.set_active(b.prop("allow-inhibiting").and_then(Value::as_bool).unwrap_or(true));
        cooldown.set_value(b.prop("cooldown-ms").and_then(Value::as_f64).unwrap_or(0.0));

        initial_sel = custom_idx;
        arg_row.set_text(&b.action);
        if let Ok(doc) = Document::parse(b.action.as_str()) {
            if let Some(n) = doc.nodes.first().filter(|n| n.children().is_empty()) {
                let strings: Option<Vec<String>> = n.args().map(|v| v.as_str().map(String::from)).collect();
                let props = n.entries.iter().any(|e| e.key.is_some());
                if let Some(i) = ACTIONS.iter().position(|(name, _)| *name == n.name).filter(|_| !props) {
                    let (_, kind) = ACTIONS[i];
                    let raw_args = b.action[n.name.len()..].trim().to_string();
                    let ok = match kind {
                        Arg::None => n.args().next().is_none(),
                        Arg::Shell => match strings.as_deref() {
                            Some([cmd]) => {
                                arg_row.set_text(cmd);
                                true
                            }
                            _ => false,
                        },
                        Arg::Argv => match &strings {
                            Some(argv) if !argv.is_empty() => {
                                let joined = shlex::try_join(argv.iter().map(String::as_str))
                                    .unwrap_or_else(|_| argv.join(" "));
                                arg_row.set_text(&joined);
                                true
                            }
                            _ => false,
                        },
                        Arg::Raw(_) => {
                            arg_row.set_text(&raw_args);
                            true
                        }
                    };
                    if ok {
                        initial_sel = i as u32;
                    } else {
                        arg_row.set_text(&b.action);
                    }
                }
            }
        }
    }

    let update_arg_row = {
        let arg_row = arg_row.clone();
        move |sel: u32| {
            if sel == custom_idx {
                arg_row.set_visible(true);
                arg_row.set_title("Action (KDL, e.g. focus-workspace 3)");
                return;
            }
            match ACTIONS[sel as usize].1 {
                Arg::None => arg_row.set_visible(false),
                Arg::Shell => {
                    arg_row.set_visible(true);
                    arg_row.set_title("Shell command");
                }
                Arg::Argv => {
                    arg_row.set_visible(true);
                    arg_row.set_title("Program and arguments");
                }
                Arg::Raw(hint) => {
                    arg_row.set_visible(true);
                    arg_row.set_title(&format!("Arguments — {hint}"));
                }
            }
        }
    };
    action_row.set_selected(initial_sel);
    update_arg_row(initial_sel);
    {
        let arg_row = arg_row.clone();
        let prev = std::cell::Cell::new(initial_sel);
        action_row.connect_selected_notify(move |row| {
            let sel = row.selected();
            // Switching kinds: clear arguments that no longer make sense.
            if prev.get() != sel {
                arg_row.set_text("");
            }
            prev.set(sel);
            update_arg_row(sel);
        });
    }

    // Any edit enables Save/Discard (connected after filling in values).
    for e in [&keys, &arg_row, &title_row] {
        let d = dirty.clone();
        e.connect_changed(move |_| d());
    }
    for sw in [&locked, &repeat, &inhibit] {
        let d = dirty.clone();
        sw.connect_active_notify(move |_| d());
    }
    {
        let d = dirty.clone();
        cooldown.connect_value_notify(move |_| d());
        let d = dirty.clone();
        action_row.connect_selected_notify(move |_| d());
    }

    // Conflict detection.
    let data = Rc::new(collect(ctx));
    let find_conflict = {
        let data = data.clone();
        let own = existing.as_ref().map(|b| b.key.clone());
        move |key: &str| -> Option<Bind> {
            let n = normalize_key(key);
            data.binds
                .iter()
                .find(|b| normalize_key(&b.key) == n && Some(&b.key) != own.as_ref())
                .cloned()
        }
    };
    {
        let conflict = conflict.clone();
        let find_conflict = find_conflict.clone();
        keys.connect_changed(move |e| {
            match find_conflict(e.text().trim()) {
                Some(b) => {
                    conflict.set_label(&format!("Already used by “{}”. Saving will replace it.", b.title()));
                    conflict.set_visible(true);
                }
                None => conflict.set_visible(false),
            }
        });
    }

    // Recording.
    let mod_key = ctx.store.borrow().string(&["input", "mod-key"]).unwrap_or_else(|| "Super".into());
    let recording = Rc::new(RefCell::new(None::<(gtk::EventControllerKey, String)>));
    let stop_recording = {
        let recording = recording.clone();
        let window = ctx.window.clone();
        let keys = keys.clone();
        let record = record.clone();
        Rc::new(move |restore: bool| {
            if let Some((ctrl, prev)) = recording.borrow_mut().take() {
                window.remove_controller(&ctrl);
                if restore {
                    keys.set_text(&prev);
                }
            }
            record.remove_css_class("destructive-action");
            record.set_icon_name("media-record-symbolic");
            if let Some(t) = window.surface().and_then(|s| s.downcast::<gdk::Toplevel>().ok()) {
                t.restore_system_shortcuts();
            }
        })
    };
    {
        let recording = recording.clone();
        let window = ctx.window.clone();
        let keys = keys.clone();
        let stop = stop_recording.clone();
        record.connect_clicked(move |btn| {
            if recording.borrow().is_some() {
                stop(true);
                return;
            }
            // Ask niri to let us see its own shortcuts while recording.
            if let Some(t) = window.surface().and_then(|s| s.downcast::<gdk::Toplevel>().ok()) {
                t.inhibit_system_shortcuts(None::<&gdk::Event>);
            }
            let prev = keys.text().to_string();
            keys.set_text("");
            keys.set_title("Press a key combination… (Esc to cancel)");
            btn.add_css_class("destructive-action");
            btn.set_icon_name("media-playback-stop-symbolic");
            let ctrl = gtk::EventControllerKey::new();
            ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
            let keys2 = keys.clone();
            let stop2 = stop.clone();
            let mod_key = mod_key.clone();
            ctrl.connect_key_pressed(move |c, keyval, keycode, state| {
                let mods = state & gtk::accelerator_get_default_mod_mask();
                if keyval == gdk::Key::Escape && mods.is_empty() {
                    keys2.set_title("Keys");
                    stop2(true);
                    return glib::Propagation::Stop;
                }
                let display = c.widget().map(|w| w.display()).unwrap_or_else(|| gdk::Display::default().unwrap());
                if let Some(k) = key_from_event(&display, keyval, keycode, mods, &mod_key) {
                    keys2.set_title("Keys");
                    keys2.set_text(&k);
                    stop2(false);
                }
                glib::Propagation::Stop
            });
            // Listen on the whole window, ahead of other widgets.
            window.add_controller(ctrl.clone());
            *recording.borrow_mut() = Some((ctrl, prev));
        });
    }
    {
        // Stop recording when the row is collapsed or goes away.
        let stop = stop_recording.clone();
        row.connect_expanded_notify(move |r| {
            if !r.is_expanded() {
                stop(true);
            }
        });
        let stop = stop_recording.clone();
        row.connect_unmap(move |_| stop(true));
    }

    // ── Actions ──
    let actions = gtk::Box::builder().spacing(6).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
    let remove = gtk::Button::builder()
        .label(if is_new { "Cancel" } else { "Delete Shortcut" })
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
        let st = state.clone();
        let key = existing.as_ref().map(|b| b.key.clone());
        remove.connect_clicked(move |_| match key.clone() {
            None => st.close_new(),
            Some(key) => {
                let st2 = st.clone();
                st.ctx.apply(move |s| s.set_node(&["binds", &key], None), move |ok| {
                    if ok {
                        st2.ctx.toast("Shortcut deleted");
                        st2.rebuild();
                    }
                });
            }
        });
    }
    {
        // Discard: rebuild, keeping this shortcut open with its saved values.
        let st = state.clone();
        let key = existing.as_ref().map(|b| b.key.clone());
        discard.connect_clicked(move |_| {
            *st.reopen.borrow_mut() = key.clone();
            st.rebuild();
        });
    }
    {
        let st = state.clone();
        save.connect_clicked(move |_| {
            let key = keys.text().trim().to_string();
            if key.is_empty() {
                st.ctx.error("Missing shortcut", "Record or type a key combination first.");
                return;
            }
            let sel = action_row.selected();
            let arg = arg_row.text().trim().to_string();
            let action = if sel == custom_idx {
                arg.clone()
            } else {
                let (name, kind) = ACTIONS[sel as usize];
                match kind {
                    Arg::None => name.to_string(),
                    Arg::Shell => format!("spawn-sh {}", kdl::quote(&arg)),
                    Arg::Argv => match shlex::split(&arg) {
                        Some(argv) if !argv.is_empty() => format!("spawn {}", quote_args(&argv)),
                        _ => {
                            st.ctx.error("Invalid command", "Couldn't split the command into arguments (check your quotes).");
                            return;
                        }
                    },
                    Arg::Raw(_) => {
                        if arg.is_empty() { name.to_string() } else { format!("{name} {arg}") }
                    }
                }
            };
            match Document::parse(action.as_str()) {
                Ok(d) if d.nodes.len() == 1 => {}
                _ => {
                    st.ctx.error("Invalid action", &format!("“{action}” is not a single valid niri action."));
                    return;
                }
            }
            if matches!(sel, s if s != custom_idx && matches!(ACTIONS[s as usize].1, Arg::Shell | Arg::Argv))
                && arg.is_empty()
            {
                st.ctx.error("Missing command", "Enter the command to run.");
                return;
            }

            // Properties: keep unknown ones from the original bind.
            let mut props: Vec<String> = Vec::new();
            let title = title_row.text().trim().to_string();
            if !title.is_empty() {
                props.push(format!("hotkey-overlay-title={}", kdl::quote(&title)));
            }
            if locked.is_active() {
                props.push("allow-when-locked=true".into());
            }
            if !repeat.is_active() {
                props.push("repeat=false".into());
            }
            if !inhibit.is_active() {
                props.push("allow-inhibiting=false".into());
            }
            let cd = cooldown.value().round() as i64;
            if cd > 0 {
                props.push(format!("cooldown-ms={cd}"));
            }
            if let Some(b) = &existing {
                for (k, v) in &b.props {
                    if !MANAGED_PROPS.contains(&k.as_str()) {
                        props.push(format!("{}={}", kdl::ident(k), v.to_kdl()));
                    }
                }
            }

            let key_text = kdl::ident(&key);
            let width = data.align.max(key_text.len() + 1);
            let mut text = format!("{key_text:<width$}");
            for p in &props {
                text.push_str(p);
                text.push(' ');
            }
            text.push_str(&format!("{{ {action}; }}"));

            let conflict = find_conflict(&key);
            let do_save = {
                let st = st.clone();
                let existing_key = existing.as_ref().map(|b| b.key.clone());
                let key = key.clone();
                move |conflict: Option<Bind>| {
                    let (existing_key, key, text) = (existing_key.clone(), key.clone(), text.clone());
                    let st2 = st.clone();
                    let saved_key = key.clone();
                    st.ctx.apply(move |s| {
                        let mut edits: Vec<(Vec<&str>, Option<String>)> = Vec::new();
                        if let Some(c) = &conflict {
                            edits.push((vec!["binds", c.key.as_str()], None));
                        }
                        // Replace in place when editing; append when adding.
                        let target = existing_key.as_deref().unwrap_or(key.as_str());
                        edits.push((vec!["binds", target], Some(text.clone())));
                        let refs: Vec<(&[&str], Option<String>)> =
                            edits.iter().map(|(p, t)| (p.as_slice(), t.clone())).collect();
                        s.set_nodes(&refs)
                    }, move |ok| {
                        if ok {
                            st2.ctx.toast("Shortcut saved");
                            st2.close_new();
                            *st2.reopen.borrow_mut() = Some(saved_key);
                            st2.rebuild();
                        }
                    });
                }
            };
            match conflict {
                None => do_save(None),
                Some(c) => {
                    let alert = adw::AlertDialog::new(
                        Some("Replace existing shortcut?"),
                        Some(&format!("{} is already used by “{}”.", key, c.title())),
                    );
                    alert.add_response("cancel", "Cancel");
                    alert.add_response("replace", "Replace");
                    alert.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
                    alert.connect_response(None, move |_, r| {
                        if r == "replace" {
                            do_save(Some(c.clone()));
                        }
                    });
                    alert.present(Some(&st.ctx.window));
                }
            }
        });
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings() {
        assert_eq!(heading("    // ─── Applications ───").as_deref(), Some("Applications"));
        assert_eq!(heading("    // --- Noctalia ---").as_deref(), Some("Noctalia"));
        assert_eq!(heading("// Please set the text editor").as_deref(), None);
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_key("Mod+CTRL+Up"), normalize_key("Mod+Ctrl+UP"));
        assert_eq!(normalize_key("Ctrl+Mod+Up"), normalize_key("Mod+Control+up"));
        assert_ne!(normalize_key("Mod+Up"), normalize_key("Mod+Shift+Up"));
    }

    #[test]
    fn describe() {
        assert_eq!(describe_action("focus-column-left"), "Focus column left");
        assert_eq!(describe_action("spawn \"ghostty\""), "Run ghostty");
        assert_eq!(describe_action("focus-workspace 3"), "Focus workspace 3");
    }
}
