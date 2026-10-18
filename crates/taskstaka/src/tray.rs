//! `taskstaka tray start|stop|status|quick`: the menu bar item. When `setup
//! macos` installed its launchd agent (`dev.taskstaka.tray`) or `setup linux`
//! its systemd unit (`taskstaka-tray.service`) the commands go through
//! `launchctl` / `systemctl --user`, so the item stays supervised and comes
//! back at login; without one they start and stop a plain `taskstaka-tray`
//! process. `quick` toggles the running item's panel (what the desktop key
//! runs).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::args::TrayCmd;
use crate::service::{self, Launchctl, Systemctl, TRAY_UNIT};
use crate::setup_macos::TRAY_LABEL;

/// Where the menu bar item stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// `setup macos` wrote the agent. `loaded`: launchd has it; `pid`: the
    /// item is running (launchd's, or a hand-started one when not loaded).
    Launchd { loaded: bool, pid: Option<u32> },
    /// `setup linux` wrote the unit. `active`: systemd is running it;
    /// `pid`: the item is running (systemd's, or a hand-started one).
    Systemd { active: bool, pid: Option<u32> },
    /// No agent on disk: whatever `taskstaka-tray` processes are running.
    Plain { pids: Vec<u32> },
}

impl State {
    pub fn pid(&self) -> Option<u32> {
        match self {
            State::Launchd { pid, .. } | State::Systemd { pid, .. } => *pid,
            State::Plain { pids } => pids.first().copied(),
        }
    }

    pub fn running(&self) -> bool {
        self.pid().is_some()
    }

    /// The agent label or unit name.
    fn label(&self) -> Option<&'static str> {
        match self {
            State::Launchd { .. } => Some(TRAY_LABEL),
            State::Systemd { .. } => Some(TRAY_UNIT),
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
    /// `systemctl --user start` the unit.
    UnitStart,
    /// `systemctl --user stop` the unit; login starts it again.
    UnitStop,
    /// No agent: run `taskstaka-tray` detached.
    Spawn,
    /// Terminate these processes.
    Kill(Vec<u32>),
    Nothing(&'static str),
}

/// The state on this machine: the launchd agent counts on macOS, the
/// systemd unit on Linux.
pub fn detect() -> State {
    let pids = tray_pids();
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return State::Plain { pids };
    };
    if cfg!(target_os = "macos") {
        detect_in(&home, &service::System, pids)
    } else if cfg!(target_os = "linux") {
        detect_systemd_in(&home, &service::System, pids)
    } else {
        State::Plain { pids }
    }
}

pub fn detect_systemd_in(home: &Path, sc: &dyn Systemctl, pids: Vec<u32>) -> State {
    if !service::unit_path(home, TRAY_UNIT).exists() {
        return State::Plain { pids };
    }
    match sc.show(TRAY_UNIT) {
        Some(unit) if unit.active => State::Systemd {
            active: true,
            pid: unit.pid.or_else(|| pids.first().copied()),
        },
        _ => State::Systemd {
            active: false,
            pid: pids.first().copied(),
        },
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
        State::Systemd { .. } => Action::UnitStart,
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
        State::Systemd {
            active: true,
            pid: Some(_),
        } => Action::UnitStop,
        State::Systemd {
            active: false,
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
        State::Systemd {
            active,
            pid: Some(pid),
        } => format!(
            "running  pid {pid}  {}",
            if *active {
                format!("systemd {TRAY_UNIT}")
            } else {
                format!("started by hand; systemd {TRAY_UNIT} installed but not running")
            }
        ),
        State::Systemd { .. } => {
            format!("not running  systemd {TRAY_UNIT} installed; `taskstaka tray start` starts it")
        }
        State::Plain { pids } => match pids.first() {
            Some(pid) => format!("running  pid {pid}  (no {SUPERVISOR})"),
            None => format!(
                "not running  (no {SUPERVISOR}; `taskstaka {SETUP}` installs one, \
                 `taskstaka tray start` runs it without)"
            ),
        },
    }
}

/// What supervises the item on this OS, for messages.
const SUPERVISOR: &str = if cfg!(target_os = "linux") {
    "systemd unit"
} else {
    "launchd agent"
};
const SETUP: &str = if cfg!(target_os = "linux") {
    "setup linux"
} else {
    "setup macos"
};
const LOG_HINT: &str = if cfg!(target_os = "linux") {
    "journalctl --user -u taskstaka-tray"
} else {
    "~/Library/Logs/taskstaka-tray.log"
};

pub fn run(cmd: TrayCmd, json: bool, dir: &Path, port: Option<u16>) -> Result<()> {
    let state = detect();
    match cmd {
        TrayCmd::Quick => quick(&state, dir, port),
        TrayCmd::Status => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "running": state.running(),
                        "pid": state.pid(),
                        "supervisor": state.label(),
                        "loaded": matches!(
                            state,
                            State::Launchd { loaded: true, .. } | State::Systemd { .. }
                        ),
                    })
                );
            } else {
                println!("{}", describe(&state));
            }
            Ok(())
        }
        TrayCmd::Start => {
            let action = start_action(&state, &home());
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
                bail!("the menu bar item did not come up; see {LOG_HINT}")
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

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Toggle the running item's panel over the control socket. No item: bring
/// one up and ask again; none to bring up: the quick page in the browser.
fn quick(state: &State, dir: &Path, port: Option<u16>) -> Result<()> {
    use taskstaka_client::tray::{TOGGLE, send};
    if send(TOGGLE).is_ok() {
        return Ok(());
    }
    if !state.running() {
        let action = start_action(state, &home());
        if let Err(e) = perform(&action) {
            eprintln!("taskstaka: {e:#}");
            return open_quick_page(dir, port);
        }
    }
    // The item takes a moment to bind its socket.
    for _ in 0..25 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if send(TOGGLE).is_ok() {
            return Ok(());
        }
    }
    open_quick_page(dir, port)
}

fn open_quick_page(dir: &Path, port: Option<u16>) -> Result<()> {
    let client = taskstaka_client::Client::connect(dir, port)?;
    let url = format!("{}/quick", client.base());
    open::that_detached(&url).with_context(|| format!("opening {url}"))
}

fn perform(action: &Action) -> Result<()> {
    let lc = service::System;
    match action {
        Action::Kickstart => lc.kickstart(TRAY_LABEL),
        Action::Bootstrap(plist) => lc.bootstrap(plist),
        Action::Bootout => lc.bootout(TRAY_LABEL),
        Action::UnitStart => Systemctl::start(&lc, TRAY_UNIT),
        Action::UnitStop => Systemctl::stop(&lc, TRAY_UNIT),
        Action::Spawn => spawn_tray(),
        Action::Kill(pids) => kill(pids),
        Action::Nothing(_) => Ok(()),
    }
}

/// SIGTERM these item processes.
pub fn kill(pids: &[u32]) -> Result<()> {
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

    struct FakeSystemctl(Option<crate::service::Unit>);

    impl crate::service::Systemctl for FakeSystemctl {
        fn show(&self, _unit: &str) -> Option<crate::service::Unit> {
            self.0
        }
        fn daemon_reload(&self) -> anyhow::Result<()> {
            Ok(())
        }
        fn enable(&self, _unit: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn disable_now(&self, _unit: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn start(&self, _unit: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stop(&self, _unit: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn restart(&self, _unit: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn home_with_unit() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let units = crate::service::units_dir(home.path());
        std::fs::create_dir_all(&units).unwrap();
        std::fs::write(units.join(TRAY_UNIT), "[Unit]\n").unwrap();
        home
    }

    #[test]
    fn a_systemd_unit_starts_and_stops_through_systemctl() {
        let h = home_with_unit();
        let running = FakeSystemctl(Some(crate::service::Unit {
            active: true,
            pid: Some(40),
        }));
        let s = detect_systemd_in(h.path(), &running, vec![40]);
        assert_eq!(
            s,
            State::Systemd {
                active: true,
                pid: Some(40)
            }
        );
        assert_eq!(
            start_action(&s, h.path()),
            Action::Nothing("already running")
        );
        assert_eq!(stop_action(&s), Action::UnitStop);
        assert!(describe(&s).contains("systemd taskstaka-tray.service"));

        // Quit, or `tray stop`: the unit is inactive; start goes through it.
        let idle = FakeSystemctl(Some(crate::service::Unit::default()));
        let s = detect_systemd_in(h.path(), &idle, vec![]);
        assert_eq!(
            s,
            State::Systemd {
                active: false,
                pid: None
            }
        );
        assert_eq!(start_action(&s, h.path()), Action::UnitStart);
        assert_eq!(stop_action(&s), Action::Nothing("not running"));

        // Started by hand beside an idle unit: killed, not stopped via systemd.
        let s = detect_systemd_in(h.path(), &idle, vec![51]);
        assert_eq!(stop_action(&s), Action::Kill(vec![51]));
        assert!(describe(&s).contains("started by hand"));

        // No unit file: plain, and systemctl is never asked.
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(
            detect_systemd_in(plain.path(), &idle, vec![]),
            State::Plain { pids: vec![] }
        );
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
        assert!(plain_off.contains(SETUP), "{plain_off}");
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
