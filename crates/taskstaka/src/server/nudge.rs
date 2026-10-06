//! Nudge scheduling and delivery. Rules live in core; this module decides
//! when to evaluate them and where the result goes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use jiff::Zoned;
use taskstaka_core::nudge::{Nudge, NudgeConfig, NudgeLog, evaluate, resurfaced};
use taskstaka_core::settings::Settings;
use taskstaka_core::{Event, Outcome, TaskId};
use tracing::{debug, info, warn};

use super::AppState;

/// Best-effort desktop delivery. The panel is the reliable surface.
pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// Logs only. Used with `--no-notify` and in tests.
pub struct LogNotifier;

impl Notifier for LogNotifier {
    fn notify(&self, title: &str, body: &str) {
        info!(target: "nudge", "{title}: {body}");
    }
}

/// Shells out to the platform's notification command. No extra dependencies,
/// and it works from an unbundled binary on macOS where the native API does
/// not.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, title: &str, body: &str) {
        let title = title.to_string();
        let body = body.to_string();
        info!(target: "nudge", "{title}: {body}");
        std::thread::spawn(move || {
            let result = if cfg!(target_os = "macos") {
                let script = format!(
                    "display notification {} with title {}",
                    applescript_quote(&body),
                    applescript_quote(&format!("taskstaka · {title}"))
                );
                std::process::Command::new("osascript")
                    .arg("-e")
                    .arg(script)
                    .status()
            } else {
                std::process::Command::new("notify-send")
                    .arg("--app-name=taskstaka")
                    .arg(format!("taskstaka · {title}"))
                    .arg(&body)
                    .status()
            };
            if let Err(e) = result {
                debug!("desktop notification failed: {e}");
            }
        });
    }
}

fn applescript_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub struct Nudger {
    pub cfg: NudgeConfig,
    pub notifier: Box<dyn Notifier>,
    log_path: PathBuf,
    dir: PathBuf,
}

impl Nudger {
    pub fn new(state: &AppState, cfg: NudgeConfig, notifier: Box<dyn Notifier>) -> Nudger {
        Nudger {
            cfg,
            notifier,
            log_path: state.dir.join(".taskstaka").join("nudges.json"),
            dir: state.dir.clone(),
        }
    }

    /// Recompute the current nudge set, publish it for `/nudges` and the
    /// panel, and deliver whichever ones are past their cooldown.
    pub fn evaluate(&self, state: &AppState) {
        let now = Zoned::now();
        let current: Vec<Nudge> = {
            let Ok(store) = state.store.lock() else {
                return;
            };
            let mut log = NudgeLog::load(&self.log_path);
            log.prune(store.tree());
            let current = evaluate(store.tree(), &now, &self.cfg);
            let due = log.take_due(&current, now.timestamp());
            if let Err(e) = log.save(&self.log_path) {
                warn!("could not save nudge log: {e}");
            }
            self.deliver(&due, &now);
            current
        };
        let changed = state
            .nudges
            .lock()
            .map(|mut slot| {
                let changed = *slot != current;
                *slot = current;
                changed
            })
            .unwrap_or(false);
        // A new nudge set is a visible change: wake the long-polls so the
        // panel/banner update without waiting for the next mutation.
        if changed {
            state.bump();
        }
    }

    /// Event-driven nudges from a mutation, fired immediately.
    pub fn on_outcome(&self, state: &AppState, out: &Outcome) {
        let ids: Vec<TaskId> = out
            .events
            .iter()
            .filter_map(|e| match e {
                Event::Resurfaced { id } => Some(id.clone()),
                _ => None,
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let now = Zoned::now();
        let Ok(store) = state.store.lock() else {
            return;
        };
        let mut log = NudgeLog::load(&self.log_path);
        // A parent that is already in hand (auto-advance started it) needs no
        // "it is ready" popup: you are looking at it.
        let nudges: Vec<Nudge> = ids
            .iter()
            .filter(|id| {
                store
                    .tree()
                    .get(id)
                    .is_some_and(|t| t.status != taskstaka_core::Status::Doing)
            })
            .filter_map(|id| resurfaced(store.tree(), id))
            .collect();
        // Fresh key each time it resurfaces: encode the moment into the key
        // so "once per resurfacing" holds while the log stays bounded by prune.
        let due: Vec<Nudge> = nudges
            .into_iter()
            .map(|mut n| {
                n.key = format!("{}@{}", n.key, now.timestamp().as_second());
                n
            })
            .filter(|n| log.is_due(n, now.timestamp()))
            .collect();
        for n in &due {
            log.record(n, now.timestamp());
        }
        let _ = log.save(&self.log_path);
        self.deliver(&due, &now);
    }

    fn deliver(&self, due: &[Nudge], now: &Zoned) {
        if due.is_empty() {
            return;
        }
        // Read per delivery so the setting applies without a restart.
        if !Settings::load(&self.dir).notifications {
            debug!("{} nudge(s) suppressed: notifications are off", due.len());
            return;
        }
        if self.cfg.in_quiet_hours(now) {
            debug!("{} nudge(s) suppressed by quiet hours", due.len());
            return;
        }
        for n in due {
            self.notifier.notify(&n.title, &n.message);
        }
    }
}

/// Evaluate every `interval` and after every mutation (debounced by the
/// interval's tick granularity via the revision watch).
pub fn spawn_scheduler(state: Arc<AppState>, nudger: Arc<Nudger>, interval: Duration) {
    tokio::spawn(async move {
        let mut rx = state.rev.subscribe();
        let mut ticker = tokio::time::interval(interval);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                changed = rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // Let a burst of mutations settle before recomputing.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    rx.mark_unchanged();
                }
            }
            let st = state.clone();
            let n = nudger.clone();
            let _ = tokio::task::spawn_blocking(move || n.evaluate(&st)).await;
        }
    });
}
