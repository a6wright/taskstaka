//! Who keeps the daemon alive. On macOS a launchd agent may own it (installed
//! by `taskstaka setup macos`, or by `brew services`); on Linux a systemd user
//! unit (`taskstaka setup linux`). `stop`/`restart`/`status` then go through
//! `launchctl` or `systemctl --user` instead of racing it with their own daemon.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

/// Our own agent label (written by `setup macos`) and Homebrew's — `sh.brew.*`
/// since Homebrew 7, `homebrew.mxcl.*` before — in the order they are looked for.
pub const LABELS: [&str; 3] = [
    "dev.taskstaka",
    "sh.brew.taskstaka",
    "homebrew.mxcl.taskstaka",
];

/// The systemd user units `setup linux` writes.
pub const DAEMON_UNIT: &str = "taskstaka.service";
pub const TRAY_UNIT: &str = "taskstaka-tray.service";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Supervisor {
    /// A loaded launchd agent owns the daemon for this folder.
    Launchd {
        label: String,
    },
    /// A systemd user unit owns the daemon for this folder.
    Systemd {
        unit: String,
    },
    None,
}

impl Supervisor {
    /// This supervisor, or `fallback()` when there is none: what owned the
    /// daemon before a stop beats a fresh guess among several loaded agents.
    pub fn or_else(self, fallback: impl FnOnce() -> Supervisor) -> Supervisor {
        match self {
            Supervisor::None => fallback(),
            s => s,
        }
    }

    /// The agent label or unit name, for `--json` and messages.
    pub fn label(&self) -> Option<&str> {
        match self {
            Supervisor::Launchd { label } => Some(label),
            Supervisor::Systemd { unit } => Some(unit),
            Supervisor::None => None,
        }
    }

    /// `launchd dev.taskstaka` / `systemd taskstaka.service`, for a person.
    pub fn describe(&self) -> Option<String> {
        match self {
            Supervisor::Launchd { label } => Some(format!("launchd {label}")),
            Supervisor::Systemd { unit } => Some(format!("systemd {unit}")),
            Supervisor::None => None,
        }
    }

    /// Bring the daemon up through its supervisor (a fresh start, or a
    /// restart if it is somehow still there).
    pub fn start(&self) -> Result<()> {
        match self {
            Supervisor::Launchd { label } => System.kickstart(label),
            Supervisor::Systemd { unit } => System.restart(unit),
            Supervisor::None => bail!("no supervisor"),
        }
    }
}

/// What `launchctl print` says about a loaded job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Job {
    pub pid: Option<u32>,
}

/// The few launchctl verbs we use, behind a trait so tests never touch launchd.
pub trait Launchctl {
    /// `None` when the label is not loaded in the gui domain.
    fn print(&self, label: &str) -> Option<Job>;
    fn kickstart(&self, label: &str) -> Result<()>;
    fn bootstrap(&self, plist: &Path) -> Result<()>;
    fn bootout(&self, label: &str) -> Result<()>;
}

/// The real thing.
pub struct System;

impl Launchctl for System {
    fn print(&self, label: &str) -> Option<Job> {
        let out = std::process::Command::new("launchctl")
            .args(["print", &format!("{}/{label}", domain())])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(parse_print(&String::from_utf8_lossy(&out.stdout)))
    }

    fn kickstart(&self, label: &str) -> Result<()> {
        run(&["kickstart", "-k", &format!("{}/{label}", domain())])
    }

    fn bootstrap(&self, plist: &Path) -> Result<()> {
        run(&["bootstrap", &domain(), &plist.to_string_lossy()])
    }

    fn bootout(&self, label: &str) -> Result<()> {
        run(&["bootout", &format!("{}/{label}", domain())])
    }
}

fn run(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .with_context(|| format!("running launchctl {}", args.join(" ")))?;
    if out.status.success() {
        Ok(())
    } else {
        bail!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

/// `pid = N` from `launchctl print` output; absent while the job is not running.
pub fn parse_print(out: &str) -> Job {
    let pid = out
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("pid = "))
        .and_then(|p| p.trim().parse().ok());
    Job { pid }
}

/// The launchd domain of this user's login session, where menu bar items
/// and the daemon live: `gui/<uid>`.
pub fn domain() -> String {
    format!("gui/{}", uid())
}

#[cfg(unix)]
fn uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::env::var_os("HOME")
        .and_then(|h| std::fs::metadata(h).ok())
        .map(|m| m.uid())
        .unwrap_or(501)
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

/// What `systemctl --user show` says about a unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub active: bool,
    /// The main process while active.
    pub pid: Option<u32>,
}

/// The few `systemctl --user` verbs we use, behind a trait so tests never
/// touch systemd.
pub trait Systemctl {
    /// `None` when systemd does not know the unit (no file, or not reloaded).
    fn show(&self, unit: &str) -> Option<Unit>;
    fn daemon_reload(&self) -> Result<()>;
    fn enable(&self, unit: &str) -> Result<()>;
    fn disable_now(&self, unit: &str) -> Result<()>;
    fn start(&self, unit: &str) -> Result<()>;
    fn stop(&self, unit: &str) -> Result<()>;
    fn restart(&self, unit: &str) -> Result<()>;
}

impl Systemctl for System {
    fn show(&self, unit: &str) -> Option<Unit> {
        let out = std::process::Command::new("systemctl")
            .args([
                "--user",
                "show",
                "-p",
                "LoadState,ActiveState,MainPID",
                unit,
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_show(&String::from_utf8_lossy(&out.stdout))
    }
    fn daemon_reload(&self) -> Result<()> {
        systemctl(&["daemon-reload"])
    }
    fn enable(&self, unit: &str) -> Result<()> {
        systemctl(&["enable", unit])
    }
    fn disable_now(&self, unit: &str) -> Result<()> {
        systemctl(&["disable", "--now", unit])
    }
    fn start(&self, unit: &str) -> Result<()> {
        systemctl(&["start", unit])
    }
    fn stop(&self, unit: &str) -> Result<()> {
        systemctl(&["stop", unit])
    }
    fn restart(&self, unit: &str) -> Result<()> {
        systemctl(&["restart", unit])
    }
}

fn systemctl(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .with_context(|| format!("running systemctl --user {}", args.join(" ")))?;
    if out.status.success() {
        Ok(())
    } else {
        bail!(
            "systemctl --user {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
    }
}

/// `LoadState=`, `ActiveState=`, `MainPID=` lines from `systemctl show`.
/// `None` for a unit systemd has not loaded.
pub fn parse_show(out: &str) -> Option<Unit> {
    let mut load = "";
    let mut active = "";
    let mut pid = 0u32;
    for line in out.lines() {
        if let Some((k, v)) = line.trim().split_once('=') {
            match k {
                "LoadState" => load = v,
                "ActiveState" => active = v,
                "MainPID" => pid = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    if load != "loaded" {
        return None;
    }
    Some(Unit {
        active: active == "active" || active == "activating",
        pid: (pid != 0).then_some(pid),
    })
}

/// `~/.config/systemd/user`, where `setup linux` writes the units.
pub fn units_dir(home: &Path) -> PathBuf {
    home.join(".config").join("systemd").join("user")
}

pub fn unit_path(home: &Path, unit: &str) -> PathBuf {
    units_dir(home).join(unit)
}

pub fn agents_dir(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents")
}

pub fn plist_path(home: &Path, label: &str) -> PathBuf {
    agents_dir(home).join(format!("{label}.plist"))
}

/// Who supervises the daemon for `dir`. `running_pid` is the pid of the
/// daemon found for `dir`, if any; `default_dir` is the folder a launchd
/// daemon would serve (it gets no `--dir`, so Settings decide, ADR 0011);
/// `port` is the port the user asked for, if any — launchd's daemon serves
/// the default one, so another port can only be an unsupervised daemon.
pub fn detect(
    dir: &Path,
    default_dir: &Path,
    running_pid: Option<u32>,
    port: Option<u16>,
) -> Supervisor {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Supervisor::None;
    };
    if cfg!(target_os = "macos") {
        detect_in(&home, &System, dir, default_dir, running_pid, port)
    } else if cfg!(target_os = "linux") {
        detect_systemd_in(&home, &System, dir, default_dir, running_pid, port)
    } else {
        Supervisor::None
    }
}

/// The Linux counterpart of `detect_in`: the daemon unit owns the daemon
/// when it is the running process, or, with nothing running, when the unit
/// is loaded and would serve this folder (it gets no `--dir` either).
pub fn detect_systemd_in(
    home: &Path,
    sc: &dyn Systemctl,
    dir: &Path,
    default_dir: &Path,
    running_pid: Option<u32>,
    port: Option<u16>,
) -> Supervisor {
    if port.is_some_and(|p| p != taskstaka_client::DEFAULT_PORT) {
        return Supervisor::None;
    }
    if !unit_path(home, DAEMON_UNIT).exists() {
        return Supervisor::None;
    }
    let Some(unit) = sc.show(DAEMON_UNIT) else {
        return Supervisor::None;
    };
    let ours = match running_pid {
        Some(pid) => unit.pid == Some(pid),
        None => same_dir(dir, default_dir),
    };
    if ours {
        Supervisor::Systemd {
            unit: DAEMON_UNIT.to_string(),
        }
    } else {
        Supervisor::None
    }
}

pub fn detect_in(
    home: &Path,
    lc: &dyn Launchctl,
    dir: &Path,
    default_dir: &Path,
    running_pid: Option<u32>,
    port: Option<u16>,
) -> Supervisor {
    if port.is_some_and(|p| p != taskstaka_client::DEFAULT_PORT) {
        return Supervisor::None;
    }
    for label in LABELS {
        if !plist_path(home, label).exists() {
            continue;
        }
        let Some(job) = lc.print(label) else {
            continue;
        };
        let ours = match running_pid {
            // A daemon is running for this folder: it is launchd's only if
            // launchd started that very process.
            Some(pid) => job.pid == Some(pid),
            // Nothing running: launchd would serve the default folder.
            None => same_dir(dir, default_dir),
        };
        if ours {
            return Supervisor::Launchd {
                label: label.to_string(),
            };
        }
    }
    Supervisor::None
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// A launchctl that answers from a table and records what was asked.
    #[derive(Default)]
    struct Fake {
        loaded: Vec<(&'static str, Option<u32>)>,
        calls: RefCell<Vec<String>>,
    }

    impl Launchctl for Fake {
        fn print(&self, label: &str) -> Option<Job> {
            self.calls.borrow_mut().push(format!("print {label}"));
            self.loaded
                .iter()
                .find(|(l, _)| *l == label)
                .map(|(_, pid)| Job { pid: *pid })
        }
        fn kickstart(&self, label: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("kickstart {label}"));
            Ok(())
        }
        fn bootstrap(&self, plist: &Path) -> anyhow::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("bootstrap {}", plist.display()));
            Ok(())
        }
        fn bootout(&self, label: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("bootout {label}"));
            Ok(())
        }
    }

    fn home_with(plists: &[&str]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let agents = agents_dir(home.path());
        std::fs::create_dir_all(&agents).unwrap();
        for p in plists {
            std::fs::write(agents.join(format!("{p}.plist")), "<plist/>").unwrap();
        }
        home
    }

    #[test]
    fn no_plist_means_no_supervisor() {
        let home = home_with(&[]);
        let lc = Fake::default();
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            Some(7),
            None,
        );
        assert!(matches!(s, Supervisor::None));
        assert!(lc.calls.borrow().is_empty(), "no plist, no launchctl call");
    }

    #[test]
    fn plist_on_disk_but_not_loaded_is_not_a_supervisor() {
        let home = home_with(&["dev.taskstaka"]);
        let lc = Fake::default();
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            None,
            None,
        );
        assert!(matches!(s, Supervisor::None));
    }

    #[test]
    fn running_daemon_is_supervised_only_when_launchd_owns_that_pid() {
        let home = home_with(&["dev.taskstaka"]);
        let lc = Fake {
            loaded: vec![("dev.taskstaka", Some(42))],
            ..Default::default()
        };
        let ours = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            Some(42),
            None,
        );
        assert_eq!(ours.label(), Some("dev.taskstaka"));
        let other = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            Some(43),
            None,
        );
        assert!(
            matches!(other, Supervisor::None),
            "a test daemon on another port is not launchd's"
        );
    }

    #[test]
    fn stopped_daemon_is_supervised_only_for_the_folder_launchd_would_serve() {
        let home = home_with(&["dev.taskstaka"]);
        let lc = Fake {
            loaded: vec![("dev.taskstaka", None)],
            ..Default::default()
        };
        let same = detect_in(
            home.path(),
            &lc,
            Path::new("/todo"),
            Path::new("/todo"),
            None,
            None,
        );
        assert_eq!(same.label(), Some("dev.taskstaka"));
        let other = detect_in(
            home.path(),
            &lc,
            Path::new("/tmp/x"),
            Path::new("/todo"),
            None,
            None,
        );
        assert!(matches!(other, Supervisor::None));
    }

    #[test]
    fn a_non_default_port_is_never_launchds_when_nothing_runs() {
        // launchd's agent serves 7407; `--port 0` (tests) or `--port 7500`
        // cannot be satisfied by kicking it.
        let home = home_with(&["dev.taskstaka"]);
        let lc = Fake {
            loaded: vec![("dev.taskstaka", None)],
            ..Default::default()
        };
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/todo"),
            Path::new("/todo"),
            None,
            Some(0),
        );
        assert!(matches!(s, Supervisor::None));
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/todo"),
            Path::new("/todo"),
            None,
            Some(7407),
        );
        assert_eq!(s.label(), Some("dev.taskstaka"));
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/todo"),
            Path::new("/todo"),
            None,
            None,
        );
        assert_eq!(s.label(), Some("dev.taskstaka"));
    }

    #[test]
    fn homebrew_labels_are_recognised_after_ours() {
        // Homebrew 7 writes sh.brew.<name>; older brews homebrew.mxcl.<name>.
        let home = home_with(&[
            "sh.brew.taskstaka",
            "homebrew.mxcl.taskstaka",
            "dev.taskstaka",
        ]);
        let lc = Fake {
            loaded: vec![
                ("sh.brew.taskstaka", Some(8)),
                ("homebrew.mxcl.taskstaka", Some(9)),
                ("dev.taskstaka", Some(10)),
            ],
            ..Default::default()
        };
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            Some(8),
            None,
        );
        assert_eq!(s.label(), Some("sh.brew.taskstaka"));
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            Some(9),
            None,
        );
        assert_eq!(s.label(), Some("homebrew.mxcl.taskstaka"));
        let s = detect_in(
            home.path(),
            &lc,
            Path::new("/n"),
            Path::new("/n"),
            Some(10),
            None,
        );
        assert_eq!(s.label(), Some("dev.taskstaka"));
    }

    #[test]
    fn the_supervisor_seen_before_a_stop_wins_over_a_fresh_guess() {
        // Two agents loaded for one folder (brew services + setup macos):
        // restart must kick the one that owned the daemon, not the first label.
        let brew = Supervisor::Launchd {
            label: "sh.brew.taskstaka".into(),
        };
        let ours = Supervisor::Launchd {
            label: "dev.taskstaka".into(),
        };
        assert_eq!(brew.clone().or_else(|| ours.clone()), brew);
        assert_eq!(Supervisor::None.or_else(|| ours.clone()), ours);
        assert_eq!(
            Supervisor::None.or_else(|| Supervisor::None),
            Supervisor::None
        );
    }

    #[test]
    fn parses_the_pid_out_of_launchctl_print() {
        let out = "gui/501/dev.taskstaka = {\n\tactive count = 1\n\tpath = /x.plist\n\tstate = running\n\n\tprogram = /x/taskstaka\n\tpid = 4242\n}\n";
        assert_eq!(parse_print(out), Job { pid: Some(4242) });
        let stopped = "gui/501/dev.taskstaka = {\n\tstate = not running\n}\n";
        assert_eq!(parse_print(stopped), Job { pid: None });
    }

    #[test]
    fn domain_is_the_gui_session_of_this_user() {
        assert!(domain().starts_with("gui/"));
        let _ = PathBuf::new();
    }

    /// A systemctl that answers from a table and records what was asked.
    #[derive(Default)]
    struct FakeSystemctl {
        units: std::collections::HashMap<String, Unit>,
        calls: RefCell<Vec<String>>,
    }

    impl Systemctl for FakeSystemctl {
        fn show(&self, unit: &str) -> Option<Unit> {
            self.calls.borrow_mut().push(format!("show {unit}"));
            self.units.get(unit).copied()
        }
        fn daemon_reload(&self) -> anyhow::Result<()> {
            self.calls.borrow_mut().push("daemon-reload".into());
            Ok(())
        }
        fn enable(&self, unit: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("enable {unit}"));
            Ok(())
        }
        fn disable_now(&self, unit: &str) -> anyhow::Result<()> {
            self.calls
                .borrow_mut()
                .push(format!("disable --now {unit}"));
            Ok(())
        }
        fn start(&self, unit: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("start {unit}"));
            Ok(())
        }
        fn stop(&self, unit: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("stop {unit}"));
            Ok(())
        }
        fn restart(&self, unit: &str) -> anyhow::Result<()> {
            self.calls.borrow_mut().push(format!("restart {unit}"));
            Ok(())
        }
    }

    fn home_with_units(units: &[&str]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let dir = units_dir(home.path());
        std::fs::create_dir_all(&dir).unwrap();
        for u in units {
            std::fs::write(dir.join(u), "[Unit]\n").unwrap();
        }
        home
    }

    #[test]
    fn parses_systemctl_show() {
        assert_eq!(
            parse_show("LoadState=loaded\nActiveState=active\nMainPID=4242\n"),
            Some(Unit {
                active: true,
                pid: Some(4242)
            })
        );
        assert_eq!(
            parse_show("LoadState=loaded\nActiveState=inactive\nMainPID=0\n"),
            Some(Unit {
                active: false,
                pid: None
            })
        );
        assert_eq!(
            parse_show("LoadState=not-found\nActiveState=inactive\nMainPID=0\n"),
            None
        );
    }

    #[test]
    fn systemd_owns_the_daemon_only_when_the_unit_runs_that_pid() {
        let home = home_with_units(&[DAEMON_UNIT]);
        let todo = home.path().join("todo");
        let mut sc = FakeSystemctl::default();
        sc.units.insert(
            DAEMON_UNIT.into(),
            Unit {
                active: true,
                pid: Some(77),
            },
        );
        let sup = detect_systemd_in(home.path(), &sc, &todo, &todo, Some(77), None);
        assert_eq!(
            sup,
            Supervisor::Systemd {
                unit: DAEMON_UNIT.into()
            }
        );
        assert_eq!(sup.describe().as_deref(), Some("systemd taskstaka.service"));
        // Another daemon on the port is not systemd's.
        let sup = detect_systemd_in(home.path(), &sc, &todo, &todo, Some(78), None);
        assert_eq!(sup, Supervisor::None);
    }

    #[test]
    fn stopped_daemon_is_systemds_for_the_default_folder_only() {
        let home = home_with_units(&[DAEMON_UNIT]);
        let todo = home.path().join("todo");
        let other = home.path().join("other");
        let mut sc = FakeSystemctl::default();
        sc.units.insert(DAEMON_UNIT.into(), Unit::default());
        assert_eq!(
            detect_systemd_in(home.path(), &sc, &todo, &todo, None, None),
            Supervisor::Systemd {
                unit: DAEMON_UNIT.into()
            }
        );
        assert_eq!(
            detect_systemd_in(home.path(), &sc, &other, &todo, None, None),
            Supervisor::None
        );
        assert_eq!(
            detect_systemd_in(home.path(), &sc, &todo, &todo, None, Some(9000)),
            Supervisor::None,
            "another port is never the unit's"
        );
    }

    #[test]
    fn no_unit_file_means_no_systemctl_call() {
        let home = tempfile::tempdir().unwrap();
        let todo = home.path().join("todo");
        let sc = FakeSystemctl::default();
        assert_eq!(
            detect_systemd_in(home.path(), &sc, &todo, &todo, None, None),
            Supervisor::None
        );
        assert!(sc.calls.borrow().is_empty());
    }
}
