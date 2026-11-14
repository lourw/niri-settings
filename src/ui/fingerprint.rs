//! Fingerprint management via fprintd's command-line tools.

use super::Ctx;
use adw::prelude::*;
use gtk::glib;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

const FINGERS: &[(&str, &str)] = &[
    ("right-index-finger", "Right index finger"),
    ("right-thumb", "Right thumb"),
    ("right-middle-finger", "Right middle finger"),
    ("right-ring-finger", "Right ring finger"),
    ("right-little-finger", "Right little finger"),
    ("left-index-finger", "Left index finger"),
    ("left-thumb", "Left thumb"),
    ("left-middle-finger", "Left middle finger"),
    ("left-ring-finger", "Left ring finger"),
    ("left-little-finger", "Left little finger"),
];

fn finger_label(f: &str) -> String {
    FINGERS.iter().find(|(id, _)| *id == f).map(|(_, l)| l.to_string()).unwrap_or_else(|| f.to_string())
}

fn user() -> String {
    std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| {
        Command::new("id")
            .arg("-un")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
}

#[derive(Debug, Default, Clone)]
struct Status {
    available: bool,
    device: Option<String>,
    fingers: Vec<String>,
    error: Option<String>,
}

fn query_status() -> Status {
    let out = match Command::new("fprintd-list").arg(user()).output() {
        Ok(o) => o,
        Err(_) => {
            return Status { error: Some("fprintd is not installed (fprintd-list not found).".into()), ..Default::default() };
        }
    };
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    parse_status(&text)
}

fn parse_status(text: &str) -> Status {
    let mut st = Status { available: true, ..Default::default() };
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("found ") {
            if rest.starts_with('0') {
                st.available = false;
            }
        } else if let Some(rest) = l.strip_prefix("Fingerprints for user ") {
            // "Fingerprints for user bob on Goodix MOC Fingerprint Sensor (press):"
            if let Some((_, dev)) = rest.split_once(" on ") {
                st.device = Some(dev.trim_end_matches(':').to_string());
            }
        } else if l.contains("has no fingers enrolled for ") {
            st.device = l.split("enrolled for ").nth(1).map(|d| d.trim_end_matches('.').to_string());
        } else if let Some(rest) = l.strip_prefix("- #") {
            if let Some((_, f)) = rest.split_once(": ") {
                st.fingers.push(f.trim().to_string());
            }
        } else if l.contains("No devices available") || l.contains("No devices found") {
            st.available = false;
        } else if l.starts_with("Impossible") || l.contains("failed") || l.contains("Failed") {
            st.error.get_or_insert_with(|| l.to_string());
        }
    }
    st
}

/// Which PAM services allow fingerprint authentication.
fn pam_services() -> Vec<(String, String, bool)> {
    let known = [
        ("sudo", "sudo"),
        ("su", "su"),
        ("polkit-1", "Admin prompts (polkit)"),
        ("login", "Console login"),
        ("system-local-login", "Local login"),
        ("system-auth", "System auth (shared by many)"),
        ("sddm", "SDDM login screen"),
        ("gdm-password", "GDM login screen"),
        ("greetd", "greetd login"),
        ("hyprlock", "hyprlock"),
        ("swaylock", "swaylock"),
    ];
    let mut out = Vec::new();
    let mut listed = std::collections::HashSet::new();
    let check = |name: &str| -> Option<bool> {
        let text = std::fs::read_to_string(format!("/etc/pam.d/{name}")).ok()?;
        Some(text.lines().any(|l| {
            let l = l.trim_start();
            !l.starts_with('#') && l.contains("pam_fprintd.so")
        }))
    };
    for (svc, label) in known {
        if let Some(on) = check(svc) {
            out.push((svc.to_string(), label.to_string(), on));
            listed.insert(svc.to_string());
        }
    }
    // Any other service that mentions fprintd.
    if let Ok(rd) = std::fs::read_dir("/etc/pam.d") {
        let mut extra: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !listed.contains(n))
            .collect();
        extra.sort();
        for svc in extra {
            if check(&svc) == Some(true) {
                out.push((svc.clone(), svc.clone(), true));
            }
        }
    }
    out
}

pub fn page(ctx: &Ctx) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let device_group = adw::PreferencesGroup::builder().title("Fingerprint Reader").build();
    let fingers_group = adw::PreferencesGroup::builder()
        .title("Enrolled Fingerprints")
        .build();
    let enroll_group = adw::PreferencesGroup::builder().title("Add a Fingerprint").build();
    let pam_group = adw::PreferencesGroup::builder()
        .title("Where Fingerprints Work")
        .description(
            "Read from /etc/pam.d. To enable a service, add “auth sufficient pam_fprintd.so” \
             near the top of its file (requires root). Your lock screen uses its own PAM service.",
        )
        .build();
    page.add(&device_group);
    page.add(&fingers_group);
    page.add(&enroll_group);
    page.add(&pam_group);

    let state = Rc::new(FpPage {
        ctx: ctx.clone(),
        device_group,
        fingers_group,
        enroll_group,
        rows: std::cell::RefCell::new(Vec::new()),
    });
    state.refresh();

    for (_, label, on) in pam_services() {
        let row = adw::ActionRow::builder().title(label).build();
        let tag = gtk::Label::builder()
            .label(if on { "Enabled" } else { "Password only" })
            .css_classes(if on { vec!["success"] } else { vec!["dim-label"] })
            .build();
        row.add_suffix(&tag);
        pam_group.add(&row);
    }
    page
}

struct FpPage {
    ctx: Ctx,
    device_group: adw::PreferencesGroup,
    fingers_group: adw::PreferencesGroup,
    enroll_group: adw::PreferencesGroup,
    rows: std::cell::RefCell<Vec<(adw::PreferencesGroup, gtk::Widget)>>,
}

impl FpPage {
    fn add(&self, g: &adw::PreferencesGroup, w: &impl IsA<gtk::Widget>) {
        g.add(w);
        self.rows.borrow_mut().push((g.clone(), w.clone().upcast()));
    }

    /// Query fprintd in the background and rebuild the rows.
    fn refresh(self: &Rc<Self>) {
        for (g, w) in self.rows.borrow_mut().drain(..) {
            g.remove(&w);
        }
        let spinner = adw::ActionRow::builder().title("Checking for a fingerprint reader…").build();
        spinner.add_prefix(&adw::Spinner::new());
        self.add(&self.device_group, &spinner);

        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(query_status());
        });
        let this = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(st) = rx.recv().await {
                this.populate(st);
            }
        });
    }

    fn populate(self: &Rc<Self>, st: Status) {
        for (g, w) in self.rows.borrow_mut().drain(..) {
            g.remove(&w);
        }
        let usable = st.available && st.error.is_none();
        let dev_row = adw::ActionRow::builder()
            .title(st.device.clone().unwrap_or_else(|| {
                if st.available { "Fingerprint reader".into() } else { "No fingerprint reader found".into() }
            }))
            .build();
        dev_row.add_prefix(&gtk::Image::from_icon_name("auth-fingerprint-symbolic"));
        if let Some(e) = &st.error {
            dev_row.set_subtitle(&glib::markup_escape_text(e));
        } else if st.available {
            dev_row.set_subtitle("Managed by fprintd");
        }
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        {
            let this = self.clone();
            refresh.connect_clicked(move |_| this.refresh());
        }
        dev_row.add_suffix(&refresh);
        self.add(&self.device_group, &dev_row);

        self.fingers_group.set_visible(usable);
        self.enroll_group.set_visible(usable);
        if !usable {
            return;
        }

        if st.fingers.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No fingerprints enrolled")
                .subtitle("Add one below to unlock with your finger")
                .build();
            self.add(&self.fingers_group, &row);
        }
        for f in &st.fingers {
            let row = adw::ActionRow::builder().title(finger_label(f)).subtitle(f).build();
            let del = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Delete fingerprint")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let this = self.clone();
            let finger = f.clone();
            del.connect_clicked(move |_| this.confirm_delete(Some(finger.clone())));
            row.add_suffix(&del);
            self.add(&self.fingers_group, &row);
        }
        if !st.fingers.is_empty() {
            let test = adw::ButtonRow::builder()
                .title("Test Fingerprint…")
                .start_icon_name("auth-fingerprint-symbolic")
                .build();
            let this = self.clone();
            test.connect_activated(move |_| this.run_dialog(Mode::Verify));
            self.add(&self.fingers_group, &test);
            if st.fingers.len() > 1 {
                let del_all = adw::ButtonRow::builder()
                    .title("Delete All Fingerprints")
                    .css_classes(["destructive-action"])
                    .build();
                let this = self.clone();
                del_all.connect_activated(move |_| this.confirm_delete(None));
                self.add(&self.fingers_group, &del_all);
            }
        }

        let available: Vec<&(&str, &str)> = FINGERS.iter().filter(|(id, _)| !st.fingers.iter().any(|f| f == id)).collect();
        if available.is_empty() {
            let row = adw::ActionRow::builder().title("All fingers are enrolled").build();
            self.add(&self.enroll_group, &row);
            return;
        }
        let model = gtk::StringList::new(&available.iter().map(|(_, l)| *l).collect::<Vec<_>>());
        let pick = adw::ComboRow::builder().title("Finger").model(&model).build();
        super::widgets::unclip(&pick);
        self.add(&self.enroll_group, &pick);
        let enroll = adw::ButtonRow::builder()
            .title("Enroll Fingerprint…")
            .start_icon_name("list-add-symbolic")
            .build();
        let ids: Vec<String> = available.iter().map(|(id, _)| id.to_string()).collect();
        let this = self.clone();
        enroll.connect_activated(move |_| {
            if let Some(id) = ids.get(pick.selected() as usize) {
                this.run_dialog(Mode::Enroll(id.clone()));
            }
        });
        self.add(&self.enroll_group, &enroll);
    }

    fn confirm_delete(self: &Rc<Self>, finger: Option<String>) {
        let body = match &finger {
            Some(f) => format!("The fingerprint for your {} will be removed.", finger_label(f).to_lowercase()),
            None => "All enrolled fingerprints will be removed.".into(),
        };
        let alert = adw::AlertDialog::new(Some("Delete fingerprint?"), Some(&body));
        alert.add_response("cancel", "Cancel");
        alert.add_response("delete", "Delete");
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        let this = self.clone();
        alert.connect_response(None, move |_, r| {
            if r != "delete" {
                return;
            }
            let mut cmd = Command::new("fprintd-delete");
            cmd.arg(user());
            if let Some(f) = &finger {
                cmd.args(["-f", f]);
            }
            let this = this.clone();
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let res = cmd.output().map_err(|e| e.to_string()).and_then(|o| {
                    if o.status.success() {
                        Ok(())
                    } else {
                        Err(format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
                    }
                });
                let _ = tx.send_blocking(res);
            });
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(())) => this.ctx.toast("Fingerprint deleted"),
                    Ok(Err(e)) => this.ctx.error("Couldn't delete fingerprint", e.trim()),
                    Err(_) => {}
                }
                this.refresh();
            });
        });
        alert.present(Some(&self.ctx.window));
    }

    fn run_dialog(self: &Rc<Self>, mode: Mode) {
        let (title, cmd_name) = match &mode {
            Mode::Enroll(_) => ("Enroll Fingerprint", "fprintd-enroll"),
            Mode::Verify => ("Test Fingerprint", "fprintd-verify"),
        };
        let dialog = adw::Dialog::builder().title(title).content_width(420).build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let status = adw::StatusPage::builder()
            .icon_name("auth-fingerprint-symbolic")
            .title(match &mode {
                Mode::Enroll(f) => format!("Place your {} on the sensor", finger_label(f).to_lowercase()),
                Mode::Verify => "Touch the sensor".to_string(),
            })
            .description("Starting…")
            .build();
        let progress = gtk::ProgressBar::builder().margin_start(24).margin_end(24).visible(false).build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&status);
        content.append(&progress);
        let done = gtk::Button::builder()
            .label("Cancel")
            .halign(gtk::Align::Center)
            .margin_bottom(24)
            .css_classes(["pill"])
            .build();
        content.append(&done);
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));

        // Line-buffer the tool's output so we see each stage as it happens.
        let mut cmd = if which("stdbuf") {
            let mut c = Command::new("stdbuf");
            c.args(["-oL", "-eL", cmd_name]);
            c
        } else {
            Command::new(cmd_name)
        };
        if let Mode::Enroll(f) = &mode {
            cmd.args(["-f", f]);
        }
        cmd.arg(user());
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());

        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = async_channel::unbounded::<Option<String>>();
        match cmd.spawn() {
            Ok(mut c) => {
                let out = c.stdout.take();
                let err = c.stderr.take();
                *child.lock().unwrap() = Some(c);
                if let Some(err) = err {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(err).lines().map_while(Result::ok) {
                            let _ = tx.send_blocking(Some(line));
                        }
                    });
                }
                let child2 = child.clone();
                std::thread::spawn(move || {
                    if let Some(out) = out {
                        for line in BufReader::new(out).lines().map_while(Result::ok) {
                            let _ = tx.send_blocking(Some(line));
                        }
                    }
                    if let Some(mut c) = child2.lock().unwrap().take() {
                        let _ = c.wait();
                    }
                    let _ = tx.send_blocking(None);
                });
            }
            Err(e) => {
                status.set_description(Some(&format!("Couldn't start {cmd_name}: {e}")));
                done.set_label("Close");
            }
        }

        let kill = {
            let child = child.clone();
            move || {
                if let Some(c) = child.lock().unwrap().as_mut() {
                    let _ = c.kill();
                }
            }
        };
        {
            let dialog = dialog.clone();
            done.connect_clicked(move |_| {
                dialog.close();
            });
        }
        {
            let kill = kill.clone();
            let this = self.clone();
            dialog.connect_closed(move |_| {
                kill();
                this.refresh();
            });
        }

        let is_enroll = matches!(mode, Mode::Enroll(_));
        glib::spawn_future_local(async move {
            let mut stages = 0u32;
            let mut finished = false;
            let mut last_error: Option<String> = None;
            while let Ok(msg) = rx.recv().await {
                let Some(line) = msg else { break };
                let l = line.trim();
                let result = l
                    .strip_prefix("Enroll result: ")
                    .or_else(|| l.strip_prefix("Verify result: "))
                    .map(|r| r.trim().trim_end_matches(" (done)").trim());
                if let Some(r) = result {
                    let (text, ok_end, fail_end) = describe_result(r);
                    if r == "enroll-stage-passed" {
                        stages += 1;
                        progress.set_visible(true);
                        // Most sensors need 5–20 touches; approach 1.0 asymptotically.
                        progress.set_fraction(1.0 - 0.85f64.powi(stages as i32));
                        status.set_description(Some(&format!("{text} ({stages} captured)")));
                    } else {
                        status.set_description(Some(text));
                    }
                    if ok_end {
                        finished = true;
                        progress.set_fraction(1.0);
                        status.set_icon_name(Some("emblem-ok-symbolic"));
                        status.set_title(if is_enroll { "Fingerprint enrolled" } else { "Fingerprint recognized" });
                        done.set_label("Done");
                        done.add_css_class("suggested-action");
                    } else if fail_end {
                        finished = true;
                        status.set_icon_name(Some("dialog-error-symbolic"));
                        status.set_title(if is_enroll { "Enrollment failed" } else { "Not recognized" });
                        done.set_label("Close");
                    }
                } else if l.contains("Failed") || l.contains("failed") || l.contains("Error") || l.contains("error") {
                    last_error = Some(l.to_string());
                }
            }
            if !finished {
                status.set_icon_name(Some("dialog-error-symbolic"));
                status.set_title("Stopped");
                status.set_description(Some(
                    &last_error.unwrap_or_else(|| "The fingerprint tool exited unexpectedly.".into()),
                ));
                done.set_label("Close");
            }
        });

        dialog.present(Some(&self.ctx.window));
    }
}

enum Mode {
    Enroll(String),
    Verify,
}

/// (message, success-final, failure-final)
fn describe_result(r: &str) -> (&'static str, bool, bool) {
    match r {
        "enroll-stage-passed" => ("Good! Lift your finger and touch again", false, false),
        "enroll-completed" => ("All done", true, false),
        "enroll-retry-scan" => ("Didn't catch that, touch again", false, false),
        "enroll-swipe-too-short" => ("Swipe was too short, try again", false, false),
        "enroll-finger-not-centered" => ("Center your finger on the sensor", false, false),
        "enroll-remove-and-retry" => ("Lift your finger and try again", false, false),
        "enroll-duplicate" => ("This finger is already enrolled", false, true),
        "enroll-data-full" => ("The sensor's storage is full", false, true),
        "enroll-disconnected" => ("The sensor was disconnected", false, true),
        "enroll-failed" => ("Enrollment failed", false, true),
        "enroll-unknown-error" => ("Unknown error", false, true),
        "verify-match" => ("Your fingerprint matched", true, false),
        "verify-no-match" => ("That fingerprint didn't match", false, true),
        "verify-retry-scan" => ("Didn't catch that, touch again", false, false),
        "verify-swipe-too-short" => ("Swipe was too short, try again", false, false),
        "verify-finger-not-centered" => ("Center your finger on the sensor", false, false),
        "verify-remove-and-retry" => ("Lift your finger and try again", false, false),
        "verify-disconnected" => ("The sensor was disconnected", false, true),
        "verify-unknown-error" => ("Unknown error", false, true),
        _ => ("Working…", false, false),
    }
}

fn which(prog: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fprintd_list() {
        let st = parse_status(
            "found 1 devices\nDevice at /net/reactivated/Fprint/Device/0\nUsing device /net/reactivated/Fprint/Device/0\nUser lourw has no fingers enrolled for Goodix MOC Fingerprint Sensor.\n",
        );
        assert!(st.available);
        assert_eq!(st.device.as_deref(), Some("Goodix MOC Fingerprint Sensor"));
        assert!(st.fingers.is_empty());
        let st = parse_status(
            "found 1 devices\nFingerprints for user bob on Goodix MOC Fingerprint Sensor (press):\n - #0: right-index-finger\n - #1: left-thumb\n",
        );
        assert_eq!(st.fingers, vec!["right-index-finger", "left-thumb"]);
        assert_eq!(st.device.as_deref(), Some("Goodix MOC Fingerprint Sensor (press)"));
    }
}
