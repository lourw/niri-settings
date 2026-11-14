//! Startup applications: `spawn-at-startup` / `spawn-sh-at-startup` entries.

use super::Ctx;
use super::widgets;
use crate::config::Item;
use crate::kdl;
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

const SPAWN: &str = "spawn-at-startup";
const SPAWN_SH: &str = "spawn-sh-at-startup";

/// How an entry reads as a single command line.
fn command_text(item: &Item) -> String {
    let args: Vec<String> = item.node.args().filter_map(|v| v.as_str().map(String::from)).collect();
    if item.node.name == SPAWN_SH {
        args.join(" ")
    } else {
        shlex::try_join(args.iter().map(String::as_str)).unwrap_or_else(|_| args.join(" "))
    }
}

/// Node text for a command line: plain programs use `spawn-at-startup` with
/// separate arguments; anything needing a shell uses `spawn-sh-at-startup`.
pub fn node_for(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return None;
    }
    let shelly = cmd.chars().any(|c| "|&;<>()$`*?[]{}~!#\n".contains(c));
    match shlex::split(cmd).filter(|a| !a.is_empty() && !shelly) {
        Some(args) => {
            let quoted: Vec<String> = args.iter().map(|a| kdl::quote(a)).collect();
            Some(format!("{SPAWN} {}", quoted.join(" ")))
        }
        None => Some(format!("{SPAWN_SH} {}", kdl::quote(cmd))),
    }
}

/// An app's command line with desktop-entry field codes (%U, %f…) removed.
fn app_command(app: &gio::AppInfo) -> Option<String> {
    let line = app.commandline()?;
    let line = line.to_string_lossy();
    let args: Vec<String> = shlex::split(&line)?
        .into_iter()
        .filter(|a| !(a.len() == 2 && a.starts_with('%')))
        .collect();
    shlex::try_join(args.iter().map(String::as_str)).ok()
}

struct State {
    ctx: Ctx,
    page: adw::PreferencesPage,
    group: RefCell<Option<adw::PreferencesGroup>>,
    apps: Vec<gio::AppInfo>,
}

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let mut apps: Vec<gio::AppInfo> = gio::AppInfo::all().into_iter().filter(|a| a.should_show()).collect();
    apps.sort_by_key(|a| a.display_name().to_lowercase());
    let state = Rc::new(State { ctx: ctx.clone(), page: page.clone(), group: RefCell::new(None), apps });
    state.rebuild();
    page
}

impl State {
    fn rebuild(self: &Rc<Self>) {
        // The old list is removed only after the new one is added, so the
        // page never shrinks and the scroll position is kept.
        let old = self.group.borrow_mut().take();
        let g = widgets::group(
            "Startup applications",
            "Started once when niri launches. Changes take effect the next time you log in.",
        );
        let items: Vec<Item> = {
            let s = self.ctx.store.borrow();
            let mut v = s.items(SPAWN);
            v.extend(s.items(SPAWN_SH));
            // Keep config order within each file.
            v.sort_by(|a, b| (&a.file, a.node.start).cmp(&(&b.file, b.node.start)));
            v
        };
        for item in items {
            g.add(&self.item_row(item));
        }
        // Add from installed apps, searched inline.
        let cmds: Vec<String>;
        let items: Vec<widgets::PickItem>;
        (cmds, items) = self
            .apps
            .iter()
            .filter_map(|a| {
                let cmd = app_command(a)?;
                let item = widgets::PickItem {
                    title: a.display_name().to_string(),
                    subtitle: cmd.clone(),
                    icon: a.icon(),
                    hay: format!("{} {}", a.display_name(), a.id().unwrap_or_default()).to_lowercase(),
                };
                Some((cmd, item))
            })
            .unzip();
        let st = self.clone();
        let on_pick = Rc::new(move |i: usize| st.add(&cmds[i]));
        g.add(&widgets::search_picker("Add Application…", "list-add-symbolic", Rc::new(items), on_pick));
        // Or type a command.
        let add_cmd = adw::EntryRow::builder().title("Add a command").show_apply_button(true).build();
        add_cmd.add_prefix(&gtk::Image::from_icon_name("utilities-terminal-symbolic"));
        add_cmd.set_tooltip_text(Some("Shell syntax such as pipes, && or $VARS runs through sh -c"));
        {
            let st = self.clone();
            add_cmd.connect_apply(move |e| st.add(&e.text()));
        }
        g.add(&add_cmd);
        self.page.add(&g);
        if let Some(old) = old {
            self.page.remove(&old);
        }
        *self.group.borrow_mut() = Some(g);
    }

    /// The installed app whose executable this command runs, if any.
    fn app_for(&self, cmd: &str) -> Option<&gio::AppInfo> {
        let first = shlex::split(cmd)?.into_iter().next()?;
        let base = std::path::Path::new(&first).file_name()?.to_string_lossy().to_string();
        self.apps.iter().find(|a| {
            app_command(a).as_deref() == Some(cmd)
                || a.executable().file_name().is_some_and(|e| e.to_string_lossy() == base)
        })
    }

    fn add(self: &Rc<Self>, cmd: &str) {
        let Some(text) = node_for(cmd) else { return };
        let st = self.clone();
        self.ctx.apply(move |s| s.add_item(&text), move |_| st.rebuild());
    }

    /// An entry as an expander holding its editable command line.
    fn item_row(self: &Rc<Self>, item: Item) -> adw::ExpanderRow {
        let cmd = command_text(&item);
        let row = adw::ExpanderRow::builder().build();
        match self.app_for(&cmd) {
            Some(app) => {
                row.set_title(&glib::markup_escape_text(&app.display_name()));
                row.set_subtitle(&glib::markup_escape_text(&cmd));
                if let Some(icon) = app.icon() {
                    let img = gtk::Image::from_gicon(&icon);
                    img.set_pixel_size(32);
                    row.add_prefix(&img);
                }
            }
            None => {
                row.set_title(&glib::markup_escape_text(&cmd));
                row.set_title_lines(2);
                if item.node.name == SPAWN_SH {
                    row.set_subtitle("Shell command");
                }
                let img = gtk::Image::from_icon_name("utilities-terminal-symbolic");
                img.set_pixel_size(32);
                row.add_prefix(&img);
            }
        }
        let st = self.clone();
        widgets::lazy_rows(&row, move |row| {
            let entry = adw::EntryRow::builder().title("Command").text(&cmd).show_apply_button(true).build();
            entry.set_tooltip_text(Some("Shell syntax such as pipes, && or $VARS runs through sh -c"));
            {
                let (st, item) = (st.clone(), item.clone());
                entry.connect_apply(move |e| {
                    let Some(text) = node_for(&e.text()) else { return };
                    let item = item.clone();
                    let st2 = st.clone();
                    st.ctx.apply(move |s| s.edit_item(&item, Some(text)), move |_| st2.rebuild());
                });
            }
            row.add_row(&entry);
            let rm = adw::ButtonRow::builder().title("Remove").css_classes(["destructive-action"]).build();
            rm.connect_activated(move |_| {
                let item = item.clone();
                let st2 = st.clone();
                st.ctx.apply(move |s| s.edit_item(&item, None), move |_| st2.rebuild());
            });
            row.add_row(&rm);
        });
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_spawn_variant() {
        assert_eq!(node_for("noctalia").unwrap(), "spawn-at-startup \"noctalia\"");
        assert_eq!(node_for("foo --x 'a b'").unwrap(), "spawn-at-startup \"foo\" \"--x\" \"a b\"");
        assert_eq!(node_for("foo | bar").unwrap(), "spawn-sh-at-startup \"foo | bar\"");
        assert_eq!(node_for("echo $HOME").unwrap(), "spawn-sh-at-startup \"echo $HOME\"");
        assert_eq!(node_for("  "), None);
    }
}
