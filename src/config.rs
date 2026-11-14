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
        self.reload()?;
        let originals: Vec<String> = self.files.iter().map(|f| f.doc.text.clone()).collect();
        let res = edits.iter().try_for_each(|(p, t)| self.stage(p, t.clone()));
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
        text = format!("{} {{\n{}\n}}", kdl::ident(p), body);
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

        // Flag on/off in an existing section.
        s.set_flag(&["input", "touchpad", "dwt"], true).unwrap();
        assert!(s.flag(&["input", "touchpad", "dwt"]));
        s.set_flag(&["input", "touchpad", "dwt"], false).unwrap();
        assert!(!s.flag(&["input", "touchpad", "dwt"]));

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
