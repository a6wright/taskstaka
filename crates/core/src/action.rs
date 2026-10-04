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
    /// Open, but some ancestor is closed: the parent was ticked off and this
    /// was reopened (or added) afterwards. Still actionable; shown with a hint.
    pub orphaned: bool,
    /// For a blocked task: the person the wait ultimately comes down to — the
    /// oldest waiting descendant. Lets the parent say "waiting on B · 4d".
    pub waiting_on_summary: Option<WaitingRef>,
}

/// One person you are waiting on, for one task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingRef {
    pub person: String,
    pub task_id: TaskId,
    pub title: String,
    pub since: Timestamp,
    pub chases: usize,
    pub last_chased: Option<Timestamp>,
    pub until: Option<Date>,
    pub waiting_ref: Option<String>,
}

impl WaitingRef {
    fn of(t: &Task) -> WaitingRef {
        WaitingRef {
            person: t.waiting_on.clone().unwrap_or_else(|| "someone".into()),
            task_id: t.id.clone(),
            title: t.title.clone(),
            since: t.waiting_since.unwrap_or(t.updated),
            chases: t.chases.len(),
            last_chased: t.chases.last().copied(),
            until: t.until,
            waiting_ref: t.waiting_ref.clone(),
        }
    }
}

/// The tag an agent puts on steps it will perform itself.
pub const AGENT_TAG: &str = "agent";

pub fn is_agent_step(task: &Task) -> bool {
    task.tags.iter().any(|t| t.eq_ignore_ascii_case(AGENT_TAG))
}

pub fn derive(tree: &Tree, task: &Task, today: Date) -> Derived {
    let blocked_by = tree
        .children(&task.id)
        .filter(|c| c.status.is_blocking())
        .count();

    let orphaned = task.status.is_open()
        && tree
            .ancestors(&task.id)
            .iter()
            .any(|a| a.status.is_closed());

    let deferred_by = if task.status == Status::Someday {
        Some(task.id.clone())
    } else {
        tree.ancestors(&task.id)
            .into_iter()
            .find(|a| matches!(a.status, Status::Someday | Status::Waiting))
            .map(|a| a.id.clone())
    };

    let followup = task.status == Status::Waiting && task.until.is_some_and(|u| u <= today);

    // A step tagged `agent` is Claude's to perform, but the human still has
    // to trigger it, so it ranks like any other leaf (with a 🤖 badge).
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

    let waiting_on_summary = if blocked_by > 0 {
        tree.descendants(&task.id)
            .into_iter()
            .filter(|d| d.status == Status::Waiting)
            .min_by_key(|d| d.waiting_since.unwrap_or(d.updated))
            .map(WaitingRef::of)
    } else {
        None
    };

    Derived {
        blocked_by,
        deferred_by,
        followup,
        actionable,
        resurfaced_at,
        orphaned,
        waiting_on_summary,
    }
}

/// One item in the "waiting on" view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingItem {
    #[serde(flatten)]
    pub wait: WaitingRef,
    /// The story this serves, if it is a subtask.
    pub root_title: Option<String>,
    pub root_id: Option<TaskId>,
    pub since_secs: i64,
    /// The follow-up date has arrived.
    pub due_now: bool,
}

/// Everything you are waiting on one person for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonGroup {
    pub person: String,
    pub items: Vec<WaitingItem>,
    pub oldest_secs: i64,
    pub any_due: bool,
}

/// Who am I waiting on, for what, how long? People with a due follow-up
/// first, then whoever has kept you waiting longest.
pub fn waiting_on(tree: &Tree, now: &Zoned, ctx: Option<Context>) -> Vec<PersonGroup> {
    let today = now.date();
    let ts = now.timestamp();
    let mut groups: Vec<PersonGroup> = Vec::new();
    for t in tree
        .iter()
        .filter(|t| t.status == Status::Waiting && in_context(tree, t, ctx))
    {
        let wait = WaitingRef::of(t);
        let root = tree.root_of(&t.id).filter(|r| r.id != t.id);
        let item = WaitingItem {
            since_secs: ts.as_second() - wait.since.as_second(),
            due_now: wait.until.is_some_and(|u| u <= today),
            root_title: root.map(|r| r.title.clone()),
            root_id: root.map(|r| r.id.clone()),
            wait,
        };
        let key = item.wait.person.trim().to_lowercase();
        match groups
            .iter_mut()
            .find(|g| g.person.trim().to_lowercase() == key)
        {
            Some(g) => g.items.push(item),
            None => groups.push(PersonGroup {
                person: item.wait.person.clone(),
                items: vec![item],
                oldest_secs: 0,
                any_due: false,
            }),
        }
    }
    for g in &mut groups {
        g.items.sort_by_key(|i| {
            (
                std::cmp::Reverse(i.due_now),
                std::cmp::Reverse(i.since_secs),
            )
        });
        g.oldest_secs = g.items.iter().map(|i| i.since_secs).max().unwrap_or(0);
        g.any_due = g.items.iter().any(|i| i.due_now);
    }
    groups.sort_by_key(|g| {
        (
            std::cmp::Reverse(g.any_due),
            std::cmp::Reverse(g.oldest_secs),
        )
    });
    groups
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
            let (tier, mut reason, sort_key) = classify(t, &d, today, soon, now_ts);
            if d.orphaned {
                reason.push_str(" · its parent is already finished");
            }
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
            format!("ready: all prerequisites done {} ago", ago(at)),
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
        let since = t.waiting_since.unwrap_or(t.updated);
        let chased = match t.chases.len() {
            0 => String::new(),
            n => format!(", chased {n}×"),
        };
        // Most-chased first, then earliest follow-up date.
        let key = t.until.map(days_since_epoch).unwrap_or(0) - (t.chases.len() as i64) * 100_000;
        return (
            Tier::FollowUp,
            format!("follow up with {who}: waiting {}{chased}", ago(since)),
            key,
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
            t.order_key() as i64,
        ),
        None => (
            Tier::Normal,
            format!("queued {}", ago(t.created)),
            t.order_key() as i64,
        ),
        Some(Priority::Low) => (
            Tier::Low,
            format!("low priority, queued {}", ago(t.created)),
            t.order_key() as i64,
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

#[cfg(test)]
mod orphan_tests {
    use super::*;
    use jiff::Timestamp;

    #[test]
    fn open_child_under_done_parent_is_orphaned_and_actionable() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let ts: Timestamp = now.timestamp();
        let mut parent = Task::new("stry00".parse().unwrap(), "Story", Context::Work, ts);
        parent.status = Status::Done;
        parent.completed = Some(ts);
        let mut kid = Task::new("k1d000".parse().unwrap(), "Leftover", Context::Work, ts);
        kid.parent = Some(parent.id.clone());
        let tree = Tree::from_tasks([parent, kid.clone()]);
        let d = derive(&tree, &kid, now.date());
        assert!(d.orphaned);
        assert!(d.actionable);
        let items = needs_action(&tree, &now, None, &RankConfig::default());
        assert_eq!(items.len(), 1);
        assert!(
            items[0].reason.contains("parent is already finished"),
            "{}",
            items[0].reason
        );
    }
}

#[cfg(test)]
mod waiting_tests {
    use super::*;
    use jiff::{Timestamp, ToSpan};

    #[test]
    fn grouped_by_person_with_due_first_and_parent_summary() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mut dns = Task::new(
            "dns000".parse().unwrap(),
            "Get DNS address",
            Context::Work,
            at(100),
        );
        let mut ask = Task::new(
            "ask000".parse().unwrap(),
            "What's possible?",
            Context::Work,
            at(96),
        );
        ask.parent = Some(dns.id.clone());
        ask.status = Status::Waiting;
        ask.waiting_on = Some("B".into());
        ask.waiting_since = Some(at(96));
        ask.chases = vec![at(48)];
        ask.until = Some(now.date()); // due today
        let mut other = Task::new(
            "0thr00".parse().unwrap(),
            "Budget sign-off",
            Context::Work,
            at(10),
        );
        other.status = Status::Waiting;
        other.waiting_on = Some("c".into());
        other.waiting_since = Some(at(10));
        other.until = Some(now.date().checked_add(3.days()).unwrap());
        let mut same = Task::new(
            "same00".parse().unwrap(),
            "Also from B",
            Context::Work,
            at(5),
        );
        same.status = Status::Waiting;
        same.waiting_on = Some("b".into()); // case-insensitive grouping
        same.waiting_since = Some(at(5));
        same.until = Some(now.date().checked_add(2.days()).unwrap());
        dns.body.clear();
        let tree = Tree::from_tasks([dns.clone(), ask, other, same]);

        let groups = waiting_on(&tree, &now, None);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].person, "B");
        assert!(groups[0].any_due);
        assert_eq!(groups[0].items.len(), 2);
        assert_eq!(
            groups[0].items[0].wait.title, "What's possible?",
            "due item first"
        );
        assert_eq!(
            groups[0].items[0].root_title.as_deref(),
            Some("Get DNS address")
        );
        assert_eq!(groups[0].items[0].wait.chases, 1);
        assert_eq!(groups[1].person, "c");

        let d = derive(&tree, &dns, now.date());
        assert!(!d.actionable);
        let w = d.waiting_on_summary.expect("parent knows who it waits on");
        assert_eq!(w.person, "B");
        assert_eq!(w.task_id.as_str(), "ask000");

        // Follow-ups rank most-chased first.
        let items = needs_action(&tree, &now, None, &RankConfig::default());
        let fu: Vec<&NextItem> = items.iter().filter(|i| i.tier == Tier::FollowUp).collect();
        assert_eq!(fu.len(), 1);
        assert!(fu[0].reason.contains("chased 1×"), "{}", fu[0].reason);
    }
}

#[cfg(test)]
mod agent_step_tests {
    use super::*;

    #[test]
    fn agent_steps_rank_for_the_human_and_block_the_story() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let ts = now.timestamp();
        let story = Task::new("stry00".parse().unwrap(), "Get DNS", Context::Work, ts);
        let mut mine = Task::new(
            "m1ne00".parse().unwrap(),
            "Install gcloud",
            Context::Work,
            ts,
        );
        mine.parent = Some(story.id.clone());
        let mut theirs = Task::new(
            "agnt00".parse().unwrap(),
            "Create the zone",
            Context::Work,
            ts,
        );
        theirs.parent = Some(story.id.clone());
        theirs.tags = vec!["Agent".into()];
        let tree = Tree::from_tasks([story.clone(), mine, theirs.clone()]);

        let items = needs_action(&tree, &now, None, &RankConfig::default());
        let mut ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            ["agnt00", "m1ne00"],
            "the agent step ranks too: a human has to trigger it"
        );
        assert!(derive(&tree, &theirs, now.date()).actionable);
        assert!(is_agent_step(&theirs));
        assert_eq!(
            derive(&tree, &story, now.date()).blocked_by,
            2,
            "and it still blocks the story"
        );
    }
}

// ---------- the plan: Next grouped by story, with what is left ----------

/// One open task that is not doable yet, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaterItem {
    pub id: TaskId,
    pub title: String,
    pub status: Status,
    /// "after Install the gcloud CLI", "✉ Dave · 3d · 1×", "💤 someday", …
    pub why: String,
    pub agent: bool,
    /// How many open tasks still sit under it: 0 means it is next in line.
    pub open_under: usize,
}

/// A story (root task) with what you can do now and what is still required.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryPlan {
    pub id: TaskId,
    pub title: String,
    pub context: Context,
    /// Actionable leaves, ranked (same tiers as `needs_action`).
    pub now: Vec<NextItem>,
    /// Everything else still open, nearest-to-doable first.
    pub later: Vec<LaterItem>,
    pub later_agent: usize,
    /// People this story is waiting on (deduplicated).
    pub waiting_on: Vec<String>,
    /// When `now` is empty: the one-line reason the story is stuck.
    pub stuck: Option<String>,
}

/// "What do I have to do now, and how much is left?" Stories are ordered by
/// their best `now` row (the Next ranking, grouped); stuck stories follow.
pub fn plan(tree: &Tree, now: &Zoned, ctx: Option<Context>, cfg: &RankConfig) -> Vec<StoryPlan> {
    let today = now.date();
    let ts = now.timestamp();
    let items = needs_action(tree, now, ctx, cfg);

    // Group actionable rows by story, in rank order of first appearance.
    let mut order: Vec<TaskId> = Vec::new();
    let mut now_by_root: std::collections::HashMap<TaskId, Vec<NextItem>> = Default::default();
    for it in items {
        let root = tree
            .root_of(&it.id)
            .map(|r| r.id.clone())
            .unwrap_or(it.id.clone());
        if !now_by_root.contains_key(&root) {
            order.push(root.clone());
        }
        now_by_root.entry(root).or_default().push(it);
    }
    // Stories with open work but nothing actionable come after, in tree order.
    for r in tree.roots() {
        if r.status.is_open()
            && r.status != Status::Someday
            && in_context(tree, r, ctx)
            && !order.contains(&r.id)
        {
            order.push(r.id.clone());
        }
    }

    // Outline position for every task, so `now` reads top-down like the tree:
    // siblings are parallel streams, nothing among them says "this one first".
    let mut pos: std::collections::HashMap<TaskId, usize> = Default::default();
    for r in tree.roots() {
        pos.insert(r.id.clone(), pos.len());
        for d in tree.descendants(&r.id) {
            pos.insert(d.id.clone(), pos.len());
        }
    }

    order
        .into_iter()
        .filter_map(|root_id| {
            let root = tree.get(&root_id)?;
            let mut now_rows = now_by_root.remove(&root_id).unwrap_or_default();
            now_rows.sort_by_key(|i| pos.get(&i.id).copied().unwrap_or(usize::MAX));
            let in_now: std::collections::HashSet<&TaskId> =
                now_rows.iter().map(|i| &i.id).collect();
            let mut later: Vec<LaterItem> = std::iter::once(root)
                .chain(tree.descendants(&root_id))
                .filter(|t| t.status.is_open() && !in_now.contains(&t.id))
                .filter(|t| {
                    // Parked subtrees are not "required to finish".
                    t.status != Status::Someday
                        && !tree
                            .ancestors(&t.id)
                            .iter()
                            .any(|a| a.status == Status::Someday)
                })
                .map(|t| {
                    let d = derive(tree, t, today);
                    let open_under = tree
                        .descendants(&t.id)
                        .iter()
                        .filter(|x| x.status.is_open())
                        .count();
                    let why = later_reason(tree, t, &d, ts);
                    LaterItem {
                        id: t.id.clone(),
                        title: t.title.clone(),
                        status: t.status,
                        why,
                        agent: is_agent_step(t),
                        open_under,
                    }
                })
                .collect();
            later.sort_by_key(|l| l.open_under);
            let later_agent = later.iter().filter(|l| l.agent).count();
            let mut waiting_on: Vec<String> = std::iter::once(root)
                .chain(tree.descendants(&root_id))
                .filter(|t| t.status == Status::Waiting)
                .filter_map(|t| t.waiting_on.clone())
                .collect();
            waiting_on.sort();
            waiting_on.dedup();
            let stuck = if now_rows.is_empty() {
                Some(if waiting_on.is_empty() {
                    if later.iter().all(|l| l.agent) {
                        "waiting on Claude".to_string()
                    } else {
                        "blocked".to_string()
                    }
                } else {
                    format!("waiting on {}", waiting_on.join(", "))
                })
            } else {
                None
            };
            if now_rows.is_empty() && later.is_empty() {
                return None;
            }
            Some(StoryPlan {
                id: root_id,
                title: root.title.clone(),
                context: root.context,
                now: now_rows,
                later,
                later_agent,
                waiting_on,
                stuck,
            })
        })
        .collect()
}

fn later_reason(tree: &Tree, t: &Task, d: &Derived, now: Timestamp) -> String {
    let mut parts = Vec::new();
    if is_agent_step(t) {
        parts.push("🤖 Claude".to_string());
    }
    match t.status {
        Status::Waiting => parts.push(format!(
            "✉ {} · {}{}",
            t.waiting_on.as_deref().unwrap_or("?"),
            crate::humanize::since(t.waiting_since.unwrap_or(t.updated), now),
            match t.chases.len() {
                0 => String::new(),
                n => format!(" · {n}×"),
            }
        )),
        Status::Doing => parts.push("▶ doing".to_string()),
        _ => {}
    }
    if d.blocked_by > 0 {
        let kids: Vec<String> = tree
            .children(&t.id)
            .filter(|c| c.status.is_blocking())
            .map(|c| c.title.clone())
            .collect();
        let shown: Vec<&str> = kids.iter().take(2).map(String::as_str).collect();
        let more = if kids.len() > 2 {
            format!(" +{}", kids.len() - 2)
        } else {
            String::new()
        };
        parts.push(format!("after {}{more}", shown.join(", ")));
    }
    if let Some(by) = &d.deferred_by
        && *by != t.id
        && let Some(a) = tree.get(by)
        && a.status == Status::Waiting
    {
        parts.push(format!("under '{}', which is waiting", a.title));
    }
    if parts.is_empty() {
        parts.push("not yet".to_string());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use jiff::{Timestamp, ToSpan};

    #[test]
    fn stories_group_now_and_later_with_reasons() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t
        };
        let mut create = mk("cr8000", "Create the zone", Some("dns000"), 50);
        create.tags = vec!["agent".into()];
        let mut ask = mk("askpr0", "Ask Priya", Some("dns000"), 40);
        ask.status = Status::Waiting;
        ask.waiting_on = Some("Priya".into());
        ask.waiting_since = Some(at(72));
        ask.until = Some(now.date().checked_add(2.days()).unwrap());
        let tasks = vec![
            mk("dns000", "Get DNS address", None, 100),
            mk("dec000", "Decide where it runs", Some("dns000"), 60),
            create,
            mk("10g1n0", "Log in", Some("cr8000"), 45),
            mk("1nst00", "Install the gcloud CLI", Some("10g1n0"), 44),
            ask,
            mk("pass00", "Renew passport", None, 30),
            mk("s1gn00", "Get a guarantor signature", Some("pass00"), 20),
        ];
        let mut tasks = tasks;
        tasks[7].status = Status::Waiting;
        tasks[7].waiting_on = Some("Dave".into());
        tasks[7].waiting_since = Some(at(20));
        tasks[7].until = Some(now.date().checked_add(1.day()).unwrap());
        let tree = Tree::from_tasks(tasks);

        let plans = plan(&tree, &now, None, &RankConfig::default());
        assert_eq!(plans.len(), 2);
        let dns = &plans[0];
        assert_eq!(dns.title, "Get DNS address");
        let now_titles: Vec<&str> = dns.now.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            now_titles,
            ["Decide where it runs", "Install the gcloud CLI"],
            "the leaves you can do"
        );
        let later: Vec<(&str, &str)> = dns
            .later
            .iter()
            .map(|l| (l.title.as_str(), l.why.as_str()))
            .collect();
        assert_eq!(
            later,
            [
                ("Ask Priya", "✉ Priya · 3d"),
                ("Log in", "after Install the gcloud CLI"),
                ("Create the zone", "🤖 Claude · after Log in"),
                (
                    "Get DNS address",
                    "after Decide where it runs, Create the zone +1"
                ),
            ],
            "nearest to doable first, each with why"
        );
        assert_eq!(dns.later_agent, 1);
        assert_eq!(dns.waiting_on, ["Priya"]);
        assert!(dns.stuck.is_none());

        let passport = &plans[1];
        assert!(passport.now.is_empty());
        assert_eq!(passport.stuck.as_deref(), Some("waiting on Dave"));
        assert_eq!(passport.later[0].title, "Get a guarantor signature");
    }
}
