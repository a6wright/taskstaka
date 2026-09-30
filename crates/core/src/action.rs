//! The needs-action computation. Everything the panel shows is derived here
//! from the stored tree and an injected clock; nothing in this module is
//! persisted.
//!
//! ```text
//! closed(t)     := status ∈ {done, dropped}
//! blocking(t)   := status ∈ {todo, doing, waiting}      # someday children do NOT block
//! blocked(t)    := ∃ child c: blocking(c)
//! deferred(t)   := status = someday ∨ ∃ ancestor with status ∈ {someday, waiting}
//! followup(t)   := status = waiting ∧ until ≤ today
//! actionable(t) := ¬closed ∧ ¬deferred ∧ ¬blocked ∧ (status ∈ {todo, doing} ∨ followup)
//! resurfaced(t) := actionable ∧ has children ∧ all children closed
//!                  ∧ max(child.completed) ≥ t.updated     # clears when the user touches t
//! ```

use jiff::civil::Date;
use jiff::{Timestamp, ToSpan, Zoned};
use serde::{Deserialize, Serialize};

use crate::model::{Context, Priority, Status, Task, TaskId};
use crate::tree::Tree;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RankConfig {
    /// Tasks due within this many days join the "due soon" tier.
    pub due_soon_days: i64,
}

impl Default for RankConfig {
    fn default() -> Self {
        RankConfig { due_soon_days: 3 }
    }
}

/// Facts about one task derived from the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derived {
    /// Number of direct children in a blocking status.
    pub blocked_by: usize,
    /// The nearest ancestor (or self) whose status defers this subtree.
    pub deferred_by: Option<TaskId>,
    pub followup: bool,
    pub actionable: bool,
    /// When the last child closed, if that makes this task resurfaced.
    pub resurfaced_at: Option<Timestamp>,
}

pub fn derive(tree: &Tree, task: &Task, today: Date) -> Derived {
    let blocked_by = tree
        .children(&task.id)
        .filter(|c| c.status.is_blocking())
        .count();

    let deferred_by = if task.status == Status::Someday {
        Some(task.id.clone())
    } else {
        tree.ancestors(&task.id)
            .into_iter()
            .find(|a| matches!(a.status, Status::Someday | Status::Waiting))
            .map(|a| a.id.clone())
    };

    let followup = task.status == Status::Waiting && task.until.is_some_and(|u| u <= today);

    let actionable = task.status.is_open()
        && deferred_by.is_none()
        && blocked_by == 0
        && (matches!(task.status, Status::Todo | Status::Doing) || followup);

    let resurfaced_at = if actionable && tree.has_children(&task.id) {
        let kids: Vec<&Task> = tree.children(&task.id).collect();
        if kids.iter().all(|c| c.status.is_closed()) {
            kids.iter()
                .filter_map(|c| c.completed)
                .max()
                .filter(|&t| t >= task.updated)
        } else {
            None
        }
    } else {
        None
    };

    Derived {
        blocked_by,
        deferred_by,
        followup,
        actionable,
        resurfaced_at,
    }
}

/// Ranking tiers, best first. Lexicographic so the panel can explain itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Overdue,
    Resurfaced,
    Doing,
    FollowUp,
    DueSoon,
    High,
    Normal,
    Low,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Overdue => "overdue",
            Tier::Resurfaced => "resurfaced",
            Tier::Doing => "doing",
            Tier::FollowUp => "follow-up",
            Tier::DueSoon => "due soon",
            Tier::High => "high",
            Tier::Normal => "normal",
            Tier::Low => "low",
        }
    }
}

/// One row of the panel: an actionable task plus everything needed to
/// display and explain it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NextItem {
    pub id: TaskId,
    pub title: String,
    pub status: Status,
    pub context: Context,
    pub tier: Tier,
    /// Human explanation of the tier, e.g. "resurfaced: all subtasks done 2h ago".
    pub reason: String,
    /// Root first, immediate parent last.
    pub ancestry: Vec<Crumb>,
    pub created: Timestamp,
    pub updated: Timestamp,
    pub started: Option<Timestamp>,
    pub due: Option<Date>,
    pub queued_secs: i64,
    pub untouched_secs: i64,
    #[serde(skip)]
    sort_key: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Crumb {
    pub id: TaskId,
    pub title: String,
}

/// Counts for the panel header. `blocked` and `waiting` are the two flavours
/// of "not actionable but open"; when `actionable == 0` and `open > 0` the
/// right message is "go unblock something".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub open: usize,
    pub actionable: usize,
    pub blocked: usize,
    pub waiting: usize,
    pub doing: usize,
    pub someday: usize,
}

fn in_context(tree: &Tree, task: &Task, ctx: Option<Context>) -> bool {
    match ctx {
        None => true,
        Some(c) => {
            tree.root_of(&task.id)
                .map(|r| r.context)
                .unwrap_or(task.context)
                == c
        }
    }
}

pub fn summary(tree: &Tree, now: &Zoned, ctx: Option<Context>) -> Summary {
    let today = now.date();
    let mut s = Summary::default();
    for t in tree
        .iter()
        .filter(|t| t.status.is_open() && in_context(tree, t, ctx))
    {
        s.open += 1;
        let d = derive(tree, t, today);
        match t.status {
            Status::Doing => s.doing += 1,
            Status::Someday => s.someday += 1,
            Status::Waiting if !d.followup => s.waiting += 1,
            _ => {}
        }
        if d.actionable {
            s.actionable += 1;
        } else if d.blocked_by > 0 {
            s.blocked += 1;
        }
    }
    s
}

pub fn needs_action(
    tree: &Tree,
    now: &Zoned,
    ctx: Option<Context>,
    cfg: &RankConfig,
) -> Vec<NextItem> {
    let today = now.date();
    let now_ts = now.timestamp();
    let soon = today.checked_add(cfg.due_soon_days.days()).unwrap_or(today);

    let mut items: Vec<NextItem> = tree
        .iter()
        .filter(|t| in_context(tree, t, ctx))
        .filter_map(|t| {
            let d = derive(tree, t, today);
            if !d.actionable {
                return None;
            }
            let (tier, reason, sort_key) = classify(t, &d, today, soon, now_ts);
            let mut ancestry: Vec<Crumb> = tree
                .ancestors(&t.id)
                .into_iter()
                .map(|a| Crumb {
                    id: a.id.clone(),
                    title: a.title.clone(),
                })
                .collect();
            ancestry.reverse();
            Some(NextItem {
                id: t.id.clone(),
                title: t.title.clone(),
                status: t.status,
                context: t.context,
                tier,
                reason,
                ancestry,
                created: t.created,
                updated: t.updated,
                started: t.started,
                due: t.due,
                queued_secs: now_ts.as_second() - t.created.as_second(),
                untouched_secs: now_ts.as_second() - t.updated.as_second(),
                sort_key,
            })
        })
        .collect();

    items.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(a.sort_key.cmp(&b.sort_key))
            .then(a.updated.cmp(&b.updated))
            .then(a.id.cmp(&b.id))
    });
    items
}

fn classify(t: &Task, d: &Derived, today: Date, soon: Date, now: Timestamp) -> (Tier, String, i64) {
    let ago = |ts: Timestamp| crate::humanize::since(ts, now);
    let days_since_epoch = |d: Date| {
        d.since(Date::constant(1970, 1, 1))
            .map(|s| s.get_days() as i64)
            .unwrap_or(0)
    };

    if let Some(due) = t.due
        && due < today
    {
        let late = today.since(due).map(|s| s.get_days()).unwrap_or(0);
        return (
            Tier::Overdue,
            format!("overdue by {late}d (due {due})"),
            days_since_epoch(due),
        );
    }
    if let Some(at) = d.resurfaced_at {
        return (
            Tier::Resurfaced,
            format!("resurfaced: all subtasks done {} ago", ago(at)),
            at.as_second(),
        );
    }
    if t.status == Status::Doing {
        let started = t.started.unwrap_or(t.created);
        return (
            Tier::Doing,
            format!("doing for {}", ago(started)),
            started.as_second(),
        );
    }
    if d.followup {
        let who = t.waiting_on.as_deref().unwrap_or("someone");
        let until = t.until.map(|u| u.to_string()).unwrap_or_default();
        return (
            Tier::FollowUp,
            format!("follow up with {who} (since {until})"),
            t.until.map(days_since_epoch).unwrap_or(0),
        );
    }
    if let Some(due) = t.due
        && due <= soon
    {
        let in_days = due.since(today).map(|s| s.get_days()).unwrap_or(0);
        let when = match in_days {
            0 => "today".to_string(),
            1 => "tomorrow".to_string(),
            n => format!("in {n}d"),
        };
        return (Tier::DueSoon, format!("due {when}"), days_since_epoch(due));
    }
    match t.priority {
        Some(Priority::High) => (
            Tier::High,
            format!("high priority, queued {}", ago(t.created)),
            t.created.as_second(),
        ),
        None => (
            Tier::Normal,
            format!("queued {}", ago(t.created)),
            t.created.as_second(),
        ),
        Some(Priority::Low) => (
            Tier::Low,
            format!("low priority, queued {}", ago(t.created)),
            t.created.as_second(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    /// Tiny fixture DSL: (id, parent, status, created_offset_hours).
    struct Fx {
        tree: Tree,
        now: Zoned,
    }

    impl Fx {
        fn new() -> Self {
            let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
            Fx {
                tree: Tree::new(),
                now,
            }
        }
        fn ts(&self, hours_ago: i64) -> Timestamp {
            self.now.timestamp() - (hours_ago * 3600).seconds()
        }
        /// Tree has no get_mut by design; mutate a clone and reinsert.
        fn set(&mut self, id: &str, f: impl FnOnce(&mut Task)) {
            let mut t = self.tree.get(&id.parse().unwrap()).unwrap().clone();
            f(&mut t);
            self.tree.insert(t);
        }
        fn next(&self) -> Vec<(String, Tier)> {
            needs_action(&self.tree, &self.now, None, &RankConfig::default())
                .into_iter()
                .map(|i| (i.id.to_string(), i.tier))
                .collect()
        }
        fn ids(&self) -> Vec<String> {
            self.next().into_iter().map(|(id, _)| id).collect()
        }
    }

    fn fx_add(fx: &mut Fx, id: &str, parent: Option<&str>, status: Status, hours_ago: i64) {
        let mut t = Task::new(id.parse().unwrap(), id, Context::Work, fx.ts(hours_ago));
        t.parent = parent.map(|p| p.parse().unwrap());
        t.status = status;
        if status == Status::Doing {
            t.started = Some(fx.ts(hours_ago));
        }
        if status.is_closed() {
            t.completed = Some(fx.ts(hours_ago / 2));
        }
        fx.tree.insert(t);
    }

    #[test]
    fn parent_with_open_children_is_blocked_not_actionable() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx_add(&mut fx, "chd100", Some("stry00"), Status::Todo, 24);
        fx_add(&mut fx, "chd200", Some("stry00"), Status::Todo, 12);
        assert_eq!(
            fx.ids(),
            ["chd100", "chd200"],
            "FIFO within normal tier, story hidden"
        );
        let d = derive(
            &fx.tree,
            fx.tree.get(&"stry00".parse().unwrap()).unwrap(),
            fx.now.date(),
        );
        assert_eq!(d.blocked_by, 2);
        assert!(!d.actionable);
    }

    #[test]
    fn closing_last_child_resurfaces_parent_at_top() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx_add(&mut fx, "chd100", Some("stry00"), Status::Done, 24);
        fx_add(&mut fx, "0thr00", None, Status::Doing, 100);
        fx_add(&mut fx, "vrgnt0", None, Status::Todo, 1);
        fx.set("vrgnt0", |t| t.priority = Some(Priority::High));
        let next = fx.next();
        assert_eq!(next[0], ("stry00".into(), Tier::Resurfaced));
        assert_eq!(next[1], ("0thr00".into(), Tier::Doing));
        assert_eq!(next[2], ("vrgnt0".into(), Tier::High));
    }

    #[test]
    fn touching_parent_clears_resurfaced() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx_add(&mut fx, "chd100", Some("stry00"), Status::Done, 24);
        let now = fx.now.timestamp();
        fx.set("stry00", |t| t.updated = now);
        assert_eq!(fx.next()[0].1, Tier::Normal);
    }

    #[test]
    fn someday_child_does_not_block_but_someday_subtree_is_deferred() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx_add(&mut fx, "1atr00", Some("stry00"), Status::Someday, 24);
        fx_add(&mut fx, "k1d000", Some("1atr00"), Status::Todo, 12);
        assert_eq!(
            fx.ids(),
            ["stry00"],
            "story actionable; someday subtree hidden"
        );
    }

    #[test]
    fn waiting_ancestor_defers_children_and_followup_surfaces() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Waiting, 48);
        fx_add(&mut fx, "chd100", Some("stry00"), Status::Todo, 24);
        fx.set("stry00", |t| t.waiting_on = Some("Sam".into()));
        assert!(
            fx.ids().is_empty(),
            "waiting on external with no until: nothing actionable"
        );

        fx.set("stry00", |t| t.until = Some(date(2026, 9, 29)));
        // story has an open child so it is still blocked, even on follow-up day
        assert!(fx.ids().is_empty());

        fx.set("chd100", |t| t.status = Status::Done);
        let next = fx.next();
        assert_eq!(next, [("stry00".into(), Tier::FollowUp)]);
    }

    #[test]
    fn tier_order_overdue_first_then_due_soon_then_priorities() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "1ate00", None, Status::Todo, 5);
        fx.set("1ate00", |t| t.due = Some(date(2026, 9, 20)));
        fx_add(&mut fx, "s00n00", None, Status::Todo, 5);
        fx.set("s00n00", |t| t.due = Some(date(2026, 10, 1)));
        fx_add(&mut fx, "far000", None, Status::Todo, 5);
        fx.set("far000", |t| t.due = Some(date(2026, 12, 1)));
        fx_add(&mut fx, "h1gh00", None, Status::Todo, 5);
        fx.set("h1gh00", |t| t.priority = Some(Priority::High));
        fx_add(&mut fx, "10w000", None, Status::Todo, 500);
        fx.set("10w000", |t| t.priority = Some(Priority::Low));
        fx_add(&mut fx, "01d000", None, Status::Todo, 400);
        assert_eq!(
            fx.ids(),
            ["1ate00", "s00n00", "h1gh00", "01d000", "far000", "10w000"]
        );
    }

    #[test]
    fn context_filters_by_root() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx.set("stry00", |t| t.context = Context::Personal);
        fx_add(&mut fx, "chd100", Some("stry00"), Status::Todo, 24);
        fx_add(&mut fx, "w0rk1t", None, Status::Todo, 10);
        let personal = needs_action(
            &fx.tree,
            &fx.now,
            Some(Context::Personal),
            &RankConfig::default(),
        );
        assert_eq!(
            personal.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["chd100"]
        );
        assert_eq!(personal[0].ancestry.len(), 1);
        assert_eq!(personal[0].ancestry[0].id.as_str(), "stry00");
        let s = summary(&fx.tree, &fx.now, Some(Context::Work));
        assert_eq!(
            s,
            Summary {
                open: 1,
                actionable: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn summary_reports_all_blocked_state() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx_add(&mut fx, "chd100", Some("stry00"), Status::Waiting, 24);
        fx.set("chd100", |t| t.waiting_on = Some("vendor".into()));
        let s = summary(&fx.tree, &fx.now, None);
        assert_eq!(s.open, 2);
        assert_eq!(s.actionable, 0);
        assert_eq!(s.blocked, 1);
        assert_eq!(s.waiting, 1);
    }
}
