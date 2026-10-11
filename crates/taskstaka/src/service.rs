//! Who keeps the daemon alive. On macOS a launchd agent may own it (installed
//! by `taskstaka setup macos`, or by `brew services`); `stop`/`restart`/`status`
//! then go through `launchctl` instead of racing it with their own daemon.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

/// Our own agent label (written by `setup macos`) and Homebrew's — `sh.brew.*`
/// since Homebrew 7, `homebrew.mxcl.*` before — in the order they are looked for.
pub const LABELS: [&str; 3] = [
    "dev.taskstaka",
    "sh.brew.taskstaka",
    "homebrew.mxcl.taskstaka",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Supervisor {
    /// A loaded launchd agent owns the daemon for this folder.
    Launchd {
        label: String,
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

    pub fn label(&self) -> Option<&str> {
        match self {
            Supervisor::Launchd { label } => Some(label),
            Supervisor::None => None,
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
    if !cfg!(target_os = "macos") {
        return Supervisor::None;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Supervisor::None;
    };
    detect_in(&home, &System, dir, default_dir, running_pid, port)
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
}
