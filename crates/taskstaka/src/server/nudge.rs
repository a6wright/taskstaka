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
}

impl Nudger {
    pub fn new(state: &AppState, cfg: NudgeConfig, notifier: Box<dyn Notifier>) -> Nudger {
        Nudger {
            cfg,
            notifier,
            log_path: state.dir.join(".taskstaka").join("nudges.json"),
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
            log.prune(store.tree(), now.timestamp());
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
        if !Settings::load().notifications {
            debug!("{} nudge(s) suppressed: notifications are off", due.len());
            return;
        }
        if self.cfg.in_quiet_hours(now) {
            debug!("{} nudge(s) suppressed by quiet hours", due.len());
            return;
        }
        if let Some((title, body)) = coalesce(due) {
            self.notifier.notify(&title, &body);
        }
    }
}

/// One notification per evaluation. Several nudges coming due together
/// (four "still doing" and a WIP warning at the same tick, once a day) used
/// to be five popups in a row; now they are one that lists them, and the
/// Nudges page has the rest.
pub fn coalesce(due: &[Nudge]) -> Option<(String, String)> {
    const SHOWN: usize = 4;
    const LINE: usize = 110;
    match due {
        [] => None,
        [n] => Some((n.title.clone(), n.message.clone())),
        many => {
            let clip = |s: &str| {
                if s.chars().count() > LINE {
                    format!("{}…", s.chars().take(LINE - 1).collect::<String>())
                } else {
                    s.to_string()
                }
            };
            let mut lines: Vec<String> = many
                .iter()
                .take(SHOWN)
                .map(|n| clip(&format!("{}: {}", n.title, n.message)))
                .collect();
            if many.len() > SHOWN {
                lines.push(format!(
                    "and {} more on the Nudges page",
                    many.len() - SHOWN
                ));
            }
            Some((format!("{} nudges", many.len()), lines.join("\n")))
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

#[cfg(test)]
mod tests {
    use super::*;
    use taskstaka_core::nudge::Rule;

    fn nudge(rule: Rule, title: &str, message: &str) -> Nudge {
        Nudge {
            rule,
            key: title.to_lowercase(),
            task: None,
            title: title.into(),
            message: message.into(),
        }
    }

    #[test]
    fn a_burst_becomes_one_notification() {
        assert_eq!(coalesce(&[]), None);
        let one = [nudge(
            Rule::WipLimit,
            "Too much in flight",
            "4 things are 'doing'. Pick one.",
        )];
        assert_eq!(
            coalesce(&one),
            Some((
                "Too much in flight".into(),
                "4 things are 'doing'. Pick one.".into()
            ))
        );
        let long = "x".repeat(200);
        let burst = [
            nudge(Rule::StuckDoing, "Still doing", "On 'a' for 3d."),
            nudge(Rule::StuckDoing, "Still doing", "On 'b' for 5d."),
            nudge(
                Rule::WipLimit,
                "Too much in flight",
                "4 things are 'doing'.",
            ),
            nudge(Rule::StaleInQueue, "Rotting in the queue", &long),
            nudge(Rule::FollowUpDue, "Follow up", "Follow up with Bo."),
        ];
        let (title, body) = coalesce(&burst).unwrap();
        assert_eq!(title, "5 nudges");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "Still doing: On 'a' for 3d.");
        assert!(lines[3].ends_with('…') && lines[3].chars().count() == 110);
        assert_eq!(lines[4], "and 1 more on the Nudges page");
    }
}
