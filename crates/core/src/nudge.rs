//! Nudges: rule-generated reminders. Pure evaluation of the tree at a point
//! in time, plus a small persisted cooldown log so restarts do not re-spam.
//!
//! | Rule            | Trigger                                              | Cooldown |
//! |-----------------|------------------------------------------------------|----------|
//! | `StuckDoing`    | doing for more than `stuck_doing_days`               | 24h/task |
//! | `AllBlocked`    | open tasks exist but nothing is actionable           | 4h       |
//! | `StaleInQueue`  | actionable and untouched for `stale_days`            | 24h      |
//! | `Resurfaced`    | event: a parent's last blocking child closed         | once     |
//! | `FollowUpDue`   | waiting task whose `until` has arrived               | 24h/task |
//! | `WipLimit`      | more than `wip_limit` tasks are doing                | 24h      |

use std::collections::HashMap;
use std::path::Path;

use jiff::{Timestamp, Zoned};
use serde::{Deserialize, Serialize};

use crate::action::{derive, summary};
use crate::humanize;
use crate::model::{Status, TaskId};
use crate::tree::Tree;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NudgeConfig {
    pub stuck_doing_days: i64,
    pub stale_days: i64,
    pub wip_limit: usize,
    /// Desktop notifications are suppressed between these local hours
    /// (start, end), e.g. (22, 8). Nudges are still computed and shown in
    /// the panel.
    pub quiet_hours: Option<(u8, u8)>,
    /// Local hour after which the daily "waiting on" digest may fire.
    pub digest_hour: u8,
}

impl Default for NudgeConfig {
    fn default() -> Self {
        NudgeConfig {
            stuck_doing_days: 2,
            stale_days: 14,
            wip_limit: 3,
            quiet_hours: None,
            digest_hour: 9,
        }
    }
}

impl NudgeConfig {
    pub fn in_quiet_hours(&self, now: &Zoned) -> bool {
        let Some((start, end)) = self.quiet_hours else {
            return false;
        };
        let h = now.hour() as u8;
        if start <= end {
            h >= start && h < end
        } else {
            h >= start || h < end
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    StuckDoing,
    AllBlocked,
    StaleInQueue,
    Resurfaced,
    FollowUpDue,
    WipLimit,
    /// Daily list of everyone you are waiting on.
    WaitingDigest,
}

impl Rule {
    pub fn cooldown_secs(self) -> i64 {
        match self {
            Rule::StuckDoing => 24 * 3600,
            Rule::AllBlocked => 4 * 3600,
            Rule::StaleInQueue => 24 * 3600,
            Rule::Resurfaced => 0,
            Rule::FollowUpDue => 24 * 3600,
            Rule::WipLimit => 24 * 3600,
            Rule::WaitingDigest => 20 * 3600,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nudge {
    pub rule: Rule,
    /// Cooldown key: rule plus the task it is about (or the rule alone).
    pub key: String,
    pub task: Option<TaskId>,
    pub title: String,
    pub message: String,
}

impl Nudge {
    fn new(
        rule: Rule,
        task: Option<&TaskId>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Nudge {
        let key = match task {
            Some(id) => format!("{}:{}", rule_slug(rule), id),
            None => rule_slug(rule).to_string(),
        };
        Nudge {
            rule,
            key,
            task: task.cloned(),
            title: title.into(),
            message: message.into(),
        }
    }
}

fn rule_slug(r: Rule) -> &'static str {
    match r {
        Rule::StuckDoing => "stuck_doing",
        Rule::AllBlocked => "all_blocked",
        Rule::StaleInQueue => "stale_in_queue",
        Rule::Resurfaced => "resurfaced",
        Rule::FollowUpDue => "follow_up_due",
        Rule::WipLimit => "wip_limit",
        Rule::WaitingDigest => "waiting_digest",
    }
}

/// Every nudge that applies to the current state, regardless of cooldown.
/// The panel shows these; the notifier filters through `NudgeLog`.
pub fn evaluate(tree: &Tree, now: &Zoned, cfg: &NudgeConfig) -> Vec<Nudge> {
    let ts = now.timestamp();
    let today = now.date();
    let mut out = Vec::new();

    let s = summary(tree, now, None);
    if s.open > 0 && s.actionable == 0 {
        let who: Vec<String> = tree
            .iter()
            .filter(|t| t.status == Status::Waiting)
            .filter_map(|t| t.waiting_on.clone())
            .take(3)
            .collect();
        let detail = if who.is_empty() {
            format!("{} blocked by subtasks.", s.blocked)
        } else {
            format!(
                "{} blocked, {} waiting on {}.",
                s.blocked,
                s.waiting,
                who.join(", ")
            )
        };
        out.push(Nudge::new(
            Rule::AllBlocked,
            None,
            "Nothing is actionable",
            format!("{detail} Go unblock something."),
        ));
    }

    let stuck_after = cfg.stuck_doing_days * 24 * 3600;
    let stale_after = cfg.stale_days * 24 * 3600;
    let secs_between = |from: Timestamp| ts.as_second() - from.as_second();
    let mut stale: Vec<(&TaskId, &str, i64)> = Vec::new();
    let mut doing = 0usize;

    for t in tree.iter().filter(|t| t.status.is_open()) {
        let d = derive(tree, t, today);
        if t.status == Status::Doing {
            doing += 1;
            let since = t.started.unwrap_or(t.created);
            if secs_between(since) > stuck_after {
                let root = tree.root_of(&t.id).filter(|r| r.id != t.id);
                let msg = match root {
                    Some(r) => format!(
                        "On '{}' for {}. '{}' is waiting on it.",
                        t.title,
                        humanize::since(since, ts),
                        r.title
                    ),
                    None => format!(
                        "On '{}' for {}. Finish it or park it.",
                        t.title,
                        humanize::since(since, ts)
                    ),
                };
                out.push(Nudge::new(
                    Rule::StuckDoing,
                    Some(&t.id),
                    "Still doing",
                    msg,
                ));
            }
        }
        if d.followup {
            let since = t.waiting_since.unwrap_or(t.updated);
            let chased = match (t.chases.len(), t.chases.last()) {
                (0, _) => String::new(),
                (n, Some(last)) => {
                    format!(", chased {n}× (last {} ago)", humanize::since(*last, ts))
                }
                (n, None) => format!(", chased {n}×"),
            };
            let mut n = Nudge::new(
                Rule::FollowUpDue,
                Some(&t.id),
                "Follow up",
                format!(
                    "Follow up with {}: '{}' — waiting {}{chased}.",
                    t.waiting_on.as_deref().unwrap_or("someone"),
                    t.title,
                    humanize::since(since, ts)
                ),
            );
            // A new chase restarts the cooldown: the next reminder is about the
            // next follow-up, not the one you just did.
            n.key = format!("{}#{}", n.key, t.chases.len());
            out.push(n);
        }
        if d.actionable && secs_between(t.updated) > stale_after && t.status != Status::Doing {
            stale.push((&t.id, t.title.as_str(), secs_between(t.updated)));
        }
    }

    if !stale.is_empty() {
        stale.sort_by_key(|(_, _, secs)| std::cmp::Reverse(*secs));
        let listed: Vec<String> = stale
            .iter()
            .take(3)
            .map(|(_, title, secs)| format!("'{}' ({})", title, humanize::duration(*secs)))
            .collect();
        let more = if stale.len() > 3 {
            format!(" and {} more", stale.len() - 3)
        } else {
            String::new()
        };
        out.push(Nudge::new(
            Rule::StaleInQueue,
            None,
            "Rotting in the queue",
            format!(
                "{}{} untouched. Do, drop, or someday them.",
                listed.join(", "),
                more
            ),
        ));
    }

    if doing > cfg.wip_limit {
        out.push(Nudge::new(
            Rule::WipLimit,
            None,
            "Too much in flight",
            format!("{doing} things are 'doing'. Pick one."),
        ));
    }

    // R7: once a day, after digest_hour, the whole "waiting on" list. Keyed by
    // date so the cooldown log makes it naturally once-per-day.
    if now.hour() as u8 >= cfg.digest_hour {
        let groups = crate::action::waiting_on(tree, now, None);
        if !groups.is_empty() {
            let listed: Vec<String> = groups
                .iter()
                .take(4)
                .map(|g| {
                    let first = &g.items[0];
                    let extra = if g.items.len() > 1 {
                        format!(" +{}", g.items.len() - 1)
                    } else {
                        String::new()
                    };
                    let chased = if first.wait.chases > 0 {
                        format!(", {}×", first.wait.chases)
                    } else {
                        String::new()
                    };
                    format!(
                        "{} — {} ({}{chased}){extra}",
                        g.person,
                        first.wait.title,
                        humanize::duration(first.since_secs)
                    )
                })
                .collect();
            let more = if groups.len() > 4 {
                format!(" and {} more", groups.len() - 4)
            } else {
                String::new()
            };
            let mut n = Nudge::new(
                Rule::WaitingDigest,
                None,
                "Waiting on",
                format!(
                    "Waiting on {} {}: {}{more}.",
                    groups.len(),
                    if groups.len() == 1 {
                        "person"
                    } else {
                        "people"
                    },
                    listed.join(" · ")
                ),
            );
            n.key = format!("waiting_digest:{}", today);
            out.push(n);
        }
    }

    out
}

/// Build the event-driven nudge for a parent that just resurfaced.
pub fn resurfaced(tree: &Tree, id: &TaskId) -> Option<Nudge> {
    let t = tree.get(id)?;
    Some(Nudge::new(
        Rule::Resurfaced,
        Some(id),
        "Back on your plate",
        format!("'{}' is ready: all its prerequisites are done.", t.title),
    ))
}

/// When each nudge key last fired. Persisted so a restart does not re-fire
/// everything.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NudgeLog {
    pub last_fired: HashMap<String, Timestamp>,
}

impl NudgeLog {
    pub fn load(path: &Path) -> NudgeLog {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("serializable");
        crate::store::atomic_write(path, text.as_bytes()).map_err(std::io::Error::other)
    }

    pub fn is_due(&self, n: &Nudge, now: Timestamp) -> bool {
        match self.last_fired.get(&n.key) {
            None => true,
            Some(last) => {
                let cd = n.rule.cooldown_secs();
                cd > 0 && now.as_second() - last.as_second() >= cd
            }
        }
    }

    pub fn record(&mut self, n: &Nudge, now: Timestamp) {
        self.last_fired.insert(n.key.clone(), now);
    }

    /// Forget keys for tasks that no longer exist, so the file stays small.
    /// Date-keyed digests have no task: keep them while their cooldown could
    /// still matter, or the once-a-day digest would fire on every tick.
    pub fn prune(&mut self, tree: &Tree, now: Timestamp) {
        const DIGEST_KEEP_SECS: i64 = 7 * 24 * 3600;
        self.last_fired.retain(|k, at| match k.split_once(':') {
            Some(("waiting_digest", _)) => now.as_second() - at.as_second() < DIGEST_KEEP_SECS,
            Some((_, id)) => id
                .parse::<TaskId>()
                .ok()
                .is_some_and(|id| tree.contains(&id)),
            None => true,
        });
    }

    /// Split into (due now, suppressed by cooldown) and record the due ones.
    pub fn take_due(&mut self, nudges: &[Nudge], now: Timestamp) -> Vec<Nudge> {
        let due: Vec<Nudge> = nudges
            .iter()
            .filter(|n| self.is_due(n, now))
            .cloned()
            .collect();
        for n in &due {
            self.record(n, now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Context, Task};
    use jiff::ToSpan;

    fn fixture() -> (Tree, Zoned) {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| now.timestamp() - (h * 3600).seconds();
        let mk = |id: &str, title: &str, parent: Option<&str>, status: Status, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t.status = status;
            if status == Status::Doing {
                t.started = Some(at(h));
            }
            t
        };
        let mut tasks = vec![
            mk("stry00", "Ship auth story", None, Status::Todo, 400),
            mk(
                "jwt000",
                "Fix JWT expiry",
                Some("stry00"),
                Status::Doing,
                72,
            ),
            mk("sta1e0", "Old thing", None, Status::Todo, 20 * 24),
            mk("wa1t00", "Renew passport", None, Status::Waiting, 96),
        ];
        tasks[3].waiting_on = Some("Service Canada".into());
        tasks[3].until = Some(now.date());
        (Tree::from_tasks(tasks), now)
    }

    #[test]
    fn rules_fire_with_readable_messages() {
        let (tree, now) = fixture();
        let n = evaluate(&tree, &now, &NudgeConfig::default());
        let rules: Vec<Rule> = n.iter().map(|x| x.rule).collect();
        assert!(rules.contains(&Rule::StuckDoing));
        assert!(rules.contains(&Rule::FollowUpDue));
        assert!(rules.contains(&Rule::StaleInQueue));
        assert!(!rules.contains(&Rule::AllBlocked), "sta1e0 is actionable");
        let stuck = n.iter().find(|x| x.rule == Rule::StuckDoing).unwrap();
        assert_eq!(
            stuck.message,
            "On 'Fix JWT expiry' for 3d. 'Ship auth story' is waiting on it."
        );
        let stale = n.iter().find(|x| x.rule == Rule::StaleInQueue).unwrap();
        assert!(
            stale.message.starts_with("'Old thing' (2w) untouched"),
            "{}",
            stale.message
        );
    }

    #[test]
    fn all_blocked_when_nothing_actionable() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let mut t = Task::new(
            "stry00".parse().unwrap(),
            "Story",
            Context::Work,
            now.timestamp(),
        );
        t.status = Status::Waiting;
        t.waiting_on = Some("Bob".into());
        let tree = Tree::from_tasks([t]);
        let n: Vec<Nudge> = evaluate(&tree, &now, &NudgeConfig::default())
            .into_iter()
            .filter(|x| x.rule != Rule::WaitingDigest)
            .collect();
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].rule, Rule::AllBlocked);
        assert!(n[0].message.contains("waiting on Bob"));
    }

    #[test]
    fn cooldown_log_suppresses_and_persists() {
        let (tree, now) = fixture();
        // The date-keyed digest is exercised in waiting_tests; drop it here.
        let n: Vec<Nudge> = evaluate(&tree, &now, &NudgeConfig::default())
            .into_iter()
            .filter(|x| x.rule != Rule::WaitingDigest)
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nudges.json");
        let mut log = NudgeLog::load(&path);
        let due = log.take_due(&n, now.timestamp());
        assert_eq!(due.len(), n.len());
        assert!(
            log.take_due(&n, now.timestamp()).is_empty(),
            "everything on cooldown"
        );
        log.save(&path).unwrap();

        let mut reloaded = NudgeLog::load(&path);
        let later = now.timestamp() + (23 * 3600).seconds();
        assert!(reloaded.take_due(&n, later).is_empty());
        let later = now.timestamp() + (25 * 3600).seconds();
        assert_eq!(reloaded.take_due(&n, later).len(), n.len());

        // Resurfaced has no cooldown but does not repeat for the same key
        // until re-recorded with a new "now"; and prune drops dead ids.
        let r = resurfaced(&tree, &"stry00".parse().unwrap()).unwrap();
        assert!(reloaded.is_due(&r, later));
        reloaded.record(&r, later);
        assert!(!reloaded.is_due(&r, later));
        reloaded
            .last_fired
            .insert("stuck_doing:zzzzzz".into(), later);
        reloaded.prune(&tree, now.timestamp());
        assert!(!reloaded.last_fired.contains_key("stuck_doing:zzzzzz"));
    }

    #[test]
    fn the_daily_digest_survives_prune_so_its_cooldown_holds() {
        // The daemon prunes then take_due on every evaluation. A date-keyed
        // digest used to be pruned each time and so fired on every tick.
        let (tree, now) = fixture();
        let digest = Nudge {
            rule: Rule::WaitingDigest,
            key: "waiting_digest:2026-09-29".into(),
            task: None,
            title: "Waiting on".into(),
            message: "Waiting on 1 person".into(),
        };
        let mut log = NudgeLog::default();
        let t0 = now.timestamp();
        for tick in 0..5 {
            let at = t0 + (tick * 300).seconds();
            log.prune(&tree, at);
            let due = log.take_due(std::slice::from_ref(&digest), at);
            assert_eq!(due.len(), usize::from(tick == 0), "tick {tick}");
        }
        // Old digest keys are eventually dropped so the file stays small.
        log.prune(&tree, t0 + (8 * 24 * 3600).seconds());
        assert!(!log.last_fired.contains_key("waiting_digest:2026-09-29"));
    }

    #[test]
    fn quiet_hours_wrap_midnight() {
        let cfg = NudgeConfig {
            quiet_hours: Some((22, 8)),
            ..Default::default()
        };
        let late: Zoned = "2026-09-29T23:30:00[UTC]".parse().unwrap();
        let early: Zoned = "2026-09-30T07:59:00[UTC]".parse().unwrap();
        let day: Zoned = "2026-09-30T12:00:00[UTC]".parse().unwrap();
        assert!(cfg.in_quiet_hours(&late));
        assert!(cfg.in_quiet_hours(&early));
        assert!(!cfg.in_quiet_hours(&day));
    }
}

#[cfg(test)]
mod waiting_tests {
    use super::*;
    use crate::model::{Context, Task};
    use jiff::ToSpan;

    #[test]
    fn digest_once_a_day_after_digest_hour_and_follow_up_mentions_chases() {
        let morning: Zoned = "2026-09-29T08:00:00[UTC]".parse().unwrap();
        let later: Zoned = "2026-09-29T10:00:00[UTC]".parse().unwrap();
        let mut t = Task::new(
            "wa1t00".parse().unwrap(),
            "DNS",
            Context::Work,
            morning.timestamp() - 3.days().try_into().unwrap_or(jiff::SignedDuration::ZERO),
        );
        t.status = Status::Waiting;
        t.waiting_on = Some("B".into());
        t.waiting_since = Some(t.created);
        t.chases = vec![t.created];
        t.until = Some(morning.date());
        let tree = Tree::from_tasks([t]);
        let cfg = NudgeConfig::default();

        let early = evaluate(&tree, &morning, &cfg);
        assert!(
            early.iter().all(|n| n.rule != Rule::WaitingDigest),
            "before digest hour"
        );
        let fu = early.iter().find(|n| n.rule == Rule::FollowUpDue).unwrap();
        assert!(
            fu.message.contains("Follow up with B") && fu.message.contains("chased 1×"),
            "{}",
            fu.message
        );
        assert!(
            fu.key.ends_with("#1"),
            "key carries the chase count: {}",
            fu.key
        );

        let day = evaluate(&tree, &later, &cfg);
        let digest = day.iter().find(|n| n.rule == Rule::WaitingDigest).unwrap();
        assert_eq!(digest.key, "waiting_digest:2026-09-29");
        assert!(
            digest.message.starts_with("Waiting on 1 person: B — DNS"),
            "{}",
            digest.message
        );

        let mut log = NudgeLog::default();
        assert_eq!(log.take_due(&day, later.timestamp()).len(), day.len());
        let again: Zoned = "2026-09-29T17:00:00[UTC]".parse().unwrap();
        assert!(
            log.take_due(&evaluate(&tree, &again, &cfg), again.timestamp())
                .is_empty(),
            "same day: quiet"
        );
    }
}
