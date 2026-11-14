pub mod fingerprint;
pub mod general;
pub mod keybinds;
pub mod keyboard;
pub mod pointer;
pub mod widgets;

use crate::config::ConfigStore;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Shared state handed to every page.
#[derive(Clone)]
pub struct Ctx {
    pub store: Rc<RefCell<ConfigStore>>,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
}

impl Ctx {
    /// Run an edit against the config, reporting failures to the user.
    pub fn apply(&self, f: impl FnOnce(&mut ConfigStore) -> Result<(), String>) -> bool {
        let res = f(&mut self.store.borrow_mut());
        match res {
            Ok(()) => true,
            Err(e) => {
                self.error("Couldn't apply setting", &e);
                false
            }
        }
    }

    pub fn error(&self, heading: &str, body: &str) {
        let dialog = adw::AlertDialog::new(Some(heading), None);
        let label = gtk::Label::builder()
            .label(body)
            .wrap(true)
            .selectable(true)
            .xalign(0.0)
            .css_classes(["monospace"])
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&label)
            .max_content_height(320)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        dialog.set_extra_child(Some(&scroller));
        dialog.add_response("ok", "OK");
        dialog.present(Some(&self.window));
    }

    pub fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::builder().title(msg).timeout(2).build());
    }
}

/// Owned config path, usable from 'static closures.
pub type Path = Rc<Vec<String>>;

pub fn path(parts: &[&str]) -> Path {
    Rc::new(parts.iter().map(|s| s.to_string()).collect())
}

pub fn refs(p: &Path) -> Vec<&str> {
    p.iter().map(String::as_str).collect()
}
