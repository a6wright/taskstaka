//! Speak raw MCP (newline-delimited JSON-RPC over stdio) to `taskstaka mcp`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

struct Session {
    dir: tempfile::TempDir,
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Session {
    fn start() -> Session {
        let dir = tempfile::tempdir().unwrap();
        let bin = assert_cmd::cargo::cargo_bin("taskstaka");
        let ok = Command::new(&bin)
            .args(["init"])
            .env("TASKSTAKA_DIR", dir.path())
            .status()
            .unwrap();
        assert!(ok.success());

        let mut child = Command::new(&bin)
            .arg("mcp")
            .env("TASKSTAKA_DIR", dir.path())
            .env("TASKSTAKA_PORT", "0")
            .env("TASKSTAKA_NO_NOTIFY", "true")
            .env_remove("TASKSTAKA_DIRECT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut s = Session {
            dir,
            child,
            reader,
            next_id: 1,
        };
        let init = s.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        assert_eq!(init["serverInfo"]["name"], "taskstaka");
        s.notify("notifications/initialized");
        s
    }

    fn send(&mut self, msg: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str) {
        self.send(json!({"jsonrpc": "2.0", "method": method}));
    }

    /// Send a request and return its `result` (panics on a JSON-RPC error).
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.try_request(method, params)
            .unwrap_or_else(|e| panic!("{method} failed: {e}"))
    }

    fn try_request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).unwrap();
            assert!(n > 0, "mcp process closed stdout");
            let msg: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if msg["id"] == json!(id) {
                if let Some(e) = msg.get("error") {
                    return Err(e.clone());
                }
                return Ok(msg["result"].clone());
            }
        }
    }

    fn call(&mut self, tool: &str, args: Value) -> Result<String, Value> {
        let r = self.try_request("tools/call", json!({"name": tool, "arguments": args}))?;
        if r["isError"] == json!(true) {
            return Err(r.clone());
        }
        Ok(r["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = Command::new(assert_cmd::cargo::cargo_bin("taskstaka"))
            .args(["stop"])
            .env("TASKSTAKA_DIR", self.dir.path())
            .output();
    }
}

#[test]
fn tools_and_resources_round_trip() {
    let mut s = Session::start();

    let tools = s.request("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "next",
        "add_task",
        "get_task",
        "set_status",
        "update_task",
        "move_task",
        "touch",
        "tree",
        "list_tasks",
        "nudges",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool {expected}: {names:?}"
        );
    }

    let out = s
        .call(
            "add_task",
            json!({"title": "Ship auth story", "context": "work"}),
        )
        .unwrap();
    let out: Value = serde_json::from_str(&out).unwrap();
    let story = out["primary"].as_str().unwrap().to_string();

    let out = s
        .call("add_task", json!({"title": "Fix JWT", "parent": story}))
        .unwrap();
    let kid = serde_json::from_str::<Value>(&out).unwrap()["primary"]
        .as_str()
        .unwrap()
        .to_string();

    let next: Value = serde_json::from_str(&s.call("next", json!({})).unwrap()).unwrap();
    assert_eq!(next["summary"]["blocked"], 1);
    assert_eq!(next["items"][0]["id"], kid);

    // Closing the child resurfaces the parent; the event is in the outcome.
    let out = s
        .call("set_status", json!({"id": &kid[..3], "status": "done"}))
        .unwrap();
    assert!(out.contains("resurfaced"), "{out}");

    let text = s.request("resources/read", json!({"uri": "taskstaka://next"}));
    let text = text["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("[resurfaced] Ship auth story"), "{text}");

    let index = s.request("resources/read", json!({"uri": "taskstaka://index"}));
    assert!(
        index["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Ship auth story")
    );

    // Errors come back as tool errors, not protocol crashes.
    let err = s
        .call("set_status", json!({"id": "zzzzzz", "status": "done"}))
        .unwrap_err();
    assert!(err.to_string().contains("no task matches"), "{err}");
    let err = s
        .call("set_status", json!({"id": &story, "status": "waiting"}))
        .unwrap_err();
    assert!(err.to_string().contains("waiting needs a reason"), "{err}");

    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn agent_loop_waiting_chase_dedupe_redirect_and_answer() {
    let mut s = Session::start();
    let story = serde_json::from_str::<Value>(
        &s.call("add_task", json!({"title": "Get DNS address"}))
            .unwrap(),
    )
    .unwrap()["primary"]
        .as_str()
        .unwrap()
        .to_string();
    let out = s
        .call("add_task", json!({"title": "Wildcard?", "parent": story, "waiting_on": "B", "until": "today", "waiting_ref": "https://teams.example/t/1"}))
        .unwrap();
    let sub = serde_json::from_str::<Value>(&out).unwrap()["primary"]
        .as_str()
        .unwrap()
        .to_string();

    let due: Value =
        serde_json::from_str(&s.call("waiting", json!({"due": true})).unwrap()).unwrap();
    assert_eq!(due[0]["person"], "B");
    assert_eq!(
        due[0]["items"][0]["waiting_ref"],
        "https://teams.example/t/1"
    );
    assert_eq!(due[0]["items"][0]["due_now"], true);

    let first = s.call("chase", json!({"id": sub, "dedupe": "2026-10-02", "source": "agent", "note": "drafted a follow-up"})).unwrap();
    assert!(first.contains("\"deduped\": false"), "{first}");
    let second = s
        .call(
            "chase",
            json!({"id": sub, "dedupe": "2026-10-02", "source": "agent"}),
        )
        .unwrap();
    assert!(second.contains("\"deduped\": true"), "{second}");

    s.call("record_reply", json!({"id": sub, "outcome": "redirected", "to": "C", "summary": "C owns DNS", "source": "agent"})).unwrap();
    let all: Value = serde_json::from_str(&s.call("waiting", json!({})).unwrap()).unwrap();
    assert_eq!(all[0]["person"], "C");
    assert_eq!(all[0]["items"][0]["chases"], 0, "handoff resets the count");

    let task = s.call("get_task", json!({"id": sub})).unwrap();
    assert!(task.contains("[agent] chased B"), "{task}");
    assert!(
        task.contains("[agent] B redirected to C: C owns DNS"),
        "{task}"
    );

    s.call(
        "record_reply",
        json!({"id": sub, "outcome": "answered", "summary": "wildcard is fine"}),
    )
    .unwrap();
    let next: Value = serde_json::from_str(&s.call("next", json!({})).unwrap()).unwrap();
    assert!(
        next["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == sub),
        "{next}"
    );
    std::thread::sleep(Duration::from_millis(50));
}
