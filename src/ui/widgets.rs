//! Rows bound to a single config node. Each reads its initial state from the
//! store and writes on change.

use super::{Ctx, Path, refs};
use crate::config::ConfigStore;
use crate::kdl::Value;
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Calls `f` after `ms` of quiet, cancelling earlier pending calls.
#[derive(Clone, Default)]
pub struct Debouncer(Rc<RefCell<Option<glib::SourceId>>>);

impl Debouncer {
    pub fn run(&self, ms: u64, f: impl FnOnce() + 'static) {
        if let Some(id) = self.0.borrow_mut().take() {
            id.remove();
        }
        let slot = self.0.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
            slot.borrow_mut().take();
            f();
        });
        *self.0.borrow_mut() = Some(id);
    }
}

/// What a value row edits: a node's first argument, or one property of it.
#[derive(Clone)]
pub enum Target {
    Arg(Path),
    Prop(Path, &'static str),
}

impl From<Path> for Target {
    fn from(p: Path) -> Self {
        Target::Arg(p)
    }
}

impl Target {
    pub fn read(&self, s: &ConfigStore) -> Option<Value> {
        match self {
            Target::Arg(p) => s.value(&refs(p)),
            Target::Prop(p, k) => s.get(&refs(p)).and_then(|n| n.prop(k).cloned()),
        }
    }
    /// Properties are reset by writing the default explicitly: removing one
    /// can leave the node invalid (e.g. a `spring` missing `stiffness`).
    fn reset_value(&self, default: Value) -> Option<Value> {
        match self {
            Target::Arg(_) => None,
            Target::Prop(..) => Some(default),
        }
    }
    pub fn write(&self, s: &mut ConfigStore, v: Option<Value>) -> Result<(), String> {
        match self {
            Target::Arg(p) => s.set_value(&refs(p), v),
            Target::Prop(p, k) => s.set_props(&refs(p), &[(k, v)]),
        }
    }
}

fn reset_button(tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build()
}

/// Presence flag: node present = on, absent = off.
pub fn switch(ctx: &Ctx, title: &str, subtitle: &str, p: Path) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
    row.set_active(ctx.store.borrow().flag(&refs(&p)));
    let ctx = ctx.clone();
    let guard = Rc::new(Cell::new(false));
    row.connect_active_notify(move |row| {
        if guard.get() {
            return;
        }
        let on = row.is_active();
        let p = p.clone();
        let (row, guard) = (row.clone(), guard.clone());
        ctx.apply(move |s| s.set_flag(&refs(&p), on), move |ok| {
            if !ok {
                guard.set(true);
                row.set_active(!on);
                guard.set(false);
            }
        });
    });
    row
}

/// Boolean with an argument and a default, e.g. `drag false`. Writing the
/// default value removes the node.
pub fn bool_arg(ctx: &Ctx, title: &str, subtitle: &str, p: Path, default: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
    let current = {
        let s = ctx.store.borrow();
        match s.get(&refs(&p)) {
            Some(n) => n.arg(0).and_then(Value::as_bool).unwrap_or(true),
            None => default,
        }
    };
    row.set_active(current);
    let ctx = ctx.clone();
    let guard = Rc::new(Cell::new(false));
    row.connect_active_notify(move |row| {
        if guard.get() {
            return;
        }
        let on = row.is_active();
        let v = (on != default).then_some(Value::Bool(on));
        let p = p.clone();
        let (row, guard) = (row.clone(), guard.clone());
        ctx.apply(move |s| s.set_value(&refs(&p), v), move |ok| {
            if !ok {
                guard.set(true);
                row.set_active(!on);
                guard.set(false);
            }
        });
    });
    row
}

/// String enum. An option with `None` value means "remove node" (default).
pub fn combo(
    ctx: &Ctx,
    title: &str,
    subtitle: &str,
    p: Path,
    options: &[(&str, Option<&str>)],
) -> adw::ComboRow {
    let mut labels: Vec<String> = options.iter().map(|(l, _)| l.to_string()).collect();
    let mut values: Vec<Option<String>> = options.iter().map(|(_, v)| v.map(String::from)).collect();
    let current = ctx.store.borrow().string(&refs(&p));
    let mut selected = values.iter().position(|v| *v == current);
    if selected.is_none() {
        // Unknown value in the config: keep it selectable.
        labels.push(current.clone().unwrap_or_default());
        values.push(current);
        selected = Some(values.len() - 1);
    }
    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let row = adw::ComboRow::builder()
        .title(title)
        .subtitle(subtitle)
        .model(&model)
        .list_factory(&unellipsized_list_factory())
        .build();
    row.set_selected(selected.unwrap() as u32);
    let ctx = ctx.clone();
    let last = Rc::new(Cell::new(selected.unwrap() as u32));
    let guard = Rc::new(Cell::new(false));
    row.connect_selected_notify(move |row| {
        if guard.get() {
            return;
        }
        let i = row.selected();
        let Some(v) = values.get(i as usize).cloned() else { return };
        let p = p.clone();
        let (row, guard, last) = (row.clone(), guard.clone(), last.clone());
        ctx.apply(move |s| s.set_value(&refs(&p), v.map(Value::String)), move |ok| {
            if ok {
                last.set(i);
            } else {
                guard.set(true);
                row.set_selected(last.get());
                guard.set(false);
            }
        });
    });
    row
}

/// Popup item factory that shows full labels (wrapping if needed) instead of
/// AdwComboRow's default ellipsized ones.
fn unellipsized_list_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(40)
            .build();
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let (Some(label), Some(obj)) = (
            item.child().and_downcast::<gtk::Label>(),
            item.item().and_downcast::<gtk::StringObject>(),
        ) else {
            return;
        };
        label.set_label(&obj.string());
    });
    factory
}

pub struct NumSpec {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub digits: u32,
    pub float: bool,
    pub default: f64,
}

fn num_value(spec: &NumSpec, v: f64) -> Value {
    if spec.float { Value::Float(v) } else { Value::Int(v.round() as i64) }
}

/// Numeric value with a reset-to-default button.
pub fn spin(ctx: &Ctx, title: &str, subtitle: &str, p: impl Into<Target>, spec: NumSpec) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(spec.min, spec.max, spec.step);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_digits(spec.digits);
    let p: Target = p.into();
    let current = p.read(&ctx.store.borrow()).and_then(|v| v.as_f64());
    row.set_value(current.unwrap_or(spec.default));

    let reset = reset_button("Reset to default");
    reset.set_visible(current.is_some());
    row.add_suffix(&reset);

    let guard = Rc::new(Cell::new(false));
    let deb = Debouncer::default();
    let default = spec.default;
    let reset_val = p.reset_value(num_value(&spec, default));
    {
        let ctx = ctx.clone();
        let p = p.clone();
        let guard = guard.clone();
        let reset = reset.clone();
        row.connect_value_notify(move |row| {
            if guard.get() {
                return;
            }
            let ctx = ctx.clone();
            let p = p.clone();
            let reset = reset.clone();
            // A zero default means "unset" (e.g. no auto-hide); don't write 0.
            let unset = spec.default == 0.0 && row.value() == 0.0 && matches!(p, Target::Arg(_));
            let val = (!unset).then(|| num_value(&spec, row.value()));
            deb.run(400, move || {
                let set = val.is_some();
                ctx.apply(move |s| p.write(s, val), move |ok| {
                    if ok {
                        reset.set_visible(set);
                    }
                });
            });
        });
    }
    {
        let ctx = ctx.clone();
        let row2 = row.clone();
        reset.connect_clicked(move |btn| {
            let p = p.clone();
            let (row2, guard, btn) = (row2.clone(), guard.clone(), btn.clone());
            let v = reset_val.clone();
            ctx.apply(move |s| p.write(s, v), move |ok| {
                if ok {
                    guard.set(true);
                    row2.set_value(default);
                    guard.set(false);
                    btn.set_visible(false);
                }
            });
        });
    }
    row
}

/// A float slider, e.g. pointer acceleration in [-1, 1].
pub fn slider(ctx: &Ctx, title: &str, subtitle: &str, p: Path, min: f64, max: f64, default: f64) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 0.05);
    scale.set_width_request(220);
    scale.set_draw_value(true);
    scale.set_digits(2);
    scale.set_value_pos(gtk::PositionType::Left);
    scale.add_mark(default, gtk::PositionType::Bottom, None);
    scale.set_valign(gtk::Align::Center);
    let current = ctx.store.borrow().value(&refs(&p)).and_then(|v| v.as_f64());
    scale.set_value(current.unwrap_or(default));
    let reset = reset_button("Reset to default");
    reset.set_visible(current.is_some());
    row.add_suffix(&scale);
    row.add_suffix(&reset);

    let guard = Rc::new(Cell::new(false));
    let deb = Debouncer::default();
    {
        let ctx = ctx.clone();
        let p = p.clone();
        let guard = guard.clone();
        let reset = reset.clone();
        scale.connect_value_changed(move |scale| {
            if guard.get() {
                return;
            }
            let v = (scale.value() * 100.0).round() / 100.0;
            let ctx = ctx.clone();
            let p = p.clone();
            let reset = reset.clone();
            deb.run(350, move || {
                ctx.apply(move |s| s.set_value(&refs(&p), Some(Value::Float(v))), move |ok| {
                    if ok {
                        reset.set_visible(true);
                    }
                });
            });
        });
    }
    {
        let ctx = ctx.clone();
        let scale = scale.clone();
        reset.connect_clicked(move |btn| {
            let p = p.clone();
            let (scale, guard, btn) = (scale.clone(), guard.clone(), btn.clone());
            ctx.apply(move |s| s.set_value(&refs(&p), None), move |ok| {
                if ok {
                    guard.set(true);
                    scale.set_value(default);
                    guard.set(false);
                    btn.set_visible(false);
                }
            });
        });
    }
    row
}


pub fn group(title: &str, description: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(gtk::glib::markup_escape_text(title)).build();
    if !description.is_empty() {
        g.set_description(Some(description));
    }
    g
}

/// Presence flag shown inverted: node present = off (e.g. `animations { off }`).
pub fn off_switch(ctx: &Ctx, title: &str, subtitle: &str, p: Path) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
    row.set_active(!ctx.store.borrow().flag(&refs(&p)));
    let ctx = ctx.clone();
    let guard = Rc::new(Cell::new(false));
    row.connect_active_notify(move |row| {
        if guard.get() {
            return;
        }
        let on = row.is_active();
        let p = p.clone();
        let (row, guard) = (row.clone(), guard.clone());
        ctx.apply(move |s| s.set_flag(&refs(&p), !on), move |ok| {
            if !ok {
                guard.set(true);
                row.set_active(!on);
                guard.set(false);
            }
        });
    });
    row
}

/// Expander for a block with `on`/`off` flags (`focus-ring`, `border`,
/// `shadow`…). The enable switch writes `on` or `off` only when it differs
/// from niri's default; sub-rows go inside via `add_row`.
pub fn block_expander(ctx: &Ctx, title: &str, subtitle: &str, base: &[&str], default_on: bool) -> adw::ExpanderRow {
    let on_p: Vec<String> = base.iter().map(|s| s.to_string()).chain(["on".into()]).collect();
    let off_p: Vec<String> = base.iter().map(|s| s.to_string()).chain(["off".into()]).collect();
    let enabled = {
        let s = ctx.store.borrow();
        let r = |p: &[String]| s.flag(&p.iter().map(String::as_str).collect::<Vec<_>>());
        if r(&off_p) {
            false
        } else if r(&on_p) {
            true
        } else {
            default_on
        }
    };
    let row = adw::ExpanderRow::builder()
        .title(title)
        .subtitle(subtitle)
        .show_enable_switch(true)
        .enable_expansion(enabled)
        .build();
    let ctx = ctx.clone();
    let guard = Rc::new(Cell::new(false));
    row.connect_enable_expansion_notify(move |row| {
        if guard.get() {
            return;
        }
        let on = row.enables_expansion();
        let (on_p, off_p) = (on_p.clone(), off_p.clone());
        let (row, guard) = (row.clone(), guard.clone());
        ctx.apply(
            move |s| {
                let on_r: Vec<&str> = on_p.iter().map(String::as_str).collect();
                let off_r: Vec<&str> = off_p.iter().map(String::as_str).collect();
                s.set_nodes(&[
                    (&on_r, (on && !default_on).then(|| "on".into())),
                    (&off_r, (!on && default_on).then(|| "off".into())),
                ])
            },
            move |ok| {
                if !ok {
                    guard.set(true);
                    row.set_enable_expansion(!on);
                    guard.set(false);
                }
            },
        );
    });
    row
}

/// Combo whose options are whole node texts (None = remove the node).
/// `selected` is the index matching the current config.
pub fn node_combo(
    ctx: &Ctx,
    title: &str,
    subtitle: &str,
    p: Path,
    options: Vec<(String, Option<String>)>,
    selected: usize,
) -> adw::ComboRow {
    let labels: Vec<&str> = options.iter().map(|(l, _)| l.as_str()).collect();
    let row = adw::ComboRow::builder()
        .title(title)
        .subtitle(subtitle)
        .model(&gtk::StringList::new(&labels))
        .list_factory(&unellipsized_list_factory())
        .build();
    row.set_selected(selected as u32);
    let values: Vec<Option<String>> = options.into_iter().map(|(_, v)| v).collect();
    let ctx = ctx.clone();
    let last = Rc::new(Cell::new(selected as u32));
    let guard = Rc::new(Cell::new(false));
    row.connect_selected_notify(move |row| {
        if guard.get() {
            return;
        }
        let i = row.selected();
        let Some(v) = values.get(i as usize).cloned() else { return };
        let p = p.clone();
        let (row, guard, last) = (row.clone(), guard.clone(), last.clone());
        ctx.apply(move |s| s.set_node(&refs(&p), v), move |ok| {
            if ok {
                last.set(i);
            } else {
                guard.set(true);
                row.set_selected(last.get());
                guard.set(false);
            }
        });
    });
    row
}

/// Hex form niri accepts: `#rrggbb`, or `#rrggbbaa` when translucent.
pub fn rgba_hex(c: &gtk::gdk::RGBA) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, bl, a) = (b(c.red()), b(c.green()), b(c.blue()), b(c.alpha()));
    if a == 255 { format!("#{r:02x}{g:02x}{bl:02x}") } else { format!("#{r:02x}{g:02x}{bl:02x}{a:02x}") }
}

/// A CSS color string with a color picker and reset-to-default button.
pub fn color(ctx: &Ctx, title: &str, subtitle: &str, p: impl Into<Target>, default: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
    let p: Target = p.into();
    let current = p.read(&ctx.store.borrow()).and_then(|v| v.as_str().map(String::from));
    let parse = |s: &str| match s.trim() {
        t if t.eq_ignore_ascii_case("transparent") => Some(gtk::gdk::RGBA::TRANSPARENT),
        t => gtk::gdk::RGBA::parse(t).ok(),
    };
    let default_rgba = parse(default).unwrap_or(gtk::gdk::RGBA::BLACK);
    let button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(true).build()));
    button.set_valign(gtk::Align::Center);
    button.set_rgba(&current.as_deref().and_then(parse).unwrap_or(default_rgba));
    if let Some(c) = current.as_deref().filter(|c| parse(c).is_none()) {
        // Something GTK can't parse; show it rather than pretending.
        row.set_subtitle(&format!("Current value: {c}"));
    }
    let reset = reset_button("Reset to default");
    reset.set_visible(current.is_some());
    row.add_suffix(&button);
    row.add_suffix(&reset);
    row.set_activatable_widget(Some(&button));

    let reset_val = p.reset_value(Value::String(default.to_string()));
    let guard = Rc::new(Cell::new(false));
    let deb = Debouncer::default();
    {
        let ctx = ctx.clone();
        let p = p.clone();
        let guard = guard.clone();
        let reset = reset.clone();
        button.connect_rgba_notify(move |b| {
            if guard.get() {
                return;
            }
            let hex = rgba_hex(&b.rgba());
            let (ctx, p, reset) = (ctx.clone(), p.clone(), reset.clone());
            deb.run(300, move || {
                ctx.apply(move |s| p.write(s, Some(Value::String(hex))), move |ok| {
                    if ok {
                        reset.set_visible(true);
                    }
                });
            });
        });
    }
    {
        let ctx = ctx.clone();
        reset.connect_clicked(move |btn| {
            let p = p.clone();
            let (button, guard, btn) = (button.clone(), guard.clone(), btn.clone());
            let v = reset_val.clone();
            ctx.apply(move |s| p.write(s, v), move |ok| {
                if ok {
                    guard.set(true);
                    button.set_rgba(&default_rgba);
                    guard.set(false);
                    btn.set_visible(false);
                }
            });
        });
    }
    row
}

/// String enum where the first option is niri's default: choosing it removes
/// the node, and an explicit default value in the config also selects it.
pub fn enum_combo(ctx: &Ctx, title: &str, subtitle: &str, p: Path, options: &[(&str, &str)]) -> adw::ComboRow {
    let name = p.last().cloned().unwrap_or_default();
    let current = ctx.store.borrow().string(&refs(&p));
    let selected = current.as_deref().and_then(|c| options.iter().position(|(_, v)| *v == c)).unwrap_or(0);
    let opts = options
        .iter()
        .enumerate()
        .map(|(i, (l, v))| {
            let text = (i > 0).then(|| format!("{} {}", crate::kdl::ident(&name), Value::String(v.to_string()).to_kdl()));
            (l.to_string(), text)
        })
        .collect();
    node_combo(ctx, title, subtitle, p, opts, selected)
}
