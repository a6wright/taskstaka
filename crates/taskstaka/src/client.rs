//! HTTP client for the daemon, plus discovery and auto-start. Blocking (ureq)
//! on purpose: the CLI wants fast startup and no runtime.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use taskstaka_core::view::NextResponse;
use taskstaka_core::{Command, Context, Outcome, TaskId, TaskView};
use ureq::Agent;

use crate::DEFAULT_PORT;

/// Written by the daemon to `<dir>/.taskstaka/daemon.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    pub version: String,
    pub started: String,
}

impl DaemonInfo {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(".taskstaka").join("daemon.json")
    }

    pub fn read(dir: &Path) -> Option<DaemonInfo> {
        let text = std::fs::read_to_string(Self::path(dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("serializable");
        std::fs::write(Self::path(dir), text)
    }

    pub fn remove(dir: &Path) {
        let _ = std::fs::remove_file(Self::path(dir));
    }
}

/// Error body every endpoint returns on failure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiErrorInner,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiErrorInner {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Clone)]
pub struct Client {
    agent: Agent,
    base: String,
}

#[derive(Debug, Deserialize)]
pub struct Health {
    pub version: String,
    pub rev: u64,
    pub todo_dir: PathBuf,
    pub pid: u32,
    #[serde(default)]
    pub started: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChangesResponse {
    rev: u64,
}

#[derive(Debug, Deserialize)]
struct ResolveResponse {
    id: TaskId,
}

impl Client {
    /// Talk to a daemon at a known address; no discovery.
    pub fn at(addr: &str) -> Client {
        let agent: Agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .into();
        Client {
            agent,
            base: format!("http://{addr}"),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Find the daemon for `dir`, or return None if none answers.
    pub fn discover(dir: &Path) -> Option<Client> {
        let info = DaemonInfo::read(dir)?;
        let c = Client::at(&format!("127.0.0.1:{}", info.port));
        c.health().ok().filter(|h| h.todo_dir == dir).map(|_| c)
    }

    /// Discover or auto-start the daemon for `dir`.
    pub fn connect(dir: &Path, port_hint: Option<u16>) -> Result<Client> {
        if let Some(c) = Self::discover(dir) {
            return Ok(c);
        }
        spawn_daemon(dir, port_hint.unwrap_or(DEFAULT_PORT))?;
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            if let Some(c) = Self::discover(dir) {
                return Ok(c);
            }
        }
        bail!(
            "started a daemon for {} but it did not answer within 4s (try `taskstaka serve` in the foreground to see why)",
            dir.display()
        )
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn read<T: serde::de::DeserializeOwned>(
        mut res: ureq::http::Response<ureq::Body>,
    ) -> Result<T> {
        let status = res.status().as_u16();
        if (200..300).contains(&status) {
            return res
                .body_mut()
                .read_json::<T>()
                .context("decoding daemon response");
        }
        let body: Option<ApiErrorBody> = res.body_mut().read_json().ok();
        match body {
            Some(b) => Err(ApiError {
                status,
                code: b.error.code,
                message: b.error.message,
                details: b.error.details,
            }
            .into()),
            None => Err(anyhow!("daemon returned HTTP {status}")),
        }
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        Self::read(self.agent.get(self.url(path)).call()?)
    }

    fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        Self::read(self.agent.post(self.url(path)).send_json(body)?)
    }

    fn post_empty<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        Self::read(self.agent.post(self.url(path)).send_empty()?)
    }

    pub fn health(&self) -> Result<Health> {
        let agent: Agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_millis(400)))
            .build()
            .into();
        Self::read(agent.get(self.url("/health")).call()?)
    }

    pub fn tasks(&self, include_closed: bool) -> Result<Vec<TaskView>> {
        self.get(&format!("/api/v1/tasks?include_closed={include_closed}"))
    }

    pub fn task(&self, id: &str) -> Result<TaskView> {
        self.get(&format!("/api/v1/tasks/{id}"))
    }

    pub fn resolve(&self, prefix: &str) -> Result<TaskId> {
        let r: ResolveResponse = self.get(&format!("/api/v1/resolve/{prefix}"))?;
        Ok(r.id)
    }

    pub fn next(&self, context: Option<Context>, limit: Option<usize>) -> Result<NextResponse> {
        let mut q = Vec::new();
        if let Some(c) = context {
            q.push(format!("context={c}"));
        }
        if let Some(n) = limit {
            q.push(format!("limit={n}"));
        }
        let qs = if q.is_empty() {
            String::new()
        } else {
            format!("?{}", q.join("&"))
        };
        self.get(&format!("/api/v1/next{qs}"))
    }

    pub fn apply(&self, cmd: &Command) -> Result<Outcome> {
        self.post("/api/v1/commands", cmd)
    }

    pub fn reload(&self) -> Result<()> {
        let _: serde_json::Value = self.post_empty("/api/v1/reload")?;
        Ok(())
    }

    pub fn reindex(&self) -> Result<()> {
        let _: serde_json::Value = self.post_empty("/api/v1/reindex")?;
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        let _: serde_json::Value = self.post_empty("/api/v1/shutdown")?;
        Ok(())
    }

    pub fn nudges(&self) -> Result<Vec<taskstaka_core::nudge::Nudge>> {
        self.get("/api/v1/nudges")
    }

    pub fn waiting(
        &self,
        context: Option<Context>,
        due: bool,
    ) -> Result<Vec<taskstaka_core::action::PersonGroup>> {
        let mut q = vec![format!("due={due}")];
        if let Some(c) = context {
            q.push(format!("context={c}"));
        }
        self.get(&format!("/api/v1/waiting?{}", q.join("&")))
    }

    /// The daemon started before this binary was built: it is serving old
    /// code. `taskstaka restart` fixes it.
    pub fn is_stale(&self) -> bool {
        let Ok(h) = self.health() else { return false };
        let Some(started) = h
            .started
            .as_deref()
            .and_then(|s| s.parse::<jiff::Timestamp>().ok())
        else {
            return false;
        };
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let Ok(md) = std::fs::metadata(exe) else {
            return false;
        };
        let Ok(mtime) = md.modified() else {
            return false;
        };
        let Ok(mtime) = jiff::Timestamp::try_from(mtime) else {
            return false;
        };
        mtime > started
    }

    /// Stop the daemon (if any) and wait until it is gone.
    pub fn stop_and_wait(dir: &Path) -> Result<bool> {
        let Some(c) = Self::discover(dir) else {
            DaemonInfo::remove(dir);
            return Ok(false);
        };
        let pid = c.health().ok().map(|h| h.pid);
        c.shutdown()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            // Gone means: port closed, daemon.json removed, and (where we can
            // tell) the process itself exited — so the lock is free.
            let port_closed = c.health().is_err();
            let file_gone = !DaemonInfo::path(dir).exists();
            let process_gone = pid.is_none_or(|p| !process_alive(p));
            if port_closed && file_gone && process_gone {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        bail!("the daemon at {} did not stop within 5s", c.base())
    }

    pub fn tasks_filtered(
        &self,
        status: Option<taskstaka_core::Status>,
        context: Option<Context>,
        include_closed: bool,
    ) -> Result<Vec<TaskView>> {
        let mut q = vec![format!("include_closed={include_closed}")];
        if let Some(s) = status {
            q.push(format!("status={s}"));
        }
        if let Some(c) = context {
            q.push(format!("context={c}"));
        }
        self.get(&format!("/api/v1/tasks?{}", q.join("&")))
    }

    pub fn tree(
        &self,
        context: Option<Context>,
        include_closed: bool,
    ) -> Result<Vec<taskstaka_core::view::TreeNode>> {
        let mut q = vec![format!("include_closed={include_closed}")];
        if let Some(c) = context {
            q.push(format!("context={c}"));
        }
        self.get(&format!("/api/v1/tree?{}", q.join("&")))
    }

    /// Block until the daemon's revision exceeds `after` (or ~25s pass).
    pub fn changes(&self, after: u64) -> Result<u64> {
        let res = self
            .agent
            .get(self.url(&format!("/api/v1/changes?after={after}")))
            .config()
            .timeout_global(Some(Duration::from_secs(40)))
            .build()
            .call()?;
        let r: ChangesResponse = Self::read(res)?;
        Ok(r.rev)
    }
}

/// Best-effort liveness check by pid. On non-Linux platforms we cannot tell
/// cheaply and assume the process is gone once its port has closed.
fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    } else {
        false
    }
}

/// Start `taskstaka serve` as a detached child and return immediately.
pub fn spawn_daemon(dir: &Path, port: u16) -> Result<()> {
    let exe = std::env::current_exe().context("locating the taskstaka binary")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("serve")
        .arg("--dir")
        .arg(dir)
        .arg("--port")
        .arg(port.to_string())
        .env_remove("TASKSTAKA_DIRECT")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().context("spawning the daemon")?;
    Ok(())
}
