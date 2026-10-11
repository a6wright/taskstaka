//! `taskstaka setup macos`: launchd agents for the daemon and the tray, so
//! both come back at login. Idempotent: run it again after `cargo install`.
//!
//! The agents carry no `TASKSTAKA_DIR`: the folder comes from Settings
//! (ADR 0011). `KeepAlive {SuccessfulExit: false}` restarts a crash but lets
//! `taskstaka stop` and the tray's Quit stick (ADR 0015).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::service::{self, Launchctl};

pub const DAEMON_LABEL: &str = "dev.taskstaka";
pub const TRAY_LABEL: &str = "dev.taskstaka.tray";

#[derive(Clone, Debug)]
pub struct MacosSetup {
    /// Leave the menu bar item out.
    pub no_tray: bool,
    /// The tray's global key for quick add.
    pub hotkey: String,
    /// Unload and delete both agents instead.
    pub uninstall: bool,
    /// Print what would be done without touching anything.
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct Report {
    /// `None` on uninstall when there was nothing to remove.
    pub daemon_plist: Option<PathBuf>,
    pub tray_plist: Option<PathBuf>,
    pub notes: Vec<String>,
}

/// Install (or remove) the agents for the running `taskstaka` binary.
pub fn install(opts: &MacosSetup) -> Result<Report> {
    if !cfg!(target_os = "macos") {
        anyhow::bail!("`taskstaka setup macos` only works on macOS");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .context("HOME is not set")?;
    let exe = std::env::current_exe().context("locating the taskstaka binary")?;
    install_in(&home, &exe_for_plist(exe), &service::System, opts)
}

/// The path launchd runs: absolute, but symlinks kept — Homebrew's
/// `/opt/homebrew/bin/taskstaka` points into a Cellar folder that
/// `brew upgrade` replaces, so resolving it would break the agents.
pub fn exe_for_plist(exe: PathBuf) -> PathBuf {
    if exe.is_absolute() {
        exe
    } else {
        std::env::current_dir().map(|d| d.join(&exe)).unwrap_or(exe)
    }
}

pub fn install_in(
    home: &Path,
    exe: &Path,
    lc: &dyn Launchctl,
    opts: &MacosSetup,
) -> Result<Report> {
    let agents = service::agents_dir(home);
    let logs = home.join("Library").join("Logs");
    let bin_dir = exe.parent().unwrap_or(Path::new("/usr/local/bin"));
    let daemon_plist = service::plist_path(home, DAEMON_LABEL);
    let tray_path = service::plist_path(home, TRAY_LABEL);
    let mut report = Report {
        daemon_plist: Some(daemon_plist.clone()),
        tray_plist: None,
        notes: Vec::new(),
    };

    if opts.uninstall {
        report.daemon_plist = None;
        for (label, path) in [(DAEMON_LABEL, &daemon_plist), (TRAY_LABEL, &tray_path)] {
            if !path.exists() {
                continue;
            }
            if !opts.dry_run {
                // bootout stops the job too; not loaded is fine, the file still goes.
                let _ = lc.bootout(label);
                std::fs::remove_file(path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
            if label == TRAY_LABEL {
                report.tray_plist = Some(path.clone());
            } else {
                report.daemon_plist = Some(path.clone());
            }
        }
        return Ok(report);
    }

    let tray_exe = exe.with_file_name(format!("taskstaka-tray{}", std::env::consts::EXE_SUFFIX));
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
        DAEMON_LABEL,
        daemon_plist.clone(),
        render_plist(
            DAEMON_LABEL,
            &[exe.to_string_lossy().into_owned(), "serve".into()],
            &logs.join("taskstaka.log"),
            bin_dir,
            false,
        ),
    )];
    if let Some(t) = &tray {
        jobs.push((
            TRAY_LABEL,
            tray_path.clone(),
            render_plist(
                TRAY_LABEL,
                &[
                    t.to_string_lossy().into_owned(),
                    "--hotkey".into(),
                    opts.hotkey.clone(),
                ],
                &logs.join("taskstaka-tray.log"),
                bin_dir,
                true,
            ),
        ));
        report.tray_plist = Some(tray_path);
    }

    if opts.dry_run {
        return Ok(report);
    }
    std::fs::create_dir_all(&agents).with_context(|| format!("creating {}", agents.display()))?;
    std::fs::create_dir_all(&logs).with_context(|| format!("creating {}", logs.display()))?;
    for (label, path, xml) in jobs {
        std::fs::write(&path, xml).with_context(|| format!("writing {}", path.display()))?;
        // Reload so a rerun after `cargo install` picks up the new binary.
        let _ = lc.bootout(label);
        bootstrap_with_retry(lc, &path)?;
    }
    Ok(report)
}

/// launchctl answers "Bootstrap failed: 5: Input/output error" when the job
/// just booted out is still winding down; a few short retries cover it.
fn bootstrap_with_retry(lc: &dyn Launchctl, plist: &Path) -> Result<()> {
    const ATTEMPTS: u32 = 6;
    let mut last = None;
    for i in 0..ATTEMPTS {
        match lc.bootstrap(plist) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if i + 1 < ATTEMPTS && !cfg!(test) {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    }
    Err(last.expect("at least one attempt")).with_context(|| format!("loading {}", plist.display()))
}

/// A LaunchAgent plist. `interactive` marks a GUI job (the tray).
pub fn render_plist(
    label: &str,
    args: &[String],
    log: &Path,
    bin_dir: &Path,
    interactive: bool,
) -> String {
    let args: String = args
        .iter()
        .map(|a| format!("<string>{}</string>", esc(a)))
        .collect();
    let path_env = format!(
        "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
        bin_dir.display()
    );
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    s.push_str("<plist version=\"1.0\"><dict>\n");
    s.push_str(&format!(
        "  <key>Label</key><string>{}</string>\n",
        esc(label)
    ));
    s.push_str(&format!(
        "  <key>ProgramArguments</key><array>{args}</array>\n"
    ));
    s.push_str("  <key>RunAtLoad</key><true/>\n");
    s.push_str("  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n");
    s.push_str(&format!(
        "  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{}</string></dict>\n",
        esc(&path_env)
    ));
    s.push_str(&format!(
        "  <key>StandardOutPath</key><string>{0}</string>\n  <key>StandardErrorPath</key><string>{0}</string>\n",
        esc(&log.to_string_lossy())
    ));
    if interactive {
        s.push_str("  <key>ProcessType</key><string>Interactive</string>\n");
    }
    s.push_str("</dict></plist>\n");
    s
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Human summary for the terminal.
pub fn describe(r: &Report, opts: &MacosSetup) -> String {
    let mut s = String::new();
    if opts.uninstall {
        let verb = if opts.dry_run {
            "would stop and remove"
        } else {
            "stopped and removed"
        };
        let mut any = false;
        for p in r.daemon_plist.iter().chain(r.tray_plist.iter()) {
            any = true;
            s.push_str(&format!("{verb}  {}\n", p.display()));
        }
        if !any {
            s.push_str("nothing to remove: no taskstaka agents in ~/Library/LaunchAgents\n");
        }
        return s;
    }
    let verb = if opts.dry_run {
        "would install"
    } else {
        "installed"
    };
    if let Some(p) = &r.daemon_plist {
        s.push_str(&format!(
            "{verb} daemon  {}  (launchd {DAEMON_LABEL})\n",
            p.display()
        ));
    }
    if let Some(p) = &r.tray_plist {
        s.push_str(&format!(
            "{verb} tray    {}  (launchd {TRAY_LABEL}, hotkey {})\n",
            p.display(),
            opts.hotkey
        ));
    }
    for n in &r.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    if !opts.dry_run {
        s.push_str(&format!(
            "\nBoth start now and at login. Logs: ~/Library/Logs/taskstaka*.log. Status: launchctl print {}/{DAEMON_LABEL}\nAfter `cargo install`, run `taskstaka restart` (daemon) and `taskstaka setup macos` again (tray).\n",
            service::domain()
        ));
    }
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Job, Launchctl};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        /// How many bootstraps still fail (launchctl's "Bootstrap failed: 5"
        /// right after a bootout).
        flaky: std::cell::Cell<u32>,
    }

    impl Launchctl for Fake {
        fn print(&self, _label: &str) -> Option<Job> {
            None
        }
        fn kickstart(&self, label: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("kickstart {label}"));
            Ok(())
        }
        fn bootstrap(&self, plist: &Path) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!(
                "bootstrap {}",
                plist.file_name().unwrap().to_string_lossy()
            ));
            if self.flaky.get() > 0 {
                self.flaky.set(self.flaky.get() - 1);
                anyhow::bail!("Bootstrap failed: 5: Input/output error");
            }
            Ok(())
        }
        fn bootout(&self, label: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("bootout {label}"));
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

    fn opts() -> MacosSetup {
        MacosSetup {
            no_tray: false,
            hotkey: "alt+space".into(),
            uninstall: false,
            dry_run: false,
        }
    }

    #[test]
    fn plist_has_the_keys_launchd_needs_and_no_notes_dir() {
        let p = render_plist(
            "dev.taskstaka",
            &["/Users/me/.cargo/bin/taskstaka".into(), "serve".into()],
            Path::new("/Users/me/Library/Logs/taskstaka.log"),
            Path::new("/Users/me/.cargo/bin"),
            false,
        );
        assert!(p.starts_with("<?xml"));
        assert!(p.contains("<key>Label</key><string>dev.taskstaka</string>"));
        assert!(
            p.contains("<string>/Users/me/.cargo/bin/taskstaka</string><string>serve</string>")
        );
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
        assert!(p.contains("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>"));
        assert!(p.contains(
            "<key>StandardOutPath</key><string>/Users/me/Library/Logs/taskstaka.log</string>"
        ));
        assert!(p.contains("<key>PATH</key><string>/Users/me/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>"));
        assert!(
            !p.contains("TASKSTAKA_DIR"),
            "the folder comes from Settings (ADR 0011)"
        );
        assert!(!p.contains("ProcessType"));
    }

    #[test]
    fn tray_plist_is_interactive_and_carries_the_hotkey() {
        let p = render_plist(
            "dev.taskstaka.tray",
            &[
                "/x/taskstaka-tray".into(),
                "--hotkey".into(),
                "ctrl+alt+space".into(),
            ],
            Path::new("/x/t.log"),
            Path::new("/x"),
            true,
        );
        assert!(p.contains("<string>--hotkey</string><string>ctrl+alt+space</string>"));
        assert!(p.contains("<key>ProcessType</key><string>Interactive</string>"));
    }

    #[test]
    fn xml_special_characters_in_paths_are_escaped() {
        let p = render_plist(
            "l",
            &["/Users/a&b/<x>/taskstaka".into()],
            Path::new("/l"),
            Path::new("/p"),
            false,
        );
        assert!(p.contains("/Users/a&amp;b/&lt;x&gt;/taskstaka"));
    }

    #[test]
    fn installs_both_agents_and_loads_them() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(true);
        let lc = Fake::default();
        let r = install_in(home.path(), &exe, &lc, &opts()).unwrap();
        let agents = home.path().join("Library/LaunchAgents");
        assert_eq!(
            r.daemon_plist.as_deref(),
            Some(agents.join("dev.taskstaka.plist").as_path())
        );
        assert_eq!(
            r.tray_plist.as_deref(),
            Some(agents.join("dev.taskstaka.tray.plist").as_path())
        );
        assert!(
            r.daemon_plist.as_ref().unwrap().exists() && r.tray_plist.as_ref().unwrap().exists()
        );
        let daemon = std::fs::read_to_string(r.daemon_plist.as_ref().unwrap()).unwrap();
        assert!(daemon.contains(&format!(
            "<string>{}</string><string>serve</string>",
            exe.display()
        )));
        let tray = std::fs::read_to_string(r.tray_plist.as_ref().unwrap()).unwrap();
        assert!(tray.contains(
            "taskstaka-tray</string><string>--hotkey</string><string>alt+space</string>"
        ));
        assert_eq!(
            *lc.calls.borrow(),
            vec![
                "bootout dev.taskstaka",
                "bootstrap dev.taskstaka.plist",
                "bootout dev.taskstaka.tray",
                "bootstrap dev.taskstaka.tray.plist",
            ]
        );
        assert!(
            home.path().join("Library/Logs").is_dir(),
            "log folder exists so launchd can write"
        );
    }

    #[test]
    fn bootstrap_right_after_bootout_is_retried() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(false);
        let lc = Fake {
            flaky: std::cell::Cell::new(2),
            ..Default::default()
        };
        install_in(home.path(), &exe, &lc, &opts()).unwrap();
        let calls = lc.calls.borrow();
        assert_eq!(
            calls.iter().filter(|c| c.starts_with("bootstrap")).count(),
            3,
            "{calls:?}"
        );
    }

    #[test]
    fn a_bootstrap_that_keeps_failing_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(false);
        let lc = Fake {
            flaky: std::cell::Cell::new(99),
            ..Default::default()
        };
        let err = install_in(home.path(), &exe, &lc, &opts()).unwrap_err();
        assert!(format!("{err:#}").contains("Bootstrap failed"), "{err:#}");
    }

    #[test]
    fn missing_tray_binary_is_a_note_not_an_error() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(false);
        let lc = Fake::default();
        let r = install_in(home.path(), &exe, &lc, &opts()).unwrap();
        assert!(r.tray_plist.is_none());
        assert!(
            r.notes.iter().any(|n| n.contains("taskstaka-tray")),
            "{:?}",
            r.notes
        );
        assert_eq!(lc.calls.borrow().len(), 2);
    }

    #[test]
    fn no_tray_flag_skips_the_tray_even_when_present() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(true);
        let r = install_in(
            home.path(),
            &exe,
            &Fake::default(),
            &MacosSetup {
                no_tray: true,
                ..opts()
            },
        )
        .unwrap();
        assert!(r.tray_plist.is_none());
        assert!(r.notes.is_empty());
    }

    #[test]
    fn dry_run_writes_and_loads_nothing() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(true);
        let lc = Fake::default();
        let r = install_in(
            home.path(),
            &exe,
            &lc,
            &MacosSetup {
                dry_run: true,
                ..opts()
            },
        )
        .unwrap();
        assert!(!r.daemon_plist.as_ref().unwrap().exists());
        assert!(lc.calls.borrow().is_empty());
        let text = describe(
            &r,
            &MacosSetup {
                dry_run: true,
                ..opts()
            },
        );
        assert!(text.contains("would install"), "{text}");
    }

    #[test]
    fn uninstall_boots_out_and_removes_what_exists() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(true);
        let lc = Fake::default();
        install_in(home.path(), &exe, &lc, &opts()).unwrap();
        lc.calls.borrow_mut().clear();
        let r = install_in(
            home.path(),
            &exe,
            &lc,
            &MacosSetup {
                uninstall: true,
                ..opts()
            },
        )
        .unwrap();
        assert!(!r.daemon_plist.as_ref().unwrap().exists());
        assert!(
            !home
                .path()
                .join("Library/LaunchAgents/dev.taskstaka.tray.plist")
                .exists()
        );
        assert_eq!(
            *lc.calls.borrow(),
            vec!["bootout dev.taskstaka", "bootout dev.taskstaka.tray"]
        );
        let text = describe(
            &r,
            &MacosSetup {
                uninstall: true,
                ..opts()
            },
        );
        assert!(text.contains("stopped and removed"), "{text}");
        assert!(
            !text.contains("stay up"),
            "bootout terminates the jobs: {text}"
        );
    }

    #[test]
    fn uninstall_with_nothing_installed_says_so() {
        let home = tempfile::tempdir().unwrap();
        let (_b, exe) = bins(true);
        let lc = Fake::default();
        let r = install_in(
            home.path(),
            &exe,
            &lc,
            &MacosSetup {
                uninstall: true,
                ..opts()
            },
        )
        .unwrap();
        assert!(r.daemon_plist.is_none() && r.tray_plist.is_none());
        assert!(lc.calls.borrow().is_empty());
        let text = describe(
            &r,
            &MacosSetup {
                uninstall: true,
                ..opts()
            },
        );
        assert!(text.contains("nothing to remove"), "{text}");
    }

    #[test]
    fn the_binary_path_in_the_plist_keeps_symlinks() {
        // Homebrew's /opt/homebrew/bin/taskstaka is a symlink into the Cellar;
        // resolving it would break the agents on the next `brew upgrade`.
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("Cellar/taskstaka/0.1.0/bin/taskstaka");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, b"").unwrap();
        let link = d.path().join("bin/taskstaka");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(exe_for_plist(link.clone()), link);
        // A relative path is made absolute, nothing more.
        let rel = exe_for_plist(PathBuf::from("target/debug/taskstaka"));
        assert!(rel.is_absolute());
        assert!(rel.ends_with("target/debug/taskstaka"));
    }
}
