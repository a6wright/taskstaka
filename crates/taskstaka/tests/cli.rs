//! End-to-end: the resurface flow through the real binary in a temp dir.

use assert_cmd::Command;
use predicates::prelude::*;

struct Env {
    dir: tempfile::TempDir,
    /// Machine settings for this run, so tests never read or write the real ones.
    cfg: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let env = Env { dir, cfg };
        env.cmd().arg("init").assert().success();
        env
    }

    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("taskstaka").unwrap();
        c.env("TASKSTAKA_DIR", self.dir.path())
            .env("TASKSTAKA_CONFIG_DIR", self.cfg.path())
            .env("TASKSTAKA_DIRECT", "1")
            .env_remove("TASKSTAKA_CONTEXT")
            .env_remove("TASKSTAKA_PORT");
        c
    }

    /// Run `add …` and return the new id.
    fn add(&self, args: &[&str]) -> String {
        let out = self.cmd().arg("add").args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        stdout.split_whitespace().nth(1).unwrap().to_string()
    }

    fn next(&self) -> String {
        let out = self
            .cmd()
            .args(["next", "--explain", "--all"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}

#[test]
fn story_is_hidden_while_blocked_and_comes_back_when_subtasks_close() {
    let env = Env::new();
    let story = env.add(&["Ship auth story", "-c", "work"]);
    let jwt = env.add(&["Fix JWT expiry", "-p", &story]);
    let mig = env.add(&["Write migration", "-p", &story, "--due", "+2d"]);

    let next = env.next();
    assert!(
        !next.contains(&story),
        "blocked story must not be actionable:\n{next}"
    );
    assert!(next.contains(&jwt) && next.contains(&mig));
    // Siblings are parallel streams: both appear, in outline order, with
    // their badges (due soon on the migration) but no ranking between them.
    assert!(next.contains("due soon"), "{next}");

    env.cmd()
        .args(["start", &jwt])
        .assert()
        .success()
        .stdout(predicate::str::contains("todo → doing"));
    env.cmd().args(["done", &jwt]).assert().success();

    // Closing the story with an open child is refused.
    env.cmd()
        .args(["done", &story])
        .assert()
        .failure()
        .stderr(predicate::str::contains("open descendant"));

    env.cmd()
        .args(["done", &mig])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "is ready: all its prerequisites are done",
        ));

    let next = env.next();
    let first_line = next
        .lines()
        .find(|l| l.starts_with(&story))
        .expect("story listed");
    // Finishing the last prerequisite in hand starts the story itself (ADR 0010).
    assert!(
        first_line.contains("in hand"),
        "story should be in hand:\n{next}"
    );
    assert!(next.contains("ready: all prerequisites done"));
    // It is the plate at the very top.
    assert!(
        next.lines()
            .any(|l| l.contains("▶ in hand") && l.contains(&story)),
        "{next}"
    );
}

#[test]
fn someday_and_waiting_and_prefix_resolution() {
    let env = Env::new();
    let zig = env.add(&["Learn Zig", "--someday"]);
    let pass = env.add(&["Renew passport", "-c", "personal", "--priority", "high"]);

    let next = env.next();
    assert!(!next.contains(&zig), "someday is not actionable:\n{next}");
    assert!(next.contains("high"));

    env.cmd()
        .args(["wait", &pass])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
    env.cmd()
        .args(["wait", &pass, "--until", "+5d", "Service", "Canada"])
        .assert()
        .success()
        .stdout(predicate::str::contains("todo → waiting"));

    let out = env.cmd().args(["next", "-c", "personal"]).output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Nothing you can do right now"), "{text}");
    assert!(text.contains("1 waiting on people"), "{text}");

    // Prefix resolution.
    env.cmd()
        .args(["show", &pass[..3]])
        .assert()
        .success()
        .stdout(predicate::str::contains("Renew passport"));
    env.cmd()
        .args(["show", "zzzzzz"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no task matches"));

    // index.md reflects state and is Obsidian-linkable.
    let index = std::fs::read_to_string(env.dir.path().join("index.md")).unwrap();
    assert!(index.contains("## Someday"));
    assert!(index.contains("waiting on Service Canada"));
    assert!(index.contains(&format!("[[{pass}-renew-passport|Renew passport]]")));
}

#[test]
fn log_lists_what_was_finished_today_as_an_outline() {
    let env = Env::new();
    let story = env.add(&["Ship auth story"]);
    let mig = env.add(&["Write migration", "-p", &story]);
    let step = env.add(&["Drop old column", "-p", &mig]);
    env.cmd().args(["done", &step]).assert().success();
    env.cmd()
        .args(["log", "--on", "today"])
        .assert()
        .success()
        .stdout(predicate::str::contains("· today  (1 finished)"))
        .stdout(predicate::str::contains("  Ship auth story\n"))
        .stdout(predicate::str::contains(
            "    ✓ Drop old column  › Write migration",
        ));
    env.cmd()
        .args(["log", "--on", "yesterday"])
        .assert()
        .success()
        .stdout(predicate::str::contains("· yesterday\n  nothing"));
    // Finishing the parent too: it nests the step under it, no chain.
    env.cmd().args(["done", &mig]).assert().success();
    env.cmd()
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "    ✓ Write migration\n      ✓ Drop old column\n",
        ));
    let out = env.cmd().args(["log", "--json"]).output().unwrap();
    let days: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(days.as_array().unwrap().len(), 7, "a week by default");
    assert_eq!(days[0]["count"], 2);
    assert_eq!(days[0]["finished"], 2);
    assert_eq!(days[0]["stories"][0]["items"][1]["depth"], 1);
    // People traffic shows under the task it happened on, finished or not.
    let ask = env.add(&["Ask Dana for the keys", "-p", &story]);
    env.cmd().args(["wait", &ask, "Dana"]).assert().success();
    env.cmd()
        .args(["chase", &ask, "pinged", "on", "Teams"])
        .assert()
        .success();
    env.cmd()
        .args(["log", "--on", "today"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(2 finished, 1 touched)"))
        .stdout(predicate::str::contains("    ✉ Ask Dana for the keys\n"))
        .stdout(predicate::str::is_match(r"\d\d:\d\d waiting on Dana; follow up").unwrap())
        .stdout(
            predicate::str::is_match(r"\d\d:\d\d chased Dana; follow up [-0-9]+: pinged on Teams")
                .unwrap(),
        );
    env.cmd()
        .args(["log", "--since", "tomorrow"])
        .assert()
        .failure();
}

#[test]
fn flags_after_title_are_flags_not_title() {
    let env = Env::new();
    let id = env.add(&["Buy", "milk", "-c", "personal"]);
    env.cmd().args(["show", &id]).assert().success().stdout(
        predicate::str::contains("Buy milk\n").and(predicate::str::contains("context   personal")),
    );
}

/// Daemon mode: the first command auto-starts a daemon on a free port, later
/// commands reuse it, `stop` shuts it down. Cleans up even on failure.
struct DaemonEnv {
    dir: tempfile::TempDir,
    cfg: tempfile::TempDir,
    /// A HOME of its own, so a developer's real launchd agents
    /// (`~/Library/LaunchAgents/dev.taskstaka.plist`) are never seen.
    home: tempfile::TempDir,
}

impl DaemonEnv {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let env = DaemonEnv { dir, cfg, home };
        env.cmd().arg("init").assert().success();
        env
    }

    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("taskstaka").unwrap();
        c.env("TASKSTAKA_DIR", self.dir.path())
            .env("TASKSTAKA_CONFIG_DIR", self.cfg.path())
            .env("HOME", self.home.path())
            .env("TASKSTAKA_PORT", "0")
            .env("TASKSTAKA_NO_NOTIFY", "true")
            .env_remove("TASKSTAKA_DIRECT")
            .env_remove("TASKSTAKA_CONTEXT");
        c
    }
}

impl Drop for DaemonEnv {
    fn drop(&mut self) {
        let _ = self.cmd().arg("stop").output();
    }
}

#[test]
fn daemon_mode_autostarts_reuses_and_stops() {
    let env = DaemonEnv::new();
    env.cmd()
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));

    // First real command starts the daemon.
    let out = env.cmd().args(["add", "Hello daemon"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    let status = env.cmd().args(["status"]).output().unwrap();
    let status = String::from_utf8(status.stdout).unwrap();
    assert!(status.starts_with("running"), "{status}");
    let pid1 = status.split_whitespace().nth(3).unwrap().to_string();

    // Second command reuses the same daemon (same pid) and sees the task.
    env.cmd()
        .args(["show", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Hello daemon"));
    let status = env.cmd().args(["status"]).output().unwrap();
    let status = String::from_utf8(status.stdout).unwrap();
    assert_eq!(status.split_whitespace().nth(3).unwrap(), pid1, "{status}");

    // While the daemon holds the lock, --direct is refused with a hint.
    env.cmd()
        .args(["--direct", "ls"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("taskstaka stop"));

    env.cmd()
        .args(["stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("asked the daemon"));
    for _ in 0..40 {
        let out = env.cmd().args(["status"]).output().unwrap();
        if String::from_utf8_lossy(&out.stdout).contains("not running") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("daemon did not stop");
}

#[test]
fn init_is_idempotent() {
    let env = Env::new();
    env.cmd()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("already initialized"));
}

/// The scenario that motivated P8: ask A, A says talk to B, B goes quiet.
#[test]
fn dns_scenario_waiting_chase_handoff_and_reply() {
    let env = Env::new();
    let dns = env.add(&["Get DNS address"]);
    let ask = env.add(&["What's possible for wildcard?", "-p", &dns, "-w", "A"]);

    // Follow-up defaulted; the parent knows who it is waiting on.
    env.cmd()
        .args(["show", &ask])
        .assert()
        .success()
        .stdout(predicate::str::contains("waiting   A (until"));
    let next = env.next();
    assert!(!next.contains(&dns), "DNS is blocked:\n{next}");
    env.cmd().args(["waiting"]).assert().success().stdout(
        predicate::str::contains("A  (1 item").and(predicate::str::contains("Get DNS address")),
    );

    // Handoff to B, then chase B twice (second one dedupes only via the API; CLI always chases).
    env.cmd()
        .args([
            "edit",
            &ask,
            "--waiting-on",
            "B",
            "--ref",
            "https://teams.example/t/1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("handed off: A → B"));
    env.cmd()
        .args(["chase", &ask, "pinged", "again"])
        .assert()
        .success()
        .stdout(predicate::str::contains("chased B (1×); follow up again"));
    env.cmd()
        .args(["waiting", "--json"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"person\": \"B\"")
                .and(predicate::str::contains("\"chases\": 1")),
        );

    // B answers: the subtask is todo again, journaled, and the story is still blocked
    // (by the now-actionable subtask) until it is done.
    env.cmd()
        .args(["reply", &ask, "answered", "use", "the", "wildcard", "cert"])
        .assert()
        .success()
        .stdout(predicate::str::contains("B replied (answered)"));
    env.cmd().args(["show", &ask]).assert().success().stdout(
        predicate::str::contains("status    todo")
            .and(predicate::str::contains("B replied: use the wildcard cert")),
    );
    env.cmd()
        .args(["waiting"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not waiting on anyone"));
}

#[test]
fn restart_swaps_the_daemon_process() {
    let env = DaemonEnv::new();
    env.cmd().args(["add", "before restart"]).assert().success();
    let pid = |env: &DaemonEnv| -> String {
        let out = env.cmd().args(["status"]).output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .nth(3)
            .unwrap()
            .to_string()
    };
    let before = pid(&env);
    env.cmd()
        .args(["restart"])
        .assert()
        .success()
        .stdout(predicate::str::contains("restarted daemon"));
    let after = pid(&env);
    assert_ne!(before, after, "a new process serves the same directory");
    env.cmd()
        .args(["ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("before restart"));
}

#[test]
fn restart_to_moves_the_daemon_and_copies_the_notes() {
    let env = DaemonEnv::new();
    env.cmd().args(["add", "carry me over"]).assert().success();
    let parent = tempfile::tempdir().unwrap();
    let moved = parent.path().join("moved");

    env.cmd()
        .args(["restart", "--to"])
        .arg(&moved)
        .assert()
        .success()
        .stdout(predicate::str::contains("copied 1 note"));

    // The old folder is untouched, the new one serves the task, and the
    // setting points at it for the next start.
    assert_eq!(
        std::fs::read_dir(env.dir.path().join("notes"))
            .unwrap()
            .count(),
        1
    );
    env.cmd()
        .arg("--dir")
        .arg(&moved)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("carry me over"));
    let saved = std::fs::read_to_string(env.cfg.path().join("settings.json")).unwrap();
    assert!(saved.contains("moved"), "{saved}");

    // A folder that is not empty and not a taskstaka folder is refused
    // before anything is stopped.
    let junk = tempfile::tempdir().unwrap();
    std::fs::write(junk.path().join("keep.txt"), "x").unwrap();
    env.cmd()
        .arg("--dir")
        .arg(&moved)
        .args(["restart", "--to"])
        .arg(junk.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not empty"));
    env.cmd()
        .arg("--dir")
        .arg(&moved)
        .arg("ls")
        .assert()
        .success();
    env.cmd()
        .arg("--dir")
        .arg(&moved)
        .arg("stop")
        .assert()
        .success();
}

/// Two stores at once, neither naming a port: the second must not fail
/// because the first (or the user's real daemon) owns 7407.
#[test]
fn second_store_falls_back_to_a_free_port() {
    let a = DaemonEnv::new();
    let b = DaemonEnv::new();
    let mut ca = a.cmd();
    ca.env_remove("TASKSTAKA_PORT");
    ca.args(["add", "in A"]).assert().success();
    let mut cb = b.cmd();
    cb.env_remove("TASKSTAKA_PORT");
    cb.args(["add", "in B"]).assert().success();
    let port = |env: &DaemonEnv| -> String {
        let out = env.cmd().args(["status"]).output().unwrap();
        let s = String::from_utf8(out.stdout).unwrap();
        s.split_whitespace()
            .nth(1)
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .to_string()
    };
    assert_ne!(
        port(&a),
        port(&b),
        "each store has its own daemon on its own port"
    );
    b.cmd()
        .args(["ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("in B"));
}

#[test]
fn notifications_setting_persists_and_defaults_on() {
    let env = Env::new();
    assert!(!env.cfg.path().join("settings.json").exists());
    env.cmd()
        .args(["notifications"])
        .assert()
        .success()
        .stdout(predicate::str::contains("notifications are on"));
    env.cmd()
        .args(["notifications", "off"])
        .assert()
        .success()
        .stdout(predicate::str::contains("notifications are off"));
    env.cmd()
        .args(["notifications"])
        .assert()
        .success()
        .stdout(predicate::str::contains("notifications are off"));
    assert!(env.cfg.path().join("settings.json").exists());
}

#[test]
fn quick_makes_a_root_task_with_project_and_a_wait() {
    let env = Env::new();
    env.cmd()
        .args(["quick", "#devops", "renew", "the", "cert", "@Priya", "+3d"])
        .assert()
        .success();
    env.cmd().args(["quick", "buy milk"]).assert().success();
    env.cmd().args(["quick", "#x"]).assert().failure();
    let out = env.cmd().args(["ls", "--json"]).output().unwrap();
    let all: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let all = all.as_array().unwrap();
    let cert = all.iter().find(|t| t["title"] == "renew the cert").unwrap();
    assert_eq!(cert["project"], "devops");
    assert_eq!(cert["waiting_on"], "Priya");
    assert!(all.iter().all(|t| t.get("parent").is_none()));
    assert!(all.iter().any(|t| t["title"] == "buy milk"));
}

#[test]
fn status_json_reports_the_supervisor() {
    let env = DaemonEnv::new();
    env.cmd()
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"supervisor\":null"));
}

#[cfg(target_os = "linux")]
#[test]
fn setup_linux_dry_run_describes_units_and_the_gnome_key_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("taskstaka")
        .unwrap()
        .env("HOME", home.path())
        .env("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")
        .args(["setup", "linux", "--dry-run", "--hotkey", "ctrl+alt+space"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("would install daemon"), "{text}");
    assert!(text.contains("systemd/user/taskstaka.service"), "{text}");
    // With taskstaka-tray built next to the CLI the key is planned in GNOME's
    // spelling; without it (a fresh CI target dir) setup says what to install.
    if text.contains("would install tray") {
        assert!(text.contains("<Control><Alt>space"), "{text}");
    } else {
        assert!(text.contains("no taskstaka-tray beside"), "{text}");
    }
    assert!(!home.path().join(".config").exists());
}

#[cfg(target_os = "macos")]
#[test]
fn setup_macos_dry_run_describes_both_agents_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("taskstaka")
        .unwrap()
        .env("HOME", home.path())
        .args(["setup", "macos", "--dry-run", "--hotkey", "ctrl+alt+space"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("would install daemon"), "{text}");
    assert!(text.contains("dev.taskstaka.plist"), "{text}");
    assert!(!home.path().join("Library/LaunchAgents").exists());
}

#[test]
fn setup_claude_dry_run_names_both_skills_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("taskstaka")
        .unwrap()
        .env("HOME", home.path())
        .args(["setup", "claude", "--dry-run", "--skip-mcp"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains(".claude/skills/taskstaka/SKILL.md"), "{text}");
    assert!(text.contains(".claude/skills/today/SKILL.md"), "{text}");
    assert!(!home.path().join(".claude").exists());
}
