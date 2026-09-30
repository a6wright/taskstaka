//! End-to-end: the resurface flow through the real binary in a temp dir.

use assert_cmd::Command;
use predicates::prelude::*;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let env = Env { dir };
        env.cmd().arg("init").assert().success();
        env
    }

    fn cmd(&self) -> Command {
        let mut c = Command::cargo_bin("tododo").unwrap();
        c.env("TODODO_DIR", self.dir.path())
            .env_remove("TODODO_CONTEXT");
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
    // Both children are due-soon/normal; the due one ranks first.
    assert!(
        next.find(&mig).unwrap() < next.find(&jwt).unwrap(),
        "{next}"
    );

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
            "is back: all its subtasks are done",
        ));

    let next = env.next();
    let first_line = next
        .lines()
        .find(|l| l.starts_with(&story))
        .expect("story listed");
    assert!(
        first_line.contains("back"),
        "story should be in the resurfaced tier:\n{next}"
    );
    assert!(next.contains("resurfaced: all subtasks done"));
    // It is the first row.
    let rows: Vec<&str> = next
        .lines()
        .filter(|l| l.len() > 6 && l.as_bytes()[6] == b' ' && !l.starts_with(' '))
        .collect();
    assert!(rows[0].starts_with(&story), "{next}");
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
    assert!(text.contains("Nothing actionable"), "{text}");
    assert!(text.contains("1 waiting"), "{text}");

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
fn flags_after_title_are_flags_not_title() {
    let env = Env::new();
    let id = env.add(&["Buy", "milk", "-c", "personal"]);
    env.cmd().args(["show", &id]).assert().success().stdout(
        predicate::str::contains("Buy milk\n").and(predicate::str::contains("context   personal")),
    );
}
