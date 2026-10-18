//! `taskstaka setup linux`: systemd user units for the daemon and the menu
//! bar item, and the quick-add key. The Linux twin of `setup macos`:
//! idempotent, run it again after `cargo install`.
//!
//! The units carry no `TASKSTAKA_DIR` (the folder comes from Settings,
//! ADR 0011) and use `Restart=on-failure`, so a crash restarts but
//! `taskstaka stop` and the item's Quit stick (ADR 0015). No process can grab
//! a key under Wayland, so the key is a desktop shortcut that runs
//! `taskstaka tray quick`: on GNOME this writes it through gsettings,
//! elsewhere the report says what to bind.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::service::{self, DAEMON_UNIT, Systemctl, TRAY_UNIT};

#[derive(Clone, Debug)]
pub struct LinuxSetup {
    /// Leave the menu bar item (and its key) out.
    pub no_tray: bool,
    /// The quick-add key, in the tray's spelling (`alt+space`).
    pub hotkey: String,
    /// Stop, disable and delete the units and the key instead.
    pub uninstall: bool,
    /// Print what would be done without touching anything.
    pub dry_run: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub daemon_unit: Option<PathBuf>,
    pub tray_unit: Option<PathBuf>,
    /// The GNOME shortcut written (or removed), in GNOME's spelling.
    pub binding: Option<String>,
    pub notes: Vec<String>,
}

/// The desktop's shortcut store, behind a trait so tests never touch
/// gsettings. Values are in gsettings' own text form (`'a'`, `['a', 'b']`).
pub trait Gsettings {
    fn get(&self, schema: &str, key: &str) -> Option<String>;
    fn set(&self, schema: &str, key: &str, value: &str) -> Result<()>;
    fn reset(&self, schema: &str, key: &str) -> Result<()>;
}

/// The real `gsettings` binary.
pub struct SystemGsettings;

impl Gsettings for SystemGsettings {
    fn get(&self, schema: &str, key: &str) -> Option<String> {
        let out = std::process::Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    fn set(&self, schema: &str, key: &str, value: &str) -> Result<()> {
        gsettings(&["set", schema, key, value])
    }
    fn reset(&self, schema: &str, key: &str) -> Result<()> {
        gsettings(&["reset", schema, key])
    }
}

fn gsettings(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("gsettings")
        .args(args)
        .output()
        .with_context(|| format!("running gsettings {}", args.join(" ")))?;
    if out.status.success() {
        Ok(())
    } else {
        bail!(
            "gsettings {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_LIST: &str = "custom-keybindings";
const CUSTOM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
/// Our entry among the custom shortcuts.
pub const BINDING_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/taskstaka/";
pub const BINDING_NAME: &str = "taskstaka: quick add";
const WM_KEYS: &str = "org.gnome.desktop.wm.keybindings";
/// GNOME binds Alt+Space to the window menu; the tray's default key.
const WINDOW_MENU: &str = "activate-window-menu";

/// Install (or remove) the units and the key for the running binary.
pub fn install(opts: &LinuxSetup) -> Result<Report> {
    if !cfg!(target_os = "linux") {
        bail!("`taskstaka setup linux` only works on Linux");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .context("HOME is not set")?;
    let exe = std::env::current_exe().context("locating the taskstaka binary")?;
    let exe = crate::setup_macos::exe_for_plist(exe);
    let gnome = is_gnome(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref());
    let gs: Option<&dyn Gsettings> = if gnome { Some(&SystemGsettings) } else { None };
    let mut notes = Vec::new();
    if !opts.dry_run && !opts.uninstall {
        // A daemon started by hand holds the port the unit's daemon needs,
        // and a hand-started item would sit beside the unit's as a second icon.
        let dir = crate::args::default_dir();
        if taskstaka_client::Client::discover(&dir).is_some() {
            taskstaka_client::Client::stop_and_wait(&dir)?;
            notes.push("stopped the daemon that was running; the unit runs it now".into());
        }
        if let crate::tray::Action::Kill(pids) = crate::tray::stop_action(&crate::tray::detect()) {
            crate::tray::kill(&pids)?;
            notes.push("stopped the menu bar item started by hand; the unit runs it now".into());
        }
    }
    let mut report = install_in(&home, &exe, &service::System, gs, opts)?;
    if gnome && report.tray_unit.is_some() {
        let enabled = std::process::Command::new("gnome-extensions")
            .args(["list", "--enabled"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        notes.extend(appindicator_note(&enabled));
    }
    report.notes.extend(notes);
    Ok(report)
}

/// GNOME shows no tray items at all without a StatusNotifier host; Ubuntu
/// ships the AppIndicator extension enabled, other GNOMEs do not. `enabled`
/// is `gnome-extensions list --enabled` (empty when the tool is missing,
/// which is not GNOME Shell: no note then).
pub fn appindicator_note(enabled: &str) -> Option<String> {
    if enabled.trim().is_empty() || enabled.to_ascii_lowercase().contains("appindicator") {
        return None;
    }
    Some(
        "GNOME hides tray items without the AppIndicator extension: install \
         gnome-shell-extension-appindicator (apt/dnf), log out and in, then \
         `gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com`"
            .to_string(),
    )
}

/// `XDG_CURRENT_DESKTOP` names GNOME (`ubuntu:GNOME`, `GNOME`, `GNOME-Classic:GNOME`).
pub fn is_gnome(desktop: Option<&str>) -> bool {
    desktop.is_some_and(|d| d.split(':').any(|p| p.eq_ignore_ascii_case("gnome")))
}

pub fn install_in(
    home: &Path,
    exe: &Path,
    sc: &dyn Systemctl,
    gs: Option<&dyn Gsettings>,
    opts: &LinuxSetup,
) -> Result<Report> {
    let units = service::units_dir(home);
    let bin_dir = exe.parent().unwrap_or(Path::new("/usr/local/bin"));
    let daemon_path = service::unit_path(home, DAEMON_UNIT);
    let tray_path = service::unit_path(home, TRAY_UNIT);
    let mut report = Report::default();

    if opts.uninstall {
        for (unit, path) in [(DAEMON_UNIT, &daemon_path), (TRAY_UNIT, &tray_path)] {
            if !path.exists() {
                continue;
            }
            if !opts.dry_run {
                // Stops it too; an unknown unit is fine, the file still goes.
                let _ = sc.disable_now(unit);
                std::fs::remove_file(path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
            if unit == TRAY_UNIT {
                report.tray_unit = Some(path.clone());
            } else {
                report.daemon_unit = Some(path.clone());
            }
        }
        if !opts.dry_run && (report.daemon_unit.is_some() || report.tray_unit.is_some()) {
            sc.daemon_reload()?;
        }
        if let Some(gs) = gs {
            report.binding = remove_binding(gs, opts.dry_run)?;
        }
        return Ok(report);
    }

    let tray_exe = exe.with_file_name("taskstaka-tray");
    let tray = if opts.no_tray {
        None
    } else if tray_exe.is_file() {
        Some(tray_exe)
    } else {
        report.notes.push(format!(
            "no taskstaka-tray beside {}; install it (cargo install --path crates/tray) and run this again for the menu bar item",
            exe.display()
        ));
        None
    };

    let mut jobs = vec![(
        DAEMON_UNIT,
        daemon_path.clone(),
        render_unit(
            UnitKind::Daemon,
            &[exe.to_string_lossy().into_owned(), "serve".into()],
            bin_dir,
        ),
    )];
    report.daemon_unit = Some(daemon_path);
    if let Some(t) = &tray {
        jobs.push((
            TRAY_UNIT,
            tray_path.clone(),
            render_unit(
                UnitKind::Tray,
                &[
                    t.to_string_lossy().into_owned(),
                    "--hotkey".into(),
                    opts.hotkey.clone(),
                ],
                bin_dir,
            ),
        ));
        report.tray_unit = Some(tray_path);
        let command = format!("{} tray quick", exe.display());
        match gs {
            Some(gs) => {
                let key = gnome_key(&opts.hotkey)?;
                write_binding(gs, &key, &command, opts.dry_run, &mut report.notes)?;
                report.binding = Some(key);
            }
            None => report.notes.push(format!(
                "not GNOME: bind {} to `{command}` in your desktop's keyboard shortcuts",
                opts.hotkey
            )),
        }
    }

    if opts.dry_run {
        return Ok(report);
    }
    std::fs::create_dir_all(&units).with_context(|| format!("creating {}", units.display()))?;
    for (_, path, text) in &jobs {
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    sc.daemon_reload()?;
    for (unit, _, _) in &jobs {
        sc.enable(unit)?;
        // Restart, not start: a rerun after `cargo install` must pick up
        // the new binary.
        sc.restart(unit)?;
    }
    Ok(report)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Daemon,
    Tray,
}

/// A systemd user unit. The tray is part of the graphical session: it
/// starts with it and stops with it.
pub fn render_unit(kind: UnitKind, args: &[String], bin_dir: &Path) -> String {
    let exec: Vec<String> = args.iter().map(|a| quote_exec(a)).collect();
    let path_env = format!("{}:/usr/local/bin:/usr/bin:/bin", bin_dir.display());
    let mut s = String::new();
    s.push_str("# Written by `taskstaka setup linux`; run it again after `cargo install`.\n");
    s.push_str("[Unit]\n");
    match kind {
        UnitKind::Daemon => s.push_str("Description=taskstaka daemon\n"),
        UnitKind::Tray => {
            s.push_str("Description=taskstaka menu bar item\n");
            s.push_str("PartOf=graphical-session.target\n");
            s.push_str("After=graphical-session.target\n");
        }
    }
    s.push_str("\n[Service]\n");
    s.push_str(&format!("ExecStart={}\n", exec.join(" ")));
    s.push_str(&format!("Environment=PATH={path_env}\n"));
    s.push_str("Restart=on-failure\n");
    s.push_str("\n[Install]\n");
    match kind {
        UnitKind::Daemon => s.push_str("WantedBy=default.target\n"),
        UnitKind::Tray => s.push_str("WantedBy=graphical-session.target\n"),
    }
    s
}

/// systemd's ExecStart quoting: double quotes around anything with
/// whitespace, backslash-escaped inside.
fn quote_exec(arg: &str) -> String {
    if arg.chars().any(char::is_whitespace) || arg.is_empty() {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

/// The tray's `alt+space` as GNOME writes it: `<Alt>space`.
pub fn gnome_key(combo: &str) -> Result<String> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
    let key = key.first().copied().unwrap_or("");
    if key.is_empty() {
        bail!("bad hotkey {combo:?}: no key after the modifiers");
    }
    let mut s = String::new();
    for m in mods {
        s.push_str(match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => "<Control>",
            "alt" | "option" => "<Alt>",
            "shift" => "<Shift>",
            "super" | "cmd" | "command" | "meta" => "<Super>",
            other => bail!("bad hotkey {combo:?}: unknown modifier {other:?}"),
        });
    }
    s.push_str(&gnome_key_name(key));
    Ok(s)
}

/// global-hotkey's key names in GTK accelerator spelling.
fn gnome_key_name(key: &str) -> String {
    if let Some(k) = key.strip_prefix("Key").filter(|k| k.len() == 1) {
        return k.to_ascii_lowercase();
    }
    if let Some(d) = key.strip_prefix("Digit").filter(|d| d.len() == 1) {
        return d.to_string();
    }
    let named = match key.to_ascii_lowercase().as_str() {
        "space" => "space",
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "tab" => "Tab",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "backquote" => "grave",
        "minus" => "minus",
        "equal" => "equal",
        "bracketleft" => "bracketleft",
        "bracketright" => "bracketright",
        "backslash" => "backslash",
        "semicolon" => "semicolon",
        "quote" => "apostrophe",
        "comma" => "comma",
        "period" => "period",
        "slash" => "slash",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    if key.len() == 1 {
        return key.to_ascii_lowercase();
    }
    // F1..F24 and anything else GTK already understands.
    key.to_string()
}

/// `<alt>space` and `<Alt>space` are the same key (modifier order too).
fn same_key(a: &str, b: &str) -> bool {
    fn norm(k: &str) -> (Vec<String>, String) {
        let mut mods = Vec::new();
        let mut rest = k.to_string();
        while let Some(end) = rest.find('>') {
            if !rest.starts_with('<') {
                break;
            }
            mods.push(rest[1..end].to_ascii_lowercase());
            rest = rest[end + 1..].to_string();
        }
        mods.sort();
        (mods, rest.to_ascii_lowercase())
    }
    norm(a) == norm(b)
}

fn write_binding(
    gs: &dyn Gsettings,
    key: &str,
    command: &str,
    dry_run: bool,
    notes: &mut Vec<String>,
) -> Result<()> {
    let list = gs
        .get(MEDIA_KEYS, CUSTOM_LIST)
        .map(|v| parse_strv(&v))
        .unwrap_or_default();
    let window_menu = gs
        .get(WM_KEYS, WINDOW_MENU)
        .map(|v| parse_strv(&v))
        .unwrap_or_default();
    let taken = window_menu.iter().any(|k| same_key(k, key));
    if taken {
        notes.push(format!(
            "{key} was GNOME's window menu; it is quick add now (`gsettings reset {WM_KEYS} {WINDOW_MENU}` gives it back)"
        ));
    }
    if dry_run {
        return Ok(());
    }
    if taken {
        let rest: Vec<String> = window_menu
            .into_iter()
            .filter(|k| !same_key(k, key))
            .collect();
        gs.set(WM_KEYS, WINDOW_MENU, &format_strv(&rest))?;
    }
    if !list.iter().any(|p| p == BINDING_PATH) {
        let mut list = list;
        list.push(BINDING_PATH.to_string());
        gs.set(MEDIA_KEYS, CUSTOM_LIST, &format_strv(&list))?;
    }
    let schema = format!("{CUSTOM_SCHEMA}:{BINDING_PATH}");
    gs.set(&schema, "name", &quote_gvariant(BINDING_NAME))?;
    gs.set(&schema, "command", &quote_gvariant(command))?;
    gs.set(&schema, "binding", &quote_gvariant(key))?;
    Ok(())
}

/// Returns the key that was bound, if our entry existed.
fn remove_binding(gs: &dyn Gsettings, dry_run: bool) -> Result<Option<String>> {
    let list = gs
        .get(MEDIA_KEYS, CUSTOM_LIST)
        .map(|v| parse_strv(&v))
        .unwrap_or_default();
    if !list.iter().any(|p| p == BINDING_PATH) {
        return Ok(None);
    }
    let schema = format!("{CUSTOM_SCHEMA}:{BINDING_PATH}");
    let key = gs.get(&schema, "binding").map(|v| unquote_gvariant(&v));
    if dry_run {
        return Ok(key);
    }
    let rest: Vec<String> = list.into_iter().filter(|p| p != BINDING_PATH).collect();
    gs.set(MEDIA_KEYS, CUSTOM_LIST, &format_strv(&rest))?;
    for k in ["name", "command", "binding"] {
        let _ = gs.reset(&schema, k);
    }
    // Give the window menu back when we took it and nothing else has it.
    if key.as_deref().is_some_and(|k| same_key(k, "<Alt>space"))
        && gs
            .get(WM_KEYS, WINDOW_MENU)
            .is_some_and(|v| parse_strv(&v).is_empty())
    {
        gs.reset(WM_KEYS, WINDOW_MENU)?;
    }
    Ok(key)
}

/// `['a', 'b']` / `@as []` / `[]` as gsettings prints a string array.
pub fn parse_strv(v: &str) -> Vec<String> {
    let v = v.trim();
    let v = v.strip_prefix("@as").map(str::trim).unwrap_or(v);
    let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in inner.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    cur.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == q {
                    out.push(std::mem::take(&mut cur));
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                }
            }
        }
    }
    out
}

pub fn format_strv(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    let quoted: Vec<String> = items.iter().map(|s| quote_gvariant(s)).collect();
    format!("[{}]", quoted.join(", "))
}

fn quote_gvariant(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn unquote_gvariant(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| v.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .map(|s| s.replace("\\'", "'").replace("\\\\", "\\"))
        .unwrap_or_else(|| v.to_string())
}

/// Human summary for the terminal.
pub fn describe(r: &Report, opts: &LinuxSetup) -> String {
    let mut s = String::new();
    if opts.uninstall {
        let verb = if opts.dry_run {
            "would stop and remove"
        } else {
            "stopped and removed"
        };
        let mut any = false;
        for p in r.daemon_unit.iter().chain(r.tray_unit.iter()) {
            any = true;
            s.push_str(&format!("{verb}  {}\n", p.display()));
        }
        if let Some(k) = &r.binding {
            any = true;
            s.push_str(&format!("{verb}  GNOME shortcut {k} ({BINDING_NAME})\n"));
        }
        if !any {
            s.push_str("nothing to remove: no taskstaka units in ~/.config/systemd/user\n");
        }
        return s;
    }
    let verb = if opts.dry_run {
        "would install"
    } else {
        "installed"
    };
    if let Some(p) = &r.daemon_unit {
        s.push_str(&format!(
            "{verb} daemon  {}  (systemd {DAEMON_UNIT})\n",
            p.display()
        ));
    }
    if let Some(p) = &r.tray_unit {
        s.push_str(&format!(
            "{verb} tray    {}  (systemd {TRAY_UNIT}, hotkey {})\n",
            p.display(),
            opts.hotkey
        ));
    }
    if let Some(k) = &r.binding {
        s.push_str(&format!(
            "{verb} key     {k} runs `taskstaka tray quick`  (GNOME shortcut \"{BINDING_NAME}\")\n"
        ));
    }
    for n in &r.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    if !opts.dry_run {
        s.push_str(
            "\nBoth start now and at login. Logs: journalctl --user -u taskstaka -u taskstaka-tray. Status: systemctl --user status taskstaka taskstaka-tray\nAfter `cargo install`, run `taskstaka setup linux` again (restarts both).\n",
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Unit;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct FakeSystemctl {
        calls: RefCell<Vec<String>>,
    }

    impl Systemctl for FakeSystemctl {
        fn show(&self, _unit: &str) -> Option<Unit> {
            None
        }
        fn daemon_reload(&self) -> Result<()> {
            self.calls.borrow_mut().push("daemon-reload".into());
            Ok(())
        }
        fn enable(&self, unit: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("enable {unit}"));
            Ok(())
        }
        fn disable_now(&self, unit: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("disable --now {unit}"));
            Ok(())
        }
        fn start(&self, unit: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("start {unit}"));
            Ok(())
        }
        fn stop(&self, unit: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("stop {unit}"));
            Ok(())
        }
        fn restart(&self, unit: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("restart {unit}"));
            Ok(())
        }
    }

    /// A gsettings store with GNOME's defaults for the keys we touch.
    struct FakeGsettings {
        values: RefCell<HashMap<String, String>>,
    }

    impl FakeGsettings {
        fn gnome_defaults() -> FakeGsettings {
            let mut v = HashMap::new();
            v.insert(format!("{MEDIA_KEYS} {CUSTOM_LIST}"), "@as []".to_string());
            v.insert(
                format!("{WM_KEYS} {WINDOW_MENU}"),
                "['<Alt>space']".to_string(),
            );
            FakeGsettings {
                values: RefCell::new(v),
            }
        }
        fn value(&self, schema: &str, key: &str) -> Option<String> {
            self.values
                .borrow()
                .get(&format!("{schema} {key}"))
                .cloned()
        }
    }

    impl Gsettings for FakeGsettings {
        fn get(&self, schema: &str, key: &str) -> Option<String> {
            self.value(schema, key)
        }
        fn set(&self, schema: &str, key: &str, value: &str) -> Result<()> {
            self.values
                .borrow_mut()
                .insert(format!("{schema} {key}"), value.to_string());
            Ok(())
        }
        fn reset(&self, schema: &str, key: &str) -> Result<()> {
            let k = format!("{schema} {key}");
            if k == format!("{WM_KEYS} {WINDOW_MENU}") {
                self.values.borrow_mut().insert(k, "['<Alt>space']".into());
            } else {
                self.values.borrow_mut().remove(&k);
            }
            Ok(())
        }
    }

    fn bins(with_tray: bool) -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("taskstaka");
        std::fs::write(&exe, b"").unwrap();
        if with_tray {
            std::fs::write(d.path().join("taskstaka-tray"), b"").unwrap();
        }
        (d, exe)
    }

    fn opts() -> LinuxSetup {
        LinuxSetup {
            no_tray: false,
            hotkey: "alt+space".into(),
            uninstall: false,
            dry_run: false,
        }
    }

    #[test]
    fn hotkeys_in_gnome_spelling() {
        assert_eq!(gnome_key("alt+space").unwrap(), "<Alt>space");
        assert_eq!(gnome_key("ctrl+alt+space").unwrap(), "<Control><Alt>space");
        assert_eq!(gnome_key("super+shift+KeyT").unwrap(), "<Super><Shift>t");
        assert_eq!(gnome_key("ctrl+Digit1").unwrap(), "<Control>1");
        assert_eq!(gnome_key("alt+Enter").unwrap(), "<Alt>Return");
        assert_eq!(gnome_key("alt+F5").unwrap(), "<Alt>F5");
        assert_eq!(gnome_key("cmd+Backquote").unwrap(), "<Super>grave");
        assert!(gnome_key("alt+").is_err());
        assert!(gnome_key("hyper+space").is_err());
        assert!(same_key("<alt>Space", "<Alt>space"));
        assert!(same_key("<Shift><Alt>space", "<Alt><Shift>space"));
        assert!(!same_key("<Alt>space", "<Super>space"));
    }

    #[test]
    fn gnome_without_an_appindicator_host_gets_a_note() {
        assert!(appindicator_note("ubuntu-appindicators@ubuntu.com\nding@rastersoft.com\n").is_none());
        assert!(appindicator_note("appindicatorsupport@rgcjonas.gmail.com\n").is_none());
        assert!(appindicator_note("").is_none(), "no gnome-extensions tool: say nothing");
        let n = appindicator_note("ding@rastersoft.com\n").unwrap();
        assert!(n.contains("gnome-shell-extension-appindicator"), "{n}");
    }

    #[test]
    fn gvariant_string_arrays_round_trip() {
        assert_eq!(parse_strv("@as []"), Vec::<String>::new());
        assert_eq!(parse_strv("[]"), Vec::<String>::new());
        assert_eq!(
            parse_strv("['/a/', '/b/']"),
            vec!["/a/".to_string(), "/b/".to_string()]
        );
        assert_eq!(parse_strv("[\"it's\"]"), vec!["it's".to_string()]);
        assert_eq!(format_strv(&[]), "@as []");
        assert_eq!(format_strv(&["/a/".into()]), "['/a/']");
        assert_eq!(unquote_gvariant("'<Alt>space'"), "<Alt>space");
    }

    #[test]
    fn units_say_what_runs_and_how_they_restart() {
        let daemon = render_unit(
            UnitKind::Daemon,
            &["/home/me/.cargo/bin/taskstaka".into(), "serve".into()],
            Path::new("/home/me/.cargo/bin"),
        );
        assert!(daemon.contains("ExecStart=/home/me/.cargo/bin/taskstaka serve\n"));
        assert!(daemon.contains("Restart=on-failure\n"));
        assert!(daemon.contains("WantedBy=default.target\n"));
        assert!(daemon.contains("Environment=PATH=/home/me/.cargo/bin:"));
        assert!(!daemon.contains("TASKSTAKA_DIR"));
        let tray = render_unit(
            UnitKind::Tray,
            &[
                "/opt/my bin/taskstaka-tray".into(),
                "--hotkey".into(),
                "alt+space".into(),
            ],
            Path::new("/opt/my bin"),
        );
        assert!(tray.contains("ExecStart=\"/opt/my bin/taskstaka-tray\" --hotkey alt+space\n"));
        assert!(tray.contains("PartOf=graphical-session.target\n"));
        assert!(tray.contains("WantedBy=graphical-session.target\n"));
    }

    #[test]
    fn install_writes_both_units_enables_them_and_binds_the_key() {
        let home = tempfile::tempdir().unwrap();
        let (_bins, exe) = bins(true);
        let sc = FakeSystemctl::default();
        let gs = FakeGsettings::gnome_defaults();
        let r = install_in(home.path(), &exe, &sc, Some(&gs), &opts()).unwrap();
        let units = service::units_dir(home.path());
        assert_eq!(r.daemon_unit, Some(units.join(DAEMON_UNIT)));
        assert_eq!(r.tray_unit, Some(units.join(TRAY_UNIT)));
        assert!(units.join(DAEMON_UNIT).is_file());
        assert!(
            std::fs::read_to_string(units.join(TRAY_UNIT))
                .unwrap()
                .contains("--hotkey alt+space")
        );
        assert_eq!(
            *sc.calls.borrow(),
            vec![
                "daemon-reload",
                "enable taskstaka.service",
                "restart taskstaka.service",
                "enable taskstaka-tray.service",
                "restart taskstaka-tray.service",
            ]
        );
        assert_eq!(r.binding.as_deref(), Some("<Alt>space"));
        assert_eq!(
            gs.value(MEDIA_KEYS, CUSTOM_LIST).unwrap(),
            format!("['{BINDING_PATH}']")
        );
        let schema = format!("{CUSTOM_SCHEMA}:{BINDING_PATH}");
        assert_eq!(gs.value(&schema, "binding").unwrap(), "'<Alt>space'");
        assert_eq!(
            gs.value(&schema, "command").unwrap(),
            format!("'{} tray quick'", exe.display())
        );
        // Alt+Space was the window menu: freed, and said so.
        assert_eq!(gs.value(WM_KEYS, WINDOW_MENU).unwrap(), "@as []");
        assert!(
            r.notes.iter().any(|n| n.contains("window menu")),
            "{:?}",
            r.notes
        );
        let text = describe(&r, &opts());
        assert!(text.contains("installed key     <Alt>space"), "{text}");
    }

    #[test]
    fn a_second_run_keeps_one_entry_and_another_key_leaves_the_window_menu() {
        let home = tempfile::tempdir().unwrap();
        let (_bins, exe) = bins(true);
        let sc = FakeSystemctl::default();
        let gs = FakeGsettings::gnome_defaults();
        let o = LinuxSetup {
            hotkey: "ctrl+alt+space".into(),
            ..opts()
        };
        install_in(home.path(), &exe, &sc, Some(&gs), &o).unwrap();
        install_in(home.path(), &exe, &sc, Some(&gs), &o).unwrap();
        assert_eq!(
            gs.value(MEDIA_KEYS, CUSTOM_LIST).unwrap(),
            format!("['{BINDING_PATH}']")
        );
        assert_eq!(gs.value(WM_KEYS, WINDOW_MENU).unwrap(), "['<Alt>space']");
    }

    #[test]
    fn dry_run_touches_nothing() {
        let home = tempfile::tempdir().unwrap();
        let (_bins, exe) = bins(true);
        let sc = FakeSystemctl::default();
        let gs = FakeGsettings::gnome_defaults();
        let o = LinuxSetup {
            dry_run: true,
            ..opts()
        };
        let r = install_in(home.path(), &exe, &sc, Some(&gs), &o).unwrap();
        assert!(r.tray_unit.is_some());
        assert_eq!(r.binding.as_deref(), Some("<Alt>space"));
        assert!(!service::units_dir(home.path()).exists());
        assert!(sc.calls.borrow().is_empty());
        assert_eq!(gs.value(MEDIA_KEYS, CUSTOM_LIST).unwrap(), "@as []");
        assert!(describe(&r, &o).contains("would install daemon"));
    }

    #[test]
    fn without_the_tray_binary_or_gnome_the_report_says_what_to_do() {
        let home = tempfile::tempdir().unwrap();
        let (_bins, exe) = bins(false);
        let sc = FakeSystemctl::default();
        let r = install_in(home.path(), &exe, &sc, None, &opts()).unwrap();
        assert!(r.tray_unit.is_none());
        assert!(r.binding.is_none());
        assert!(r.notes.iter().any(|n| n.contains("no taskstaka-tray")));
        assert_eq!(
            sc.calls.borrow().len(),
            3,
            "reload, enable, restart the daemon only"
        );

        let (_bins, exe) = bins(true);
        let r = install_in(home.path(), &exe, &sc, None, &opts()).unwrap();
        assert!(r.tray_unit.is_some());
        assert!(
            r.notes
                .iter()
                .any(|n| n.contains("not GNOME") && n.contains("tray quick"))
        );
        assert!(!is_gnome(Some("KDE")));
        assert!(is_gnome(Some("ubuntu:GNOME")));
        assert!(is_gnome(Some("GNOME-Classic:GNOME")));
    }

    #[test]
    fn uninstall_removes_units_and_gives_the_window_menu_back() {
        let home = tempfile::tempdir().unwrap();
        let (_bins, exe) = bins(true);
        let sc = FakeSystemctl::default();
        let gs = FakeGsettings::gnome_defaults();
        install_in(home.path(), &exe, &sc, Some(&gs), &opts()).unwrap();
        sc.calls.borrow_mut().clear();
        let o = LinuxSetup {
            uninstall: true,
            ..opts()
        };
        let r = install_in(home.path(), &exe, &sc, Some(&gs), &o).unwrap();
        assert!(r.daemon_unit.is_some() && r.tray_unit.is_some());
        assert_eq!(r.binding.as_deref(), Some("<Alt>space"));
        assert!(!service::unit_path(home.path(), DAEMON_UNIT).exists());
        assert_eq!(
            *sc.calls.borrow(),
            vec![
                "disable --now taskstaka.service",
                "disable --now taskstaka-tray.service",
                "daemon-reload",
            ]
        );
        assert_eq!(gs.value(MEDIA_KEYS, CUSTOM_LIST).unwrap(), "@as []");
        assert_eq!(gs.value(WM_KEYS, WINDOW_MENU).unwrap(), "['<Alt>space']");
        assert!(describe(&r, &o).contains("stopped and removed  GNOME shortcut <Alt>space"));

        // Nothing left: says so.
        let r = install_in(home.path(), &exe, &sc, Some(&gs), &o).unwrap();
        assert!(describe(&r, &o).contains("nothing to remove"));
    }
}
