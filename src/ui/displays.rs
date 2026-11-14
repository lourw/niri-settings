//! Displays: per-output mode, scale, rotation, position, VRR and colors.
//!
//! Connected outputs come from `niri msg -j outputs`; settings are written to
//! the matching `output "<name>" { … }` block, which is created on demand.

use super::widgets::{self, NumSpec};
use super::{Ctx, Path, path, refs};
use crate::kdl::{self, Value};
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Logical {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub scale: f64,
}

pub struct Output {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: Option<String>,
    pub modes: Vec<Mode>,
    pub vrr_supported: bool,
    /// Live logical geometry, when the output is enabled.
    pub logical: Option<Logical>,
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
                let i = |k: &str| l.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                Logical { x: i("x"), y: i("y"), w: i("width"), h: i("height"), scale: l.get("scale").and_then(|x| x.as_f64()).unwrap_or(1.0) }
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

    let keys: Vec<String> = outputs.iter().map(|o| config_key(ctx, o)).collect();
    let stack = adw::ViewStack::builder().vhomogeneous(false).build();
    let arrangement: Rc<RefCell<Option<Rc<Arrangement>>>> = Rc::default();
    let redraw: Rc<dyn Fn()> = {
        let arrangement = arrangement.clone();
        Rc::new(move || {
            if let Some(a) = arrangement.borrow().as_ref() {
                a.area.queue_draw();
            }
        })
    };
    let mut setters = HashMap::new();
    for (o, key) in outputs.iter().zip(&keys) {
        let (g, set_pos) = output_group(ctx, o, key, outputs.len() == 1, redraw.clone());
        setters.insert(key.clone(), set_pos);
        stack.add_titled(&g, Some(&o.name), &o.name).set_icon_name(Some("video-display-symbolic"));
    }

    if outputs.len() > 1 {
        let g = widgets::group("Arrangement", "Drag displays to place them next to each other");
        let arr = Arrangement::new(ctx, &outputs, &keys, setters, stack.clone());
        g.add(&arr.frame);
        page.add(&g);
        *arrangement.borrow_mut() = Some(arr.clone());
        // Redraw when the selected tab changes, so it's highlighted.
        stack.connect_visible_child_name_notify(move |_| arr.area.queue_draw());
    }
    if !outputs.is_empty() {
        let g = adw::PreferencesGroup::new();
        let tabs = gtk::Box::new(gtk::Orientation::Vertical, 18);
        if outputs.len() > 1 {
            let switcher = adw::InlineViewSwitcher::builder().stack(&stack).halign(gtk::Align::Center).build();
            tabs.append(&switcher);
        }
        tabs.append(&stack);
        g.add(&tabs);
        page.add(&g);
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

fn output_group(ctx: &Ctx, o: &Output, key: &str, only: bool, on_change: Rc<dyn Fn()>) -> (adw::PreferencesGroup, PosSetter) {
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

    let live_scale = o.logical.map(|l| l.scale).unwrap_or(1.0);
    g.add(&widgets::spin(
        ctx,
        "Scale",
        "Unset picks one automatically",
        p("scale"),
        NumSpec { min: 0.25, max: 10.0, step: 0.25, digits: 2, float: true, default: live_scale },
    ));
    g.add(&widgets::enum_combo(ctx, "Rotation", "", p("transform"), TRANSFORMS));
    let (pos, set_pos) = position_row(ctx, p("position"), o.logical.map(|l| (l.x, l.y)).unwrap_or((0, 0)), on_change);
    g.add(&pos);

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
    (g, set_pos)
}

/// Sets both position fields without writing to the config.
type PosSetter = Rc<dyn Fn(i64, i64)>;

/// `position x=… y=…`: both coordinates are always written together.
fn position_row(ctx: &Ctx, p: Path, live: (i64, i64), on_change: Rc<dyn Fn()>) -> (adw::ActionRow, PosSetter) {
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
        let (ctx, p, x, y, guard, deb, reset, on_change) =
            (ctx.clone(), p.clone(), x.clone(), y.clone(), guard.clone(), deb.clone(), reset.clone(), on_change.clone());
        b.connect_value_changed(move |_| {
            if guard.get() {
                return;
            }
            let (xv, yv) = (x.value() as i64, y.value() as i64);
            let (ctx, p, reset, on_change) = (ctx.clone(), p.clone(), reset.clone(), on_change.clone());
            deb.run(500, move || {
                ctx.apply(
                    move |s| s.set_props(&refs(&p), &[("x", Some(Value::Int(xv))), ("y", Some(Value::Int(yv)))]),
                    move |ok| {
                        if ok {
                            reset.set_visible(true);
                        }
                        on_change();
                    },
                );
            });
        });
    }
    {
        let (ctx, on_change) = (ctx.clone(), on_change.clone());
        reset.connect_clicked(move |btn| {
            let p = p.clone();
            let (btn, on_change) = (btn.clone(), on_change.clone());
            ctx.apply(move |s| s.set_node(&refs(&p), None), move |ok| {
                if ok {
                    btn.set_visible(false);
                }
                on_change();
            });
        });
    }
    let setter: PosSetter = Rc::new(move |xv, yv| {
        guard.set(true);
        x.set_value(xv as f64);
        y.set_value(yv as f64);
        guard.set(false);
        reset.set_visible(true);
    });
    (row, setter)
}

/// A display rectangle in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w - 0.5 && o.x < self.x + self.w - 0.5 && self.y < o.y + o.h - 0.5 && o.y < self.y + self.h - 0.5
    }
}

/// Where a display dropped at `r` should go: flush against the nearest edge
/// of another display, aligned to its edges when close, never overlapping.
fn snap(r: Rect, others: &[Rect]) -> (f64, f64) {
    const ALIGN: f64 = 80.0;
    if others.is_empty() {
        return (r.x.round(), r.y.round());
    }
    let align = |v: f64, lo: f64, hi: f64, size: f64, start: f64, end: f64| {
        let v = v.clamp(lo, hi);
        // Line up starts, ends or centres when within reach.
        [start, end - size, start + (end - start - size) / 2.0]
            .into_iter()
            .filter(|c| (c - v).abs() < ALIGN)
            .min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
            .unwrap_or(v)
    };
    let mut best: Option<((f64, f64), f64)> = None;
    for o in others {
        let ys = align(r.y, o.y - r.h + 1.0, o.y + o.h - 1.0, r.h, o.y, o.y + o.h);
        let xs = align(r.x, o.x - r.w + 1.0, o.x + o.w - 1.0, r.w, o.x, o.x + o.w);
        for (x, y) in [(o.x + o.w, ys), (o.x - r.w, ys), (xs, o.y + o.h), (xs, o.y - r.h)] {
            let cand = Rect { x, y, ..r };
            if others.iter().any(|q| cand.overlaps(q)) {
                continue;
            }
            let d = (x - r.x).hypot(y - r.y);
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some(((x.round(), y.round()), d));
            }
        }
    }
    best.map(|b| b.0).unwrap_or((r.x.round(), r.y.round()))
}

struct Screen {
    name: String,
    key: String,
    live: Logical,
}

#[derive(Clone, Copy)]
struct View {
    scale: f64,
    ox: f64,
    oy: f64,
}

struct Drag {
    idx: usize,
    start: (f64, f64),
    cur: (f64, f64),
    view: View,
}

/// Drag-to-arrange canvas of the enabled displays.
pub struct Arrangement {
    ctx: Ctx,
    screens: Vec<Screen>,
    pub area: gtk::DrawingArea,
    pub frame: gtk::Frame,
    setters: HashMap<String, PosSetter>,
    stack: adw::ViewStack,
    drag: RefCell<Option<Drag>>,
}

impl Arrangement {
    fn new(ctx: &Ctx, outputs: &[Output], keys: &[String], setters: HashMap<String, PosSetter>, stack: adw::ViewStack) -> Rc<Self> {
        let screens = outputs
            .iter()
            .zip(keys)
            .filter_map(|(o, k)| o.logical.map(|live| Screen { name: o.name.clone(), key: k.clone(), live }))
            .collect();
        let area = gtk::DrawingArea::builder().content_height(240).hexpand(true).build();
        let frame = gtk::Frame::builder().child(&area).css_classes(["view"]).build();
        let arr = Rc::new(Arrangement { ctx: ctx.clone(), screens, area, frame, setters, stack, drag: RefCell::new(None) });

        let weak = Rc::downgrade(&arr);
        arr.area.set_draw_func(move |area, cr, w, h| {
            if let Some(a) = weak.upgrade() {
                a.draw(area, cr, w as f64, h as f64);
            }
        });

        let gesture = gtk::GestureDrag::new();
        {
            let weak = Rc::downgrade(&arr);
            gesture.connect_drag_begin(move |_, x, y| {
                let Some(a) = weak.upgrade() else { return };
                let rects = a.rects();
                let view = a.view(&rects);
                let hit = rects.iter().enumerate().rev().find_map(|(i, r)| {
                    let r = (*r)?;
                    let (sx, sy) = (view.ox + r.x * view.scale, view.oy + r.y * view.scale);
                    (x >= sx && x <= sx + r.w * view.scale && y >= sy && y <= sy + r.h * view.scale).then_some((i, r))
                });
                *a.drag.borrow_mut() = hit.map(|(idx, r)| Drag { idx, start: (r.x, r.y), cur: (r.x, r.y), view });
            });
        }
        {
            let weak = Rc::downgrade(&arr);
            gesture.connect_drag_update(move |_, dx, dy| {
                let Some(a) = weak.upgrade() else { return };
                if let Some(d) = a.drag.borrow_mut().as_mut() {
                    d.cur = (d.start.0 + dx / d.view.scale, d.start.1 + dy / d.view.scale);
                }
                a.area.queue_draw();
            });
        }
        {
            let weak = Rc::downgrade(&arr);
            gesture.connect_drag_end(move |_, dx, dy| {
                let Some(a) = weak.upgrade() else { return };
                let Some(d) = a.drag.borrow_mut().take() else { return };
                let name = a.screens[d.idx].name.clone();
                a.stack.set_visible_child_name(&name);
                if dx.hypot(dy) < 4.0 {
                    a.area.queue_draw();
                    return;
                }
                a.drop_at(d.idx, d.cur);
            });
        }
        arr.area.add_controller(gesture);
        arr
    }

    /// Current rectangles: config positions (else live ones), sized from the
    /// live size adjusted for a configured scale. None for disabled displays.
    fn rects(&self) -> Vec<Option<Rect>> {
        let s = self.ctx.store.borrow();
        self.screens
            .iter()
            .map(|sc| {
                let base = kdl::keyed("output", &sc.key);
                if s.flag(&[&base, "off"]) {
                    return None;
                }
                let pos = s.get(&[&base, "position"]);
                let prop = |k: &str| pos.and_then(|n| n.prop(k)).and_then(Value::as_f64);
                let ratio = s
                    .value(&[&base, "scale"])
                    .and_then(|v| v.as_f64())
                    .filter(|v| *v > 0.0)
                    .map(|cfg| sc.live.scale / cfg)
                    .unwrap_or(1.0);
                Some(Rect {
                    x: prop("x").unwrap_or(sc.live.x as f64),
                    y: prop("y").unwrap_or(sc.live.y as f64),
                    w: (sc.live.w as f64 * ratio).max(1.0),
                    h: (sc.live.h as f64 * ratio).max(1.0),
                })
            })
            .collect()
    }

    fn view(&self, rects: &[Option<Rect>]) -> View {
        let (w, h) = (self.area.width() as f64, self.area.height() as f64);
        let rs: Vec<&Rect> = rects.iter().flatten().collect();
        if rs.is_empty() {
            return View { scale: 1.0, ox: 0.0, oy: 0.0 };
        }
        let minx = rs.iter().map(|r| r.x).fold(f64::INFINITY, f64::min);
        let miny = rs.iter().map(|r| r.y).fold(f64::INFINITY, f64::min);
        let maxx = rs.iter().map(|r| r.x + r.w).fold(f64::NEG_INFINITY, f64::max);
        let maxy = rs.iter().map(|r| r.y + r.h).fold(f64::NEG_INFINITY, f64::max);
        let pad = 24.0;
        let scale = ((w - 2.0 * pad) / (maxx - minx)).min((h - 2.0 * pad) / (maxy - miny)).max(0.001);
        View {
            scale,
            ox: (w - (maxx - minx) * scale) / 2.0 - minx * scale,
            oy: (h - (maxy - miny) * scale) / 2.0 - miny * scale,
        }
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &gtk::cairo::Context, _w: f64, _h: f64) {
        let mut rects = self.rects();
        let drag = self.drag.borrow();
        // Keep the view fixed while dragging so the display follows the pointer.
        let view = drag.as_ref().map(|d| d.view).unwrap_or_else(|| self.view(&rects));
        if let Some(d) = drag.as_ref() {
            if let Some(r) = rects[d.idx].as_mut() {
                r.x = d.cur.0;
                r.y = d.cur.1;
            }
        }
        let fg = area.color();
        let accent = adw::StyleManager::default().accent_color_rgba();
        let selected = self.stack.visible_child_name();
        for (i, r) in rects.iter().enumerate() {
            let Some(r) = r else { continue };
            let (x, y, w, h) = (view.ox + r.x * view.scale, view.oy + r.y * view.scale, r.w * view.scale, r.h * view.scale);
            let sel = selected.as_deref() == Some(self.screens[i].name.as_str());
            rounded(cr, x + 2.0, y + 2.0, w - 4.0, h - 4.0, 8.0);
            let (fill, alpha) = if sel { (accent, 0.85) } else { (fg, 0.15) };
            cr.set_source_rgba(fill.red() as f64, fill.green() as f64, fill.blue() as f64, alpha);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.35);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let label = &self.screens[i].name;
            cr.set_font_size(13.0);
            if let Ok(ext) = cr.text_extents(label) {
                let c = if sel { gtk::gdk::RGBA::WHITE } else { fg };
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 1.0);
                cr.move_to(x + (w - ext.width()) / 2.0 - ext.x_bearing(), y + (h - ext.height()) / 2.0 - ext.y_bearing());
                let _ = cr.show_text(label);
            }
        }
    }

    /// Snap the dragged display, then write every display's position,
    /// shifted so the arrangement starts at 0,0.
    fn drop_at(&self, idx: usize, at: (f64, f64)) {
        let mut rects = self.rects();
        let Some(r) = rects[idx] else { return };
        let others: Vec<Rect> = rects.iter().enumerate().filter(|(i, _)| *i != idx).filter_map(|(_, r)| *r).collect();
        let (x, y) = snap(Rect { x: at.0, y: at.1, ..r }, &others);
        rects[idx] = Some(Rect { x, y, ..r });
        let minx = rects.iter().flatten().map(|r| r.x).fold(f64::INFINITY, f64::min);
        let miny = rects.iter().flatten().map(|r| r.y).fold(f64::INFINITY, f64::min);
        let placed: Vec<(String, i64, i64)> = self
            .screens
            .iter()
            .zip(&rects)
            .filter_map(|(sc, r)| r.map(|r| (sc.key.clone(), (r.x - minx).round() as i64, (r.y - miny).round() as i64)))
            .collect();
        let edits: Vec<(Vec<String>, String)> = placed
            .iter()
            .map(|(k, x, y)| (vec![kdl::keyed("output", k), "position".into()], format!("position x={x} y={y}")))
            .collect();
        let setters: Vec<(PosSetter, i64, i64)> =
            placed.iter().filter_map(|(k, x, y)| self.setters.get(k).map(|s| (s.clone(), *x, *y))).collect();
        let area = self.area.clone();
        self.ctx.apply(
            move |s| {
                let paths: Vec<Vec<&str>> = edits.iter().map(|(p, _)| p.iter().map(String::as_str).collect()).collect();
                let e: Vec<(&[&str], Option<String>)> =
                    paths.iter().zip(&edits).map(|(p, (_, t))| (p.as_slice(), Some(t.clone()))).collect();
                s.set_nodes(&e)
            },
            move |ok| {
                if ok {
                    for (set, x, y) in &setters {
                        set(*x, *y);
                    }
                }
                area.queue_draw();
            },
        );
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::PI;
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_flush_and_aligned() {
        let a = Rect { x: 0.0, y: 0.0, w: 1440.0, h: 960.0 };
        // Dropped slightly right of and overlapping a: goes flush right, top-aligned.
        let b = Rect { x: 1300.0, y: 40.0, w: 2560.0, h: 1440.0 };
        assert_eq!(snap(b, &[a]), (1440.0, 0.0));
        // Dropped above: goes flush on top, left edges aligned.
        let c = Rect { x: 30.0, y: -1500.0, w: 2560.0, h: 1440.0 };
        assert_eq!(snap(c, &[a]), (0.0, -1440.0));
        // Far from any edge alignment: keeps its offset along the edge.
        let d = Rect { x: -2560.0, y: 400.0, w: 2560.0, h: 1440.0 };
        assert_eq!(snap(d, &[a]), (-2560.0, 400.0));
    }

    #[test]
    fn parses_niri_outputs_json() {
        let json = r#"{"eDP-1":{"name":"eDP-1","make":"BOE","model":"X","serial":null,"modes":[
            {"width":2880,"height":1920,"refresh_rate":120000,"is_preferred":true}],
            "current_mode":0,"vrr_supported":true,"logical":{"x":0,"y":0,"width":1440,"height":960,"scale":2.0}}}"#;
        let o = parse_outputs(json);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].modes[0].kdl(), "2880x1920@120.000");
        assert_eq!(o[0].logical, Some(Logical { x: 0, y: 0, w: 1440, h: 960, scale: 2.0 }));
        assert!(o[0].vrr_supported);
    }
}
