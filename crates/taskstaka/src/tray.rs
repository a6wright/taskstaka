//! `taskstaka tray start|stop|status`: the menu bar item. When `setup macos`
//! installed its launchd agent (`dev.taskstaka.tray`) the commands go through
//! `launchctl`, so the item stays supervised and comes back at login; without
//! an agent they start and stop a plain `taskstaka-tray` process.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::args::TrayCmd;
use crate::service::{self, Launchctl};
use crate::setup_macos::TRAY_LABEL;

/// Where the menu bar item stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// `setup macos` wrote the agent. `loaded`: launchd has it; `pid`: the
    /// item is running (launchd's, or a hand-started one when not loaded).
    Launchd { loaded: bool, pid: Option<u32> },
    /// No agent on disk: whatever `taskstaka-tray` processes are running.
    Plain { pids: Vec<u32> },
}

impl State {
    pub fn pid(&self) -> Option<u32> {
        match self {
            State::Launchd { pid, .. } => *pid,
            State::Plain { pids } => pids.first().copied(),
        }
    }

    pub fn running(&self) -> bool {
        self.pid().is_some()
    }

    fn label(&self) -> Option<&'static str> {
        match self {
            State::Launchd { .. } => Some(TRAY_LABEL),
            State::Plain { .. } => None,
        }
    }
}

/// What `start` or `stop` has to do from a given state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Loaded agent, item quit: launchd starts it again.
    Kickstart,
    /// Agent on disk but booted out: load it (RunAtLoad brings the item up).
    Bootstrap(PathBuf),
    /// Unload the agent; the item exits with it. Login loads it again.
    Bootout,
    /// No agent: run `taskstaka-tray` detached.
    Spawn,
    /// Terminate these processes.
    Kill(Vec<u32>),
    Nothing(&'static str),
}

/// The state on this machine: the agent only counts on macOS.
pub fn detect() -> State {
    let pids = tray_pids();
    if !cfg!(target_os = "macos") {
        return State::Plain { pids };
    }
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) => detect_in(&home, &service::System, pids),
        None => State::Plain { pids },
    }
}

pub fn detect_in(home: &Path, lc: &dyn Launchctl, pids: Vec<u32>) -> State {
    if !service::plist_path(home, TRAY_LABEL).exists() {
        return State::Plain { pids };
    }
    match lc.print(TRAY_LABEL) {
        Some(job) => State::Launchd {
            loaded: true,
            pid: job.pid,
        },
        None => State::Launchd {
            loaded: false,
            pid: pids.first().copied(),
        },
    }
}

pub fn start_action(state: &State, home: &Path) -> Action {
    if state.running() {
        return Action::Nothing("already running");
    }
    match state {
        State::Launchd { loaded: true, .. } => Action::Kickstart,
        State::Launchd { loaded: false, .. } => {
            Action::Bootstrap(service::plist_path(home, TRAY_LABEL))
        }
        State::Plain { .. } => Action::Spawn,
    }
}

pub fn stop_action(state: &State) -> Action {
    match state {
        State::Launchd {
            loaded: true,
            pid: Some(_),
        } => Action::Bootout,
        State::Launchd {
            loaded: false,
            pid: Some(pid),
        } => Action::Kill(vec![*pid]),
        State::Plain { pids } if !pids.is_empty() => Action::Kill(pids.clone()),
        _ => Action::Nothing("not running"),
    }
}

/// One line for `taskstaka tray status`.
pub fn describe(state: &State) -> String {
    match state {
        State::Launchd {
            loaded,
            pid: Some(pid),
        } => format!(
            "running  pid {pid}  {}",
            if *loaded {
                format!("launchd {TRAY_LABEL}")
            } else {
                format!("started by hand; launchd {TRAY_LABEL} installed but not loaded")
            }
        ),
        State::Launchd { loaded: true, .. } => {
            format!("not running  launchd {TRAY_LABEL} loaded; `taskstaka tray start` starts it")
        }
        State::Launchd { loaded: false, .. } => format!(
            "not running  launchd {TRAY_LABEL} installed but not loaded; `taskstaka tray start` loads it"
        ),
        State::Plain { pids } => match pids.first() {
            Some(pid) => format!("running  pid {pid}  (no launchd agent)"),
            None => "not running  (no launchd agent; `taskstaka setup macos` installs one, \
                     `taskstaka tray start` runs it without)"
                .to_string(),
        },
    }
}

pub fn run(cmd: TrayCmd, json: bool) -> Result<()> {
    let state = detect();
    match cmd {
        TrayCmd::Status => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "running": state.running(),
                        "pid": state.pid(),
                        "launchd": state.label(),
                        "loaded": matches!(state, State::Launchd { loaded: true, .. }),
                    })
                );
            } else {
                println!("{}", describe(&state));
            }
            Ok(())
        }
        TrayCmd::Start => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let action = start_action(&state, &home);
            perform(&action)?;
            if let Action::Nothing(why) = action {
                println!("{why}");
                return Ok(());
            }
            let after = wait_until(|s| s.running());
            if after.running() {
                println!("{}", describe(&after));
                Ok(())
            } else {
                bail!("the menu bar item did not come up; see ~/Library/Logs/taskstaka-tray.log")
            }
        }
        TrayCmd::Stop => {
            let action = stop_action(&state);
            perform(&action)?;
            if let Action::Nothing(why) = action {
                println!("{why}");
                return Ok(());
            }
            let after = wait_until(|s| !s.running());
            println!("{}", describe(&after));
            Ok(())
        }
    }
}

fn perform(action: &Action) -> Result<()> {
    let lc = service::System;
    match action {
        Action::Kickstart => lc.kickstart(TRAY_LABEL),
        Action::Bootstrap(plist) => lc.bootstrap(plist),
        Action::Bootout => lc.bootout(TRAY_LABEL),
        Action::Spawn => spawn_tray(),
        Action::Kill(pids) => {
            let mut cmd = std::process::Command::new("kill");
            cmd.arg("-TERM");
            for pid in pids {
                cmd.arg(pid.to_string());
            }
            let out = cmd.output().context("running kill")?;
            if out.status.success() {
                Ok(())
            } else {
                bail!(
                    "kill failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )
            }
        }
        Action::Nothing(_) => Ok(()),
    }
}

/// Poll `detect` for up to ~5 s.
fn wait_until(done: impl Fn(&State) -> bool) -> State {
    let mut state = detect();
    for _ in 0..25 {
        if done(&state) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        state = detect();
    }
    state
}

/// `taskstaka-tray` next to this binary, else on PATH.
fn tray_exe() -> PathBuf {
    let name = format!("taskstaka-tray{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .map(|me| me.with_file_name(&name))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn spawn_tray() -> Result<()> {
    let exe = tray_exe();
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
            "taskstaka-tray is not installed (looked for {}); `cargo install --path crates/tray`, or Homebrew ships it",
            exe.display()
        ),
        Err(e) => Err(e).context("starting taskstaka-tray"),
    }
}

/// Pids of running `taskstaka-tray` processes.
fn tray_pids() -> Vec<u32> {
    if !cfg!(unix) {
        return Vec::new();
    }
    std::process::Command::new("pgrep")
        .args(["-x", "taskstaka-tray"])
        .output()
        .ok()
        .map(|o| parse_pids(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

pub fn parse_pids(out: &str) -> Vec<u32> {
    out.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Job, Launchctl};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    #[derive(Default)]
    struct Fake {
        loaded: Option<Option<u32>>,
        calls: RefCell<Vec<String>>,
    }

    impl Launchctl for Fake {
        fn print(&self, label: &str) -> Option<Job> {
            self.calls.borrow_mut().push(format!("print {label}"));
            self.loaded.map(|pid| Job { pid })
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

    fn home(with_plist: bool) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        if with_plist {
            let agents = crate::service::agents_dir(home.path());
            std::fs::create_dir_all(&agents).unwrap();
            std::fs::write(agents.join("dev.taskstaka.tray.plist"), "<plist/>").unwrap();
        }
        home
    }

    #[test]
    fn without_an_agent_the_tray_is_a_plain_process() {
        let h = home(false);
        let lc = Fake::default();
        let idle = detect_in(h.path(), &lc, vec![]);
        assert_eq!(idle, State::Plain { pids: vec![] });
        assert_eq!(start_action(&idle, h.path()), Action::Spawn);
        assert_eq!(stop_action(&idle), Action::Nothing("not running"));
        assert!(lc.calls.borrow().is_empty(), "no plist, no launchctl");

        let busy = detect_in(h.path(), &lc, vec![31, 32]);
        assert_eq!(
            start_action(&busy, h.path()),
            Action::Nothing("already running")
        );
        assert_eq!(stop_action(&busy), Action::Kill(vec![31, 32]));
    }

    #[test]
    fn a_loaded_agent_with_a_pid_is_running_and_stops_by_bootout() {
        let h = home(true);
        let lc = Fake {
            loaded: Some(Some(90)),
            ..Default::default()
        };
        let s = detect_in(h.path(), &lc, vec![90]);
        assert_eq!(
            s,
            State::Launchd {
                loaded: true,
                pid: Some(90)
            }
        );
        assert_eq!(
            start_action(&s, h.path()),
            Action::Nothing("already running")
        );
        assert_eq!(stop_action(&s), Action::Bootout);
    }

    #[test]
    fn a_loaded_agent_after_quit_starts_by_kickstart() {
        let h = home(true);
        let lc = Fake {
            loaded: Some(None),
            ..Default::default()
        };
        let s = detect_in(h.path(), &lc, vec![]);
        assert_eq!(
            s,
            State::Launchd {
                loaded: true,
                pid: None
            }
        );
        assert_eq!(start_action(&s, h.path()), Action::Kickstart);
        assert_eq!(stop_action(&s), Action::Nothing("not running"));
    }

    #[test]
    fn an_installed_but_unloaded_agent_starts_by_bootstrap() {
        // `taskstaka tray stop` boots the agent out; `start` loads it again
        // (RunAtLoad brings the item up).
        let h = home(true);
        let lc = Fake::default();
        let s = detect_in(h.path(), &lc, vec![]);
        assert_eq!(
            s,
            State::Launchd {
                loaded: false,
                pid: None
            }
        );
        let plist = crate::service::plist_path(h.path(), "dev.taskstaka.tray");
        assert_eq!(start_action(&s, h.path()), Action::Bootstrap(plist));
        assert_eq!(stop_action(&s), Action::Nothing("not running"));
    }

    #[test]
    fn a_hand_started_tray_next_to_an_unloaded_agent_is_killed_not_booted_out() {
        let h = home(true);
        let lc = Fake::default();
        let s = detect_in(h.path(), &lc, vec![77]);
        assert_eq!(
            s,
            State::Launchd {
                loaded: false,
                pid: Some(77)
            }
        );
        assert_eq!(
            start_action(&s, h.path()),
            Action::Nothing("already running")
        );
        assert_eq!(stop_action(&s), Action::Kill(vec![77]));
    }

    #[test]
    fn parses_pgrep_output() {
        assert_eq!(parse_pids("123\n456\n"), vec![123, 456]);
        assert_eq!(parse_pids(""), Vec::<u32>::new());
        assert_eq!(parse_pids(" 9 \nnope\n"), vec![9]);
    }

    #[test]
    fn describes_each_state_for_a_person() {
        let plain_off = describe(&State::Plain { pids: vec![] });
        assert!(plain_off.starts_with("not running"), "{plain_off}");
        assert!(plain_off.contains("setup macos"), "{plain_off}");
        let plain_on = describe(&State::Plain { pids: vec![5] });
        assert!(plain_on.starts_with("running  pid 5"), "{plain_on}");
        let quit = describe(&State::Launchd {
            loaded: true,
            pid: None,
        });
        assert!(quit.contains("tray start"), "{quit}");
        let on = describe(&State::Launchd {
            loaded: true,
            pid: Some(8),
        });
        assert!(
            on.starts_with("running  pid 8") && on.contains("launchd"),
            "{on}"
        );
        let _ = PathBuf::new();
    }
}
