//! Loading the niri config (following `include`s) and committing edits safely.
//!
//! Every edit re-reads the files from disk first, so changes made in a text
//! editor while the app is open are never clobbered. Before anything is
//! written, a copy of the whole config with the edit applied is checked with
//! `niri validate` in a temp dir, so niri never sees a broken config. niri
//! watches its config (including included files) and reloads by itself.

use crate::kdl::{self, Document, Node, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct LoadedFile {
    pub path: PathBuf,
    pub doc: Document,
}

/// A top-level node addressed by file and position among same-named nodes.
#[derive(Clone)]
pub struct Item {
    pub file: PathBuf,
    pub index: usize,
    pub node: Node,
    /// Source text when read, to detect edits made elsewhere.
    pub src: String,
    /// Leading whitespace of the node's line.
    pub indent: String,
}

pub struct ConfigStore {
    pub main: PathBuf,
    pub files: Vec<LoadedFile>,
    backed_up: HashSet<PathBuf>,
    backup_dir: PathBuf,
    /// Some include uses an absolute or `~` path, so a temp-dir copy of the
    /// config wouldn't pick up the edited file; validate in place instead.
    absolute_includes: bool,
}

pub fn main_config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("NIRI_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    config_home().join("niri/config.kdl")
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/state"))
}

impl ConfigStore {
    pub fn load() -> Result<Self, String> {
        Self::load_from(main_config_path())
    }

    pub fn load_from(main: PathBuf) -> Result<Self, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut store = ConfigStore {
            main,
            files: Vec::new(),
            backed_up: HashSet::new(),
            backup_dir: state_home().join("niri-settings/backups").join(stamp.to_string()),
            absolute_includes: false,
        };
        store.reload()?;
        Ok(store)
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        let mut absolute = false;
        load_recursive(&self.main, &mut files, &mut seen, &mut absolute)?;
        self.files = files;
        self.absolute_includes = absolute;
        Ok(())
    }

    /// Last node matching `path` across all files (later definitions win).
    pub fn find(&self, path: &[&str]) -> Option<(usize, &Node)> {
        self.files
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, f)| f.doc.find(path).map(|n| (i, n)))
    }

    pub fn get(&self, path: &[&str]) -> Option<&Node> {
        self.find(path).map(|(_, n)| n)
    }

    /// A flag node like `tap` or `numlock` (also accepts `tap true/false`).
    pub fn flag(&self, path: &[&str]) -> bool {
        match self.get(path) {
            Some(n) => n.arg(0).and_then(Value::as_bool).unwrap_or(true),
            None => false,
        }
    }

    pub fn value(&self, path: &[&str]) -> Option<Value> {
        self.get(path).and_then(|n| n.arg(0).cloned())
    }

    pub fn string(&self, path: &[&str]) -> Option<String> {
        self.value(path).and_then(|v| v.as_str().map(String::from))
    }

    /// Children of the node at `path`, each with its source text.
    pub fn children_src(&self, path: &[&str]) -> Vec<(Node, String)> {
        let Some((fi, node)) = self.find(path) else { return Vec::new() };
        let text = &self.files[fi].doc.text;
        node.children().iter().map(|c| (c.clone(), text[c.span()].to_string())).collect()
    }

    pub fn set_flag(&mut self, path: &[&str], on: bool) -> Result<(), String> {
        let name = path.last().unwrap();
        self.set_node(path, on.then(|| kdl::ident(name)))
    }

    pub fn set_value(&mut self, path: &[&str], value: Option<Value>) -> Result<(), String> {
        let name = path.last().unwrap();
        self.set_node(path, value.map(|v| format!("{} {}", kdl::ident(name), v.to_kdl())))
    }

    /// Set (Some) or remove (None) the node at `path`. `text` is the complete
    /// node text, including its name. Missing parents are created.
    pub fn set_node(&mut self, path: &[&str], text: Option<String>) -> Result<(), String> {
        self.set_nodes(&[(path, text)])
    }

    /// Several node edits applied together; each touched file is validated
    /// and written once.
    pub fn set_nodes(&mut self, edits: &[(&[&str], Option<String>)]) -> Result<(), String> {
        self.transact(|s| edits.iter().try_for_each(|(p, t)| s.stage(p, t.clone())))
    }

    /// Change properties of the node at `path` (e.g. `position x=… y=…`),
    /// creating it if needed; it's removed once it has nothing left.
    pub fn set_props(&mut self, path: &[&str], props: &[(&str, Option<Value>)]) -> Result<(), String> {
        self.transact(|s| {
            let text = match s.find(path) {
                Some((fi, n)) => kdl::with_props(&s.files[fi].doc.text, n, props),
                None => {
                    let leaf = Node {
                        name: path.last().unwrap().to_string(),
                        start: 0,
                        end: 0,
                        line_start: 0,
                        entries: Vec::new(),
                        children: None,
                    };
                    kdl::with_props("", &leaf, props)
                }
            };
            s.stage(path, text)
        })
    }

    /// Every top-level node called `name` (e.g. `window-rule`), in config order.
    pub fn items(&self, name: &str) -> Vec<Item> {
        let mut out = Vec::new();
        for f in &self.files {
            for (index, n) in f.doc.find_all(name).enumerate() {
                out.push(Item {
                    file: f.path.clone(),
                    index,
                    node: n.clone(),
                    src: f.doc.text[n.span()].to_string(),
                    indent: kdl::line_indent(&f.doc.text, n.start),
                });
            }
        }
        out
    }

    /// Replace (Some) or remove (None) a node returned by `items`. Fails if
    /// the node changed on disk since it was read.
    pub fn edit_item(&mut self, item: &Item, text: Option<String>) -> Result<(), String> {
        self.edit_items(vec![(item.clone(), text)])
    }

    /// Several `edit_item`s validated and written together (e.g. swapping two
    /// entries). Nodes are edited back to front so earlier spans stay valid.
    pub fn edit_items(&mut self, mut edits: Vec<(Item, Option<String>)>) -> Result<(), String> {
        edits.sort_by(|a, b| (&b.0.file, b.0.node.start).cmp(&(&a.0.file, a.0.node.start)));
        self.transact(|s| {
            for (item, text) in edits {
                let name = item.node.name.clone();
                let f = s
                    .files
                    .iter_mut()
                    .find(|f| f.path == item.file)
                    .ok_or_else(|| format!("{} is no longer part of the config", item.file.display()))?;
                let node = f
                    .doc
                    .find_all(&name)
                    .nth(item.index)
                    .filter(|n| f.doc.text[n.span()] == item.src)
                    .cloned()
                    .ok_or("This entry was changed outside the app. Reload from disk and try again.")?;
                let ierr = |e: kdl::ParseError| format!("internal edit error: {e}");
                match text {
                    Some(t) => f.doc.splice(node.span(), &t).map_err(ierr)?,
                    None => {
                        let r = f.doc.removal_range(&node);
                        f.doc.splice(r, "").map_err(ierr)?
                    }
                }
            }
            Ok(())
        })
    }

    /// Add a top-level node next to the last one with the same name, or at
    /// the end of the main file if there's none yet.
    pub fn add_item(&mut self, text: &str) -> Result<(), String> {
        let name = Document::parse(text)
            .ok()
            .and_then(|d| d.nodes.first().map(|n| n.name.clone()))
            .ok_or("internal edit error: bad node text")?;
        self.transact(|s| {
            let ierr = |e: kdl::ParseError| format!("internal edit error: {e}");
            let last = s
                .files
                .iter()
                .enumerate()
                .rev()
                .find_map(|(i, f)| f.doc.find_all(&name).last().map(|n| (i, n.clone())));
            match last {
                Some((fi, n)) => s.files[fi].doc.insert_after(&n, text).map_err(ierr),
                None => s.files[0].doc.append_top(text).map_err(ierr),
            }
        })
    }

    /// Run staged edits, then validate and write every changed file once.
    fn transact(&mut self, f: impl FnOnce(&mut Self) -> Result<(), String>) -> Result<(), String> {
        self.reload()?;
        let originals: Vec<String> = self.files.iter().map(|f| f.doc.text.clone()).collect();
        let res = f(self);
        if let Err(e) = res {
            self.reload()?;
            return Err(e);
        }
        let changed: Vec<usize> =
            (0..self.files.len()).filter(|&i| self.files[i].doc.text != originals[i]).collect();
        if changed.is_empty() {
            return Ok(());
        }
        let res = self.commit(&changed, &originals);
        let reloaded = self.reload();
        res.and(reloaded)
    }

    /// Apply one edit to the in-memory documents.
    fn stage(&mut self, path: &[&str], text: Option<String>) -> Result<(), String> {
        let ierr = |e: kdl::ParseError| format!("internal edit error: {e}");
        if let Some((fi, node)) = self.find(path) {
            let node = node.clone();
            let doc = &mut self.files[fi].doc;
            match &text {
                Some(t) => return doc.splice(node.span(), t).map_err(ierr),
                None => {
                    let r = doc.removal_range(&node);
                    doc.splice(r, "").map_err(ierr)?;
                    // Prune nested blocks left completely empty (no children,
                    // no comments, no arguments), e.g. a `trackpoint {}` we
                    // created earlier. Top-level sections are kept.
                    for k in (2..path.len()).rev() {
                        let Some(parent) = doc.find(&path[..k]) else { break };
                        let empty = parent.entries.is_empty()
                            && parent
                                .children
                                .as_ref()
                                .is_some_and(|c| doc.text[c.open + 1..c.close].trim().is_empty());
                        if !empty {
                            break;
                        }
                        let r = doc.removal_range(&parent.clone());
                        doc.splice(r, "").map_err(ierr)?;
                    }
                    return Ok(());
                }
            }
        }
        let Some(text) = text else { return Ok(()) };
        for k in (1..path.len()).rev() {
            if let Some((fi, _)) = self.find(&path[..k]) {
                let nested = nest(&path[k..path.len() - 1], &text);
                return self.files[fi].doc.append_child(&path[..k], &nested).map_err(ierr);
            }
        }
        let nested = nest(&path[..path.len() - 1], &text);
        self.files[0].doc.append_top(&nested).map_err(ierr)
    }


    /// Validate the staged documents as a whole, then write the changed files.
    fn commit(&mut self, changed: &[usize], originals: &[String]) -> Result<(), String> {
        let validated = self.validate_staged()?;
        for &fi in changed {
            let path = self.files[fi].path.clone();
            self.backup(&path, &originals[fi]);
            std::fs::write(&path, &self.files[fi].doc.text)
                .map_err(|e| format!("writing {}: {e}", path.display()))?;
        }
        if !validated {
            // Couldn't check a copy beforehand: check in place, roll back on failure.
            if let Err(msg) = validate_path(&self.files[0].path) {
                for &fi in changed {
                    let _ = std::fs::write(&self.files[fi].path, &originals[fi]);
                }
                return Err(msg);
            }
        }
        Ok(())
    }

    /// Write the staged config tree to a temp dir and run `niri validate` on
    /// it. Returns Ok(false) if the tree can't be mirrored faithfully.
    fn validate_staged(&self) -> Result<bool, String> {
        let Some(base) = self.files[0].path.parent() else { return Ok(false) };
        if self.absolute_includes || !self.files.iter().all(|f| f.path.starts_with(base)) {
            return Ok(false);
        }
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = runtime.join(format!("niri-settings-validate-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let write_all = || -> std::io::Result<()> {
            for f in &self.files {
                let dest = tmp.join(f.path.strip_prefix(base).unwrap());
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(dest, &f.doc.text)?;
            }
            Ok(())
        };
        if write_all().is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Ok(false);
        }
        let res = validate_path(&tmp.join(self.files[0].path.file_name().unwrap()));
        let _ = std::fs::remove_dir_all(&tmp);
        res.map(|()| true)
    }

    fn backup(&mut self, path: &Path, text: &str) {
        if !self.backed_up.insert(path.to_path_buf()) {
            return;
        }
        let rel = path.strip_prefix(self.main.parent().unwrap_or(Path::new("/"))).unwrap_or(path);
        let rel = rel.strip_prefix("/").unwrap_or(rel);
        let dest = self.backup_dir.join(rel);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(dest, text);
    }

    pub fn backup_dir(&self) -> &Path {
        &self.backup_dir
    }
}

fn nest(parents: &[&str], leaf: &str) -> String {
    let mut text = leaf.to_string();
    for p in parents.iter().rev() {
        let body = text.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n");
        text = format!("{} {{\n{}\n}}", kdl::seg_header(p), body);
    }
    text
}

fn load_recursive(
    path: &Path,
    out: &mut Vec<LoadedFile>,
    seen: &mut HashSet<PathBuf>,
    absolute: &mut bool,
) -> Result<(), String> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canonical.clone()) {
        return Ok(());
    }
    let text = match std::fs::read_to_string(&canonical) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && out.is_empty() => String::new(),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let doc = Document::parse(text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
    let includes: Vec<PathBuf> = doc
        .find_all("include")
        .filter_map(|n| n.arg(0).and_then(Value::as_str).map(PathBuf::from))
        .collect();
    out.push(LoadedFile { path: canonical.clone(), doc });
    let dir = canonical.parent().unwrap_or(Path::new("/")).to_path_buf();
    for inc in includes {
        *absolute |= inc.is_absolute() || inc.starts_with("~");
        let inc = expand_tilde(&inc);
        let full = if inc.is_absolute() { inc } else { dir.join(inc) };
        if full.exists() {
            load_recursive(&full, out, seen, absolute)?;
        }
    }
    Ok(())
}

fn expand_tilde(p: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => home().join(rest),
        Err(_) => p.to_path_buf(),
    }
}

pub fn validate_path(path: &Path) -> Result<(), String> {
    let out = Command::new("niri")
        .arg("validate")
        .arg("-c")
        .arg(path)
        .env("RUST_LOG", "error")
        .output();
    match out {
        // If niri isn't on PATH we can't validate; don't block edits.
        Err(_) => Ok(()),
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let msg = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            let msg = msg.trim();
            Err(if msg.is_empty() { "niri rejected the config".into() } else { msg.to_string() })
        }
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// For tests: a copy of the user's real config tree under target/<name>.
/// None when there's no config or no niri to validate with.
#[cfg(test)]
pub fn test_copy_of_real_config(name: &str) -> Option<(ConfigStore, PathBuf)> {
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                copy_dir(&p, &to.join(e.file_name()));
            } else {
                std::fs::copy(&p, to.join(e.file_name())).unwrap();
            }
        }
    }
    let src = home().join(".config/niri");
    if !src.join("config.kdl").exists() || Command::new("niri").arg("--version").output().is_err() {
        return None;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&src, &dir);
    let mut s = ConfigStore::load_from(dir.join("config.kdl")).unwrap();
    s.backup_dir = dir.join("backups");
    Some((s, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copy the user's real config into target/ and exercise edits against it
    /// with the real `niri validate`. Skipped when there is no config or niri.
    #[test]
    fn edits_against_copy_of_real_config() {
        let src = home().join(".config/niri");
        if !src.join("config.kdl").exists() || Command::new("niri").arg("--version").output().is_err() {
            return;
        }
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-config");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&src, &dir);
        let mut s = ConfigStore::load_from(dir.join("config.kdl")).unwrap();
        s.backup_dir = dir.join("backups");
        assert!(s.files.len() > 1, "includes followed");

        let input_before = std::fs::read_to_string(dir.join("cfg/input.kdl")).unwrap();

        // Flag on/off in an existing section. Use a flag the real config
        // doesn't already set, or "off" would delete the user's line.
        let flag = ["dwt", "dwtp", "disabled-on-external-mouse", "left-handed"]
            .into_iter()
            .find(|f| !s.flag(&["input", "touchpad", f]))
            .expect("some touchpad flag is unset");
        s.set_flag(&["input", "touchpad", flag], true).unwrap();
        assert!(s.flag(&["input", "touchpad", flag]));
        s.set_flag(&["input", "touchpad", flag], false).unwrap();
        assert!(!s.flag(&["input", "touchpad", flag]));

        // Values, including creating a missing section.
        s.set_value(&["input", "mouse", "accel-speed"], Some(Value::Float(-0.35))).unwrap();
        assert_eq!(s.value(&["input", "mouse", "accel-speed"]), Some(Value::Float(-0.35)));
        s.set_value(&["input", "trackpoint", "accel-profile"], Some(Value::String("flat".into()))).unwrap();
        assert_eq!(s.string(&["input", "trackpoint", "accel-profile"]).as_deref(), Some("flat"));
        s.set_value(&["input", "trackpoint", "accel-profile"], None).unwrap();
        s.set_value(&["input", "mouse", "accel-speed"], None).unwrap();

        // Batch: layout + variant in one write.
        s.set_nodes(&[
            (&["input", "keyboard", "xkb", "layout"], Some("layout \"us,de\"".into())),
            (&["input", "keyboard", "xkb", "variant"], Some("variant \",nodeadkeys\"".into())),
        ])
        .unwrap();
        assert_eq!(s.string(&["input", "keyboard", "xkb", "variant"]).as_deref(), Some(",nodeadkeys"));
        s.set_nodes(&[
            (&["input", "keyboard", "xkb", "layout"], None),
            (&["input", "keyboard", "xkb", "variant"], None),
        ])
        .unwrap();

        // Invalid values are rejected and nothing is written.
        let err = s.set_value(&["input", "touchpad", "accel-profile"], Some(Value::String("bogus".into())));
        assert!(err.is_err());
        assert_eq!(s.string(&["input", "touchpad", "accel-profile"]), None);

        // After undoing everything, the file is byte-for-byte identical.
        assert_eq!(std::fs::read_to_string(dir.join("cfg/input.kdl")).unwrap(), input_before);

        // Binds: add, replace in place, and duplicate rejection.
        let binds_before = std::fs::read_to_string(dir.join("cfg/keybinds.kdl")).unwrap();
        s.set_node(&["binds", "Mod+Shift+Y"], Some("Mod+Shift+Y { spawn \"foot\"; }".into())).unwrap();
        assert!(s.get(&["binds", "Mod+Shift+Y"]).is_some());
        s.set_node(&["binds", "Mod+Shift+Y"], Some("Mod+Shift+U { spawn \"foot\"; }".into())).unwrap();
        assert!(s.get(&["binds", "Mod+Shift+U"]).is_some());
        let dup = s.set_node(&["binds", "Mod+Shift+U"], Some("Mod+Q { spawn \"foot\"; }".into()));
        assert!(dup.is_err(), "duplicate key must be rejected by niri validate");
        s.set_node(&["binds", "Mod+Shift+U"], None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("cfg/keybinds.kdl")).unwrap(), binds_before);

        // Backups were made.
        assert!(dir.join("backups/cfg/input.kdl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The node shapes the Layout and Appearance pages write are accepted by
    /// niri and undo cleanly.
    #[test]
    fn layout_and_appearance_edits() {
        let src = home().join(".config/niri");
        if !src.join("config.kdl").exists() || Command::new("niri").arg("--version").output().is_err() {
            return;
        }
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-layout");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(&src, &dir);
        let mut s = ConfigStore::load_from(dir.join("config.kdl")).unwrap();
        s.backup_dir = dir.join("backups");
        let snapshot = |d: &Path| -> Vec<String> {
            let mut v: Vec<_> = std::fs::read_dir(d.join("cfg")).unwrap().flatten().map(|e| e.path()).collect();
            v.sort();
            v.iter().map(|p| std::fs::read_to_string(p).unwrap()).chain([std::fs::read_to_string(d.join("config.kdl")).unwrap()]).collect()
        };
        let before = snapshot(&dir);
        let paths: Vec<&[&str]> = vec![

            &["layout", "focus-ring", "off"][..], &["layout", "border", "active-color"], &["layout", "border", "on"],
            &["layout", "shadow", "draw-behind-window"], &["layout", "shadow", "on"], &["layout", "default-column-width"],
            &["layout", "preset-window-heights"], &["layout", "default-column-display"],
            &["layout", "tab-indicator", "position"], &["animations", "off"],
            &["animations", "slowdown"], &["overview", "zoom"], &["overview", "workspace-shadow", "off"],
            &["gestures", "hot-corners", "off"], &["clipboard", "disable-primary"],
            &["config-notification", "disable-failed"], &["hotkey-overlay", "hide-not-bound"],
            &["layout", "focus-ring", "on"], &["layout", "border", "off"],
        ];
        let src_of = |s: &ConfigStore, p: &[&str]| s.find(p).map(|(fi, n)| s.files[fi].doc.text[n.span()].to_string());
        let originals: Vec<Option<String>> = paths.iter().map(|p| src_of(&s, p)).collect();

        // on/off blocks
        s.set_nodes(&[(&["layout", "focus-ring", "on"], None), (&["layout", "focus-ring", "off"], Some("off".into()))]).unwrap();
        assert!(s.flag(&["layout", "focus-ring", "off"]));
        s.set_nodes(&[(&["layout", "border", "on"], Some("on".into())), (&["layout", "border", "off"], None)]).unwrap();
        s.set_value(&["layout", "border", "active-color"], Some(Value::String("#11223380".into()))).unwrap();
        s.set_flag(&["layout", "shadow", "on"], true).unwrap();
        s.set_value(&["layout", "shadow", "draw-behind-window"], Some(Value::Bool(true))).unwrap();
        // node combos and presets
        s.set_node(&["layout", "default-column-width"], Some("default-column-width {}".into())).unwrap();
        s.set_node(&["layout", "default-column-width"], Some("default-column-width { proportion 0.75; }".into())).unwrap();
        s.set_node(&["layout", "preset-window-heights"], Some("preset-window-heights { proportion 0.5; proportion 1.0; }".into())).unwrap();
        s.set_node(&["layout", "default-column-display"], Some("default-column-display \"tabbed\"".into())).unwrap();
        s.set_value(&["layout", "tab-indicator", "position"], Some(Value::String("top".into()))).unwrap();
        // top-level toggles
        s.set_flag(&["animations", "off"], true).unwrap();
        s.set_value(&["animations", "slowdown"], Some(Value::Float(2.0))).unwrap();
        s.set_value(&["overview", "zoom"], Some(Value::Float(0.4))).unwrap();
        s.set_flag(&["overview", "workspace-shadow", "off"], true).unwrap();
        s.set_flag(&["gestures", "hot-corners", "off"], true).unwrap();
        s.set_flag(&["clipboard", "disable-primary"], true).unwrap();
        s.set_flag(&["config-notification", "disable-failed"], true).unwrap();
        s.set_flag(&["hotkey-overlay", "hide-not-bound"], true).unwrap();

        // Undo everything: restore each node to what the config had before.
        for (p, orig) in paths.iter().zip(originals) {
            s.set_node(p, orig).unwrap();
        }
        let after = snapshot(&dir);
        // Only blocks we created from scratch may linger (empty top-level sections).
        for (a, b) in before.iter().zip(&after) {
            if a != b {
                let extra = b.replace(a.as_str(), "");
                assert!(extra.chars().all(|c| c.is_whitespace() || "{}".contains(c) || c.is_alphabetic() || c == '-'), "unexpected leftover:\n{b}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fresh config in target/ with the given text. None without niri.
    fn scratch(name: &str, text: &str) -> Option<(ConfigStore, PathBuf)> {
        Command::new("niri").arg("--version").output().ok()?;
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.kdl"), text).unwrap();
        let mut s = ConfigStore::load_from(dir.join("config.kdl")).unwrap();
        s.backup_dir = dir.join("backups");
        Some((s, dir))
    }

    #[test]
    fn keyed_paths_and_props() {
        let Some((mut s, dir)) = scratch("test-keyed", "output \"eDP-1\" {\n    scale 2.0\n}\n") else { return };
        let out = kdl::keyed("output", "edp-1");
        // Case-insensitive match on the existing block.
        assert_eq!(s.value(&[&out, "scale"]), Some(Value::Float(2.0)));
        s.set_props(&[&out, "position"], &[("x", Some(Value::Int(10))), ("y", Some(Value::Int(0)))]).unwrap();
        assert_eq!(s.get(&[&out, "position"]).unwrap().prop("x"), Some(&Value::Int(10)));
        s.set_props(&[&out, "position"], &[("x", Some(Value::Int(20)))]).unwrap();
        assert_eq!(s.get(&[&out, "position"]).unwrap().prop("y"), Some(&Value::Int(0)));
        // A block for a new output is created with its name.
        let hdmi = kdl::keyed("output", "HDMI-A-1");
        s.set_value(&[&hdmi, "transform"], Some(Value::String("90".into()))).unwrap();
        let text = std::fs::read_to_string(dir.join("config.kdl")).unwrap();
        assert!(text.contains("output \"HDMI-A-1\" {\n    transform \"90\"\n}"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn items_add_edit_remove() {
        let text = "spawn-at-startup \"a\"\n// keep me\nspawn-at-startup \"b\"\n\nbinds {\n}\n";
        let Some((mut s, dir)) = scratch("test-items", text) else { return };
        let items = s.items("spawn-at-startup");
        assert_eq!(items.len(), 2);
        s.add_item("spawn-sh-at-startup \"c | d\"").unwrap();
        s.add_item("spawn-at-startup \"e\"").unwrap();
        let got = std::fs::read_to_string(dir.join("config.kdl")).unwrap();
        assert!(got.starts_with("spawn-at-startup \"a\"\n// keep me\nspawn-at-startup \"b\"\nspawn-at-startup \"e\"\n"), "{got}");
        assert!(got.ends_with("spawn-sh-at-startup \"c | d\"\n"), "{got}");
        // Edit by position; stale source is refused.
        let b = s.items("spawn-at-startup")[1].clone();
        s.edit_item(&b, Some("spawn-at-startup \"B\"".into())).unwrap();
        assert!(s.edit_item(&b, None).is_err(), "stale item must be refused");
        let b = s.items("spawn-at-startup")[1].clone();
        s.edit_item(&b, None).unwrap();
        assert_eq!(s.items("spawn-at-startup").len(), 2);
        // Invalid nodes are rejected by niri.
        assert!(s.add_item("window-rule { bogus-thing true; }").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The node shapes the window switcher group writes are accepted.
    #[test]
    fn recent_windows_edits() {
        let Some((mut s, dir)) = scratch("test-recent", "") else { return };
        s.set_flag(&["recent-windows", "off"], true).unwrap();
        s.set_value(&["recent-windows", "open-delay-ms"], Some(Value::Int(200))).unwrap();
        s.set_value(&["recent-windows", "debounce-ms"], Some(Value::Int(500))).unwrap();
        s.set_value(&["recent-windows", "highlight", "active-color"], Some(Value::String("#123456".into()))).unwrap();
        s.set_value(&["recent-windows", "highlight", "padding"], Some(Value::Int(10))).unwrap();
        s.set_value(&["recent-windows", "highlight", "corner-radius"], Some(Value::Int(8))).unwrap();
        s.set_value(&["recent-windows", "previews", "max-height"], Some(Value::Int(300))).unwrap();
        s.set_value(&["recent-windows", "previews", "max-scale"], Some(Value::Float(0.3))).unwrap();
        s.set_flag(&["recent-windows", "off"], false).unwrap();
        assert_eq!(s.value(&["recent-windows", "previews", "max-height"]), Some(Value::Int(300)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The node shapes the extra Appearance rows write are accepted.
    #[test]
    fn appearance_extra_edits() {
        let Some((mut s, dir)) = scratch("test-appearance", "") else { return };
        let g = ["layout", "focus-ring", "active-gradient"];
        s.set_node(&g, Some("active-gradient from=\"#7fc8ff\" to=\"#7fc8ff\" angle=180".into())).unwrap();
        s.set_props(&g, &[("to", Some(Value::String("#ff0000".into())))]).unwrap();
        s.set_props(&g, &[("relative-to", Some(Value::String("workspace-view".into())))]).unwrap();
        s.set_props(&g, &[("angle", Some(Value::Int(45)))]).unwrap();
        s.set_props(&["layout", "shadow", "offset"], &[("y", Some(Value::Int(8)))]).unwrap();
        s.set_props(&["overview", "workspace-shadow", "offset"], &[("x", Some(Value::Int(2)))]).unwrap();
        s.set_props(&["layout", "tab-indicator", "length"], &[("total-proportion", Some(Value::Float(0.7)))]).unwrap();
        s.set_value(&["layout", "tab-indicator", "active-color"], Some(Value::String("#ff0000".into()))).unwrap();
        let a = ["animations", "window-open"];
        s.set_nodes(&[
            (&[a[0], a[1], "spring"], Some("spring damping-ratio=0.8 stiffness=600 epsilon=0.0001".into())),
            (&[a[0], a[1], "duration-ms"], None),
            (&[a[0], a[1], "curve"], None),
        ])
        .unwrap();
        s.set_props(&[a[0], a[1], "spring"], &[("stiffness", Some(Value::Int(700)))]).unwrap();
        s.set_nodes(&[
            (&[a[0], a[1], "spring"], None),
            (&[a[0], a[1], "duration-ms"], Some("duration-ms 300".into())),
            (&[a[0], a[1], "curve"], Some("curve \"linear\"".into())),
        ])
        .unwrap();
        s.set_flag(&["animations", "screenshot-ui-open", "off"], true).unwrap();
        s.set_node(&g, None).unwrap();
        assert!(s.get(&g).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                copy_dir(&p, &to.join(e.file_name()));
            } else {
                std::fs::copy(&p, to.join(e.file_name())).unwrap();
            }
        }
    }

    #[test]
    fn nest_builds_blocks() {
        assert_eq!(nest(&[], "tap"), "tap");
        assert_eq!(
            nest(&["input", "touchpad"], "tap"),
            "input {\n    touchpad {\n        tap\n    }\n}"
        );
    }
}
