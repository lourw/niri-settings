//! Named workspaces: `workspace "name" { open-on-output "…"; }`.
//!
//! Named workspaces always exist, in config order, before any dynamic ones.

use super::{Ctx, path, widgets};
use crate::config::Item;
use crate::kdl::{self, Value};
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

const NAME: &str = "workspace";

fn ws_name(item: &Item) -> String {
    item.node.arg(0).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The item's text with its name replaced, keeping everything after it.
fn renamed(item: &Item, new: &str) -> Option<String> {
    let doc = kdl::Document::parse(item.src.clone()).ok()?;
    let node = doc.nodes.first()?;
    let rest = node.children.as_ref().map(|c| format!(" {}", &doc.text[c.open..=c.close])).unwrap_or_default();
    Some(format!("{NAME} {}{rest}", kdl::quote(new)))
}

struct State {
    ctx: Ctx,
    page: adw::PreferencesPage,
    group: RefCell<Option<adw::PreferencesGroup>>,
    outputs: Vec<String>,
}

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let outputs = super::displays::connected_outputs().into_iter().map(|o| o.name).collect();
    let st = Rc::new(State { ctx: ctx.clone(), page: page.clone(), group: RefCell::new(None), outputs });
    st.rebuild();
    page
}

impl State {
    fn rebuild(self: &Rc<Self>) {
        if let Some(g) = self.group.borrow_mut().take() {
            self.page.remove(&g);
        }
        let g = widgets::group(
            "Named workspaces",
            "Always present, in this order, above dynamic workspaces. Use their names in window rules and shortcuts.",
        );
        let items = self.ctx.store.borrow().items(NAME);
        for (i, item) in items.iter().enumerate() {
            let prev = i.checked_sub(1).map(|j| &items[j]).filter(|p| p.file == item.file);
            let next = items.get(i + 1).filter(|n| n.file == item.file);
            g.add(&self.row(item, prev, next));
        }
        let add = adw::ButtonRow::builder().title("Add Workspace…").start_icon_name("list-add-symbolic").build();
        let st = self.clone();
        add.connect_activated(move |_| st.add_dialog());
        g.add(&add);
        self.page.add(&g);
        *self.group.borrow_mut() = Some(g);
    }

    fn row(self: &Rc<Self>, item: &Item, prev: Option<&Item>, next: Option<&Item>) -> adw::ExpanderRow {
        let name = ws_name(item);
        let base = kdl::keyed(NAME, &name);
        let out_p = path(&[&base, "open-on-output"]);
        let current_out = self.ctx.store.borrow().string(&[&base, "open-on-output"]);
        let row = adw::ExpanderRow::builder()
            .title(glib::markup_escape_text(&name))
            .subtitle(glib::markup_escape_text(current_out.as_deref().unwrap_or("Any display")))
            .build();

        let entry = adw::EntryRow::builder().title("Name").text(&name).show_apply_button(true).build();
        {
            let (st, item) = (self.clone(), item.clone());
            entry.connect_apply(move |e| {
                let new = e.text().trim().to_string();
                if new.is_empty() || new == ws_name(&item) {
                    return;
                }
                let Some(text) = renamed(&item, &new) else { return };
                let item = item.clone();
                let st2 = st.clone();
                st.ctx.apply(move |s| s.edit_item(&item, Some(text)), move |_| st2.rebuild());
            });
        }
        row.add_row(&entry);

        let mut opts: Vec<(&str, Option<&str>)> = vec![("Any display", None)];
        opts.extend(self.outputs.iter().map(|o| (o.as_str(), Some(o.as_str()))));
        let combo = widgets::combo(&self.ctx, "Display", "Where the workspace is created", out_p, &opts);
        {
            let row = row.clone();
            combo.connect_selected_notify(move |c| {
                if let Some(s) = c.selected_item().and_downcast::<gtk::StringObject>() {
                    row.set_subtitle(&glib::markup_escape_text(&s.string()));
                }
            });
        }
        row.add_row(&combo);
        let order = adw::ActionRow::builder().title("Order").build();
        for (other, icon, tip) in [(prev, "go-up-symbolic", "Move up"), (next, "go-down-symbolic", "Move down")] {
            let b = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .sensitive(other.is_some())
                .build();
            if let Some(other) = other.cloned() {
                let (st, item) = (self.clone(), item.clone());
                b.connect_clicked(move |_| {
                    let edits = vec![(item.clone(), Some(other.src.clone())), (other.clone(), Some(item.src.clone()))];
                    let st2 = st.clone();
                    st.ctx.apply(move |s| s.edit_items(edits), move |_| st2.rebuild());
                });
            }
            order.add_suffix(&b);
        }
        row.add_row(&order);
        if item.node.children().iter().any(|c| c.name != "open-on-output") {
            row.add_row(
                &adw::ActionRow::builder()
                    .title("Other settings")
                    .subtitle("This workspace has more settings (like a layout override); they're kept as written")
                    .build(),
            );
        }
        let rm = adw::ButtonRow::builder().title("Remove Workspace").css_classes(["destructive-action"]).build();
        {
            let (st, item) = (self.clone(), item.clone());
            rm.connect_activated(move |_| {
                let item = item.clone();
                let st2 = st.clone();
                st.ctx.apply(move |s| s.edit_item(&item, None), move |_| st2.rebuild());
            });
        }
        row.add_row(&rm);
        row
    }

    fn add_dialog(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::new(Some("Add Workspace"), None);
        let entry = gtk::Entry::builder().placeholder_text("Name").activates_default(true).build();
        dialog.set_extra_child(Some(&entry));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("add", "Add");
        dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("add"));
        dialog.set_close_response("cancel");
        let st = self.clone();
        dialog.connect_response(None, move |_, resp| {
            let name = entry.text().trim().to_string();
            if resp != "add" || name.is_empty() {
                return;
            }
            let text = format!("{NAME} {}", kdl::quote(&name));
            let st2 = st.clone();
            st.ctx.apply(move |s| s.add_item(&text), move |_| st2.rebuild());
        });
        dialog.present(Some(&self.ctx.window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_keeps_body() {
        let doc = kdl::Document::parse("workspace \"a\" {\n    open-on-output \"DP-1\"\n}").unwrap();
        let n = doc.nodes[0].clone();
        let item = Item { file: "x".into(), index: 0, src: doc.text.clone(), indent: String::new(), node: n };
        assert_eq!(renamed(&item, "b").unwrap(), "workspace \"b\" {\n    open-on-output \"DP-1\"\n}");
    }

    #[test]
    fn add_swap_and_set_output() {
        let Some((mut s, dir)) = crate::config::test_copy_of_real_config("test-workspaces") else { return };
        s.add_item("workspace \"one\"").unwrap();
        s.add_item("workspace \"two\"").unwrap();
        s.set_value(&[&kdl::keyed(NAME, "one"), "open-on-output"], Some(Value::String("eDP-1".into()))).unwrap();
        let items = s.items(NAME);
        let (a, b) = (items[items.len() - 2].clone(), items[items.len() - 1].clone());
        s.edit_items(vec![(a.clone(), Some(b.src.clone())), (b.clone(), Some(a.src.clone()))]).unwrap();
        let names: Vec<String> = s.items(NAME).iter().map(ws_name).collect();
        assert_eq!(&names[names.len() - 2..], ["two", "one"]);
        assert_eq!(s.string(&[&kdl::keyed(NAME, "one"), "open-on-output"]).as_deref(), Some("eDP-1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
