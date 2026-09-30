//! Speak raw MCP (newline-delimited JSON-RPC over stdio) to `tododo mcp`.

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
        let bin = assert_cmd::cargo::cargo_bin("tododo");
        let ok = Command::new(&bin)
            .args(["init"])
            .env("TODODO_DIR", dir.path())
            .status()
            .unwrap();
        assert!(ok.success());

        let mut child = Command::new(&bin)
            .arg("mcp")
            .env("TODODO_DIR", dir.path())
            .env("TODODO_PORT", "0")
            .env_remove("TODODO_DIRECT")
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
        assert_eq!(init["serverInfo"]["name"], "tododo");
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
        let _ = Command::new(assert_cmd::cargo::cargo_bin("tododo"))
            .args(["stop"])
            .env("TODODO_DIR", self.dir.path())
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

    let text = s.request("resources/read", json!({"uri": "tododo://next"}));
    let text = text["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("[resurfaced] Ship auth story"), "{text}");

    let index = s.request("resources/read", json!({"uri": "tododo://index"}));
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
