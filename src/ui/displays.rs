//! Displays: per-output mode, scale, rotation, position, VRR and colors.
//!
//! Connected outputs come from `niri msg -j outputs`; settings are written to
//! the matching `output "<name>" { … }` block, which is created on demand.

use super::widgets::{self, NumSpec};
use super::{Ctx, Path, path, refs};
use crate::kdl::{self, Value};
use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub struct Mode {
    pub width: u64,
    pub height: u64,
    /// mHz, as niri reports it.
    pub refresh: u64,
    pub preferred: bool,
}

impl Mode {
    /// The string `mode` takes, e.g. `2880x1920@120.000`.
    pub fn kdl(&self) -> String {
        format!("{}x{}@{:.3}", self.width, self.height, self.refresh as f64 / 1000.0)
    }
}

pub struct Output {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: Option<String>,
    pub modes: Vec<Mode>,
    pub vrr_supported: bool,
    /// Logical position and scale, when the output is enabled.
    pub logical: Option<(i64, i64, f64)>,
}

impl Output {
    fn title(&self) -> String {
        let desc = format!("{} {}", self.make, self.model);
        if desc.trim().is_empty() { self.name.clone() } else { desc.trim().to_string() }
    }
}

/// Connected outputs, sorted by name. Empty if niri isn't reachable.
pub fn connected_outputs() -> Vec<Output> {
    let Ok(out) = std::process::Command::new("niri").args(["msg", "-j", "outputs"]).output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_outputs(&String::from_utf8_lossy(&out.stdout))
}

fn parse_outputs(json: &str) -> Vec<Output> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    let mut outputs: Vec<Output> = map
        .values()
        .map(|o| Output {
            name: s(o, "name").unwrap_or_default(),
            make: s(o, "make").unwrap_or_default(),
            model: s(o, "model").unwrap_or_default(),
            serial: s(o, "serial"),
            modes: o
                .get("modes")
                .and_then(|m| m.as_array())
                .map(|ms| {
                    ms.iter()
                        .map(|m| Mode {
                            width: m.get("width").and_then(|x| x.as_u64()).unwrap_or(0),
                            height: m.get("height").and_then(|x| x.as_u64()).unwrap_or(0),
                            refresh: m.get("refresh_rate").and_then(|x| x.as_u64()).unwrap_or(0),
                            preferred: m.get("is_preferred").and_then(|x| x.as_bool()).unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            vrr_supported: o.get("vrr_supported").and_then(|x| x.as_bool()).unwrap_or(false),
            logical: o.get("logical").filter(|l| !l.is_null()).map(|l| {
                (
                    l.get("x").and_then(|x| x.as_i64()).unwrap_or(0),
                    l.get("y").and_then(|x| x.as_i64()).unwrap_or(0),
                    l.get("scale").and_then(|x| x.as_f64()).unwrap_or(1.0),
                )
            }),
        })
        .collect();
    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    outputs
}

/// The name an existing `output` block uses for this output (connector name,
/// or "make model serial"), else the connector name.
fn config_key(ctx: &Ctx, o: &Output) -> String {
    let desc = format!("{} {} {}", o.make, o.model, o.serial.as_deref().unwrap_or("Unknown"));
    ctx.store
        .borrow()
        .items("output")
        .iter()
        .filter_map(|i| i.node.arg(0).and_then(Value::as_str).map(String::from))
        .find(|a| a.eq_ignore_ascii_case(&o.name) || a.eq_ignore_ascii_case(&desc))
        .unwrap_or_else(|| o.name.clone())
}

const TRANSFORMS: &[(&str, &str)] = &[
    ("Normal", "normal"),
    ("90°", "90"),
    ("180°", "180"),
    ("270°", "270"),
    ("Flipped", "flipped"),
    ("Flipped, 90°", "flipped-90"),
    ("Flipped, 180°", "flipped-180"),
    ("Flipped, 270°", "flipped-270"),
];

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let outputs = connected_outputs();

    if outputs.is_empty() {
        let g = widgets::group(
            "No displays found",
            "Couldn't ask niri for connected displays (is niri running?). Displays already in your config are listed below.",
        );
        page.add(&g);
    }

    let mut keys = Vec::new();
    for o in &outputs {
        let key = config_key(ctx, o);
        page.add(&output_group(ctx, o, &key, outputs.len() == 1));
        keys.push(key);
    }

    // Configured but not connected.
    let others: Vec<_> = ctx
        .store
        .borrow()
        .items("output")
        .into_iter()
        .filter(|i| {
            let a = i.node.arg(0).and_then(Value::as_str).unwrap_or("");
            !keys.iter().any(|k| k.eq_ignore_ascii_case(a))
        })
        .collect();
    if !others.is_empty() {
        let g = widgets::group("Other configured displays", "Settings for displays that aren't connected right now");
        for item in others {
            let name = item.node.arg(0).and_then(Value::as_str).unwrap_or("?").to_string();
            let row = adw::ActionRow::builder().title(glib_escape(&name)).build();
            let rm = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Remove these settings")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let (ctx2, row2, g2) = (ctx.clone(), row.clone(), g.clone());
            rm.connect_clicked(move |_| {
                let item = item.clone();
                let (row2, g2) = (row2.clone(), g2.clone());
                ctx2.apply(move |s| s.edit_item(&item, None), move |ok| {
                    if ok {
                        g2.remove(&row2);
                    }
                });
            });
            row.add_suffix(&rm);
            g.add(&row);
        }
        page.add(&g);
    }
    page
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

fn output_group(ctx: &Ctx, o: &Output, key: &str, only: bool) -> adw::PreferencesGroup {
    let base = kdl::keyed("output", key);
    let p = |leaf: &str| path(&[&base, leaf]);
    let g = widgets::group(&o.title(), &o.name);

    let enabled = widgets::off_switch(ctx, "Enabled", "", p("off"));
    if only && enabled.is_active() {
        enabled.set_sensitive(false);
        enabled.set_subtitle("This is your only display");
    }
    g.add(&enabled);

    // Mode: preferred first, then by size and refresh rate.
    let mut modes: Vec<&Mode> = o.modes.iter().collect();
    modes.sort_by(|a, b| (b.width * b.height, b.refresh).cmp(&(a.width * a.height, a.refresh)));
    let labels: Vec<(String, String)> = modes
        .iter()
        .map(|m| {
            let star = if m.preferred { " (preferred)" } else { "" };
            (format!("{}×{} @ {:.2} Hz{star}", m.width, m.height, m.refresh as f64 / 1000.0), m.kdl())
        })
        .collect();
    let mut opts: Vec<(&str, Option<&str>)> = vec![("Automatic", None)];
    opts.extend(labels.iter().map(|(l, v)| (l.as_str(), Some(v.as_str()))));
    g.add(&widgets::combo(ctx, "Resolution & refresh rate", "", p("mode"), &opts));

    let live_scale = o.logical.map(|l| l.2).unwrap_or(1.0);
    g.add(&widgets::spin(
        ctx,
        "Scale",
        "Unset picks one automatically",
        p("scale"),
        NumSpec { min: 0.25, max: 10.0, step: 0.25, digits: 2, float: true, default: live_scale },
    ));
    g.add(&widgets::enum_combo(ctx, "Rotation", "", p("transform"), TRANSFORMS));
    g.add(&position_row(ctx, p("position"), o.logical.map(|l| (l.0, l.1)).unwrap_or((0, 0))));

    if o.vrr_supported || ctx.store.borrow().get(&refs(&p("variable-refresh-rate"))).is_some() {
        let current = ctx.store.borrow().get(&refs(&p("variable-refresh-rate"))).map(|n| {
            n.prop("on-demand").and_then(Value::as_bool).unwrap_or(false)
        });
        let selected = match current {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        };
        g.add(&widgets::node_combo(
            ctx,
            "Variable refresh rate",
            "",
            p("variable-refresh-rate"),
            vec![
                ("Off".into(), None),
                ("Always on".into(), Some("variable-refresh-rate".into())),
                ("On demand (window rules)".into(), Some("variable-refresh-rate on-demand=true".into())),
            ],
            selected,
        ));
    }
    g.add(&widgets::switch(
        ctx,
        "Focus at startup",
        "Put focus on this display when niri starts",
        p("focus-at-startup"),
    ));
    g.add(&widgets::color(
        ctx,
        "Background color",
        "Overrides the layout background on this display",
        p("background-color"),
        "#262626",
    ));
    g.add(&widgets::color(ctx, "Backdrop color", "Behind workspaces in the overview", p("backdrop-color"), "#262626"));
    g
}

/// `position x=… y=…`: both coordinates are always written together.
fn position_row(ctx: &Ctx, p: Path, live: (i64, i64)) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title("Position")
        .subtitle("Top-left corner in the global space (logical pixels)")
        .build();
    let node = ctx.store.borrow().get(&refs(&p)).cloned();
    let read = |k: &str| node.as_ref().and_then(|n| n.prop(k)).and_then(Value::as_f64);
    let set = node.is_some();
    let mk = |v: f64| {
        let b = gtk::SpinButton::with_range(-32768.0, 32768.0, 1.0);
        b.set_value(v);
        b.set_width_chars(5);
        b.set_valign(gtk::Align::Center);
        b
    };
    let x = mk(read("x").unwrap_or(live.0 as f64));
    let y = mk(read("y").unwrap_or(live.1 as f64));
    row.add_suffix(&gtk::Label::new(Some("x")));
    row.add_suffix(&x);
    row.add_suffix(&gtk::Label::new(Some("y")));
    row.add_suffix(&y);
    let reset = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Arrange automatically")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .visible(set)
        .build();
    row.add_suffix(&reset);

    let guard = Rc::new(Cell::new(false));
    let deb = widgets::Debouncer::default();
    for b in [&x, &y] {
        let (ctx, p, x, y, guard, deb, reset) =
            (ctx.clone(), p.clone(), x.clone(), y.clone(), guard.clone(), deb.clone(), reset.clone());
        b.connect_value_changed(move |_| {
            if guard.get() {
                return;
            }
            let (xv, yv) = (x.value() as i64, y.value() as i64);
            let (ctx, p, reset) = (ctx.clone(), p.clone(), reset.clone());
            deb.run(500, move || {
                ctx.apply(
                    move |s| s.set_props(&refs(&p), &[("x", Some(Value::Int(xv))), ("y", Some(Value::Int(yv)))]),
                    move |ok| {
                        if ok {
                            reset.set_visible(true);
                        }
                    },
                );
            });
        });
    }
    {
        let ctx = ctx.clone();
        reset.connect_clicked(move |btn| {
            let p = p.clone();
            let btn = btn.clone();
            ctx.apply(move |s| s.set_node(&refs(&p), None), move |ok| {
                if ok {
                    btn.set_visible(false);
                }
            });
        });
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_niri_outputs_json() {
        let json = r#"{"eDP-1":{"name":"eDP-1","make":"BOE","model":"X","serial":null,"modes":[
            {"width":2880,"height":1920,"refresh_rate":120000,"is_preferred":true}],
            "current_mode":0,"vrr_supported":true,"logical":{"x":0,"y":0,"scale":2.0}}}"#;
        let o = parse_outputs(json);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].modes[0].kdl(), "2880x1920@120.000");
        assert_eq!(o[0].logical, Some((0, 0, 2.0)));
        assert!(o[0].vrr_supported);
    }
}
