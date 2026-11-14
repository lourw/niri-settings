//! Rows bound to a single config node. Each reads its initial state from the
//! store and writes on change.

use super::{Ctx, Path, refs};
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
        if !ctx.apply(|s| s.set_flag(&refs(&p), on)) {
            guard.set(true);
            row.set_active(!on);
            guard.set(false);
        }
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
        if !ctx.apply(|s| s.set_value(&refs(&p), v)) {
            guard.set(true);
            row.set_active(!on);
            guard.set(false);
        }
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
    let row = adw::ComboRow::builder().title(title).subtitle(subtitle).model(&model).build();
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
        if ctx.apply(|s| s.set_value(&refs(&p), v.map(Value::String))) {
            last.set(i);
        } else {
            guard.set(true);
            row.set_selected(last.get());
            guard.set(false);
        }
    });
    row
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
pub fn spin(ctx: &Ctx, title: &str, subtitle: &str, p: Path, spec: NumSpec) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(spec.min, spec.max, spec.step);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_digits(spec.digits);
    let current = ctx.store.borrow().value(&refs(&p)).and_then(|v| v.as_f64());
    row.set_value(current.unwrap_or(spec.default));

    let reset = reset_button("Reset to default");
    reset.set_visible(current.is_some());
    row.add_suffix(&reset);

    let guard = Rc::new(Cell::new(false));
    let deb = Debouncer::default();
    let default = spec.default;
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
            let val = (!(spec.default == 0.0 && row.value() == 0.0)).then(|| num_value(&spec, row.value()));
            deb.run(400, move || {
                let set = val.is_some();
                if ctx.apply(|s| s.set_value(&refs(&p), val)) {
                    reset.set_visible(set);
                }
            });
        });
    }
    {
        let ctx = ctx.clone();
        let row2 = row.clone();
        reset.connect_clicked(move |btn| {
            if ctx.apply(|s| s.set_value(&refs(&p), None)) {
                guard.set(true);
                row2.set_value(default);
                guard.set(false);
                btn.set_visible(false);
            }
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
                if ctx.apply(|s| s.set_value(&refs(&p), Some(Value::Float(v)))) {
                    reset.set_visible(true);
                }
            });
        });
    }
    {
        let ctx = ctx.clone();
        let scale = scale.clone();
        reset.connect_clicked(move |btn| {
            if ctx.apply(|s| s.set_value(&refs(&p), None)) {
                guard.set(true);
                scale.set_value(default);
                guard.set(false);
                btn.set_visible(false);
            }
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
