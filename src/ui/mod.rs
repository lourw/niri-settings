pub mod appearance;
pub mod displays;
pub mod fingerprint;
pub mod general;
pub mod keybinds;
pub mod keyboard;
pub mod layout;
pub mod pointer;
pub mod widgets;

use crate::config::ConfigStore;
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

type Job = Box<dyn FnOnce(&mut ConfigStore) -> Result<(), String> + Send>;
type Reply = async_channel::Sender<Result<(), String>>;

/// Runs config edits on a background thread, one at a time and in order, so
/// validating/writing never blocks the UI and rapid changes can't race.
pub struct Worker {
    tx: async_channel::Sender<(Job, Reply)>,
}

impl Worker {
    pub fn spawn(main: std::path::PathBuf) -> Self {
        let (tx, rx) = async_channel::unbounded::<(Job, Reply)>();
        std::thread::spawn(move || {
            let mut store = ConfigStore::load_from(main);
            while let Ok((job, reply)) = rx.recv_blocking() {
                let res = match &mut store {
                    Ok(s) => job(s),
                    Err(e) => Err(e.clone()),
                };
                let _ = reply.send_blocking(res);
            }
        });
        Worker { tx }
    }
}

/// Shared state handed to every page.
#[derive(Clone)]
pub struct Ctx {
    /// Main-thread copy used for reading; refreshed after every edit.
    pub store: Rc<RefCell<ConfigStore>>,
    pub worker: Rc<Worker>,
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
}

impl Ctx {
    /// Run an edit on the worker thread. `done(ok)` runs on the main thread
    /// afterwards (failures have already been reported to the user).
    pub fn apply(
        &self,
        job: impl FnOnce(&mut ConfigStore) -> Result<(), String> + Send + 'static,
        done: impl FnOnce(bool) + 'static,
    ) {
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        if self.worker.tx.send_blocking((Box::new(job), reply_tx)).is_err() {
            self.error("Couldn't apply setting", "The background worker stopped.");
            done(false);
            return;
        }
        let ctx = self.clone();
        glib::spawn_future_local(async move {
            let res = reply_rx.recv().await.unwrap_or_else(|_| Err("The background worker stopped.".into()));
            let _ = ctx.store.borrow_mut().reload();
            match res {
                Ok(()) => done(true),
                Err(e) => {
                    ctx.error("Couldn't apply setting", &e);
                    done(false);
                }
            }
        });
    }

    /// Fire-and-forget variant.
    pub fn apply_quiet(&self, job: impl FnOnce(&mut ConfigStore) -> Result<(), String> + Send + 'static) {
        self.apply(job, |_| {});
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

/// Owned config path, usable from 'static closures and the worker thread.
pub type Path = Arc<Vec<String>>;

pub fn path(parts: &[&str]) -> Path {
    Arc::new(parts.iter().map(|s| s.to_string()).collect())
}

pub fn refs(p: &Path) -> Vec<&str> {
    p.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kdl::Value;

    /// Fire many edits at the worker without waiting; they must apply in
    /// order, with none lost, against a copy of the real config.
    #[test]
    fn worker_serializes_rapid_edits() {
        let src = crate::config::home().join(".config/niri");
        if !src.join("config.kdl").exists() {
            return;
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-worker");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cfg")).unwrap();
        std::fs::copy(src.join("config.kdl"), dir.join("config.kdl")).unwrap();
        for e in std::fs::read_dir(src.join("cfg")).unwrap().flatten() {
            std::fs::copy(e.path(), dir.join("cfg").join(e.file_name())).unwrap();
        }
        let worker = Worker::spawn(dir.join("config.kdl"));
        let mut replies = Vec::new();
        for i in 0..20 {
            let (tx, rx) = async_channel::bounded(1);
            let job: Job = Box::new(move |s| {
                s.set_flag(&["input", "touchpad", "dwt"], i % 2 == 0)?;
                s.set_value(&["input", "mouse", "accel-speed"], Some(Value::Float(i as f64 / 100.0)))
            });
            worker.tx.send_blocking((job, tx)).unwrap();
            replies.push(rx);
        }
        for rx in replies {
            rx.recv_blocking().unwrap().unwrap();
        }
        let store = ConfigStore::load_from(dir.join("config.kdl")).unwrap();
        assert!(!store.flag(&["input", "touchpad", "dwt"]), "last write (i=19) turned dwt off");
        assert_eq!(store.value(&["input", "mouse", "accel-speed"]), Some(Value::Float(0.19)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
