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

use std::collections::{BTreeMap, HashMap};

use jiff::civil::Date;
use jiff::{Timestamp, ToSpan, Zoned};
use serde::{Deserialize, Serialize};

use crate::model::{Context, Priority, Status, Task, TaskId};
use crate::project::ProjectTag;
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
    /// Waits whose follow-up is not yet due (the ones that are due count as actionable).
    pub waiting: usize,
    /// Every open wait, due or not.
    pub waiting_total: usize,
    pub doing: usize,
    pub someday: usize,
}

/// A task belongs to its root's context (CONTEXT: Context), so a subtask
/// moved under a story of the other context follows the story.
pub fn in_context(tree: &Tree, task: &Task, ctx: Option<Context>) -> bool {
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

/// The task in hand (CONTEXT: The hand): the most recently started task that
/// is open and unblocked, or nothing. One rule, shared by every client.
pub fn in_hand<'a>(tree: &'a Tree, now: &Zoned, ctx: Option<Context>) -> Option<&'a Task> {
    let today = now.date();
    // Actionable covers both "unblocked" and "not under a waiting/parked
    // ancestor": a doing task you cannot see on Next is not in hand.
    tree.iter()
        .filter(|t| t.status == Status::Doing && in_context(tree, t, ctx))
        .filter(|t| derive(tree, t, today).actionable)
        .max_by_key(|t| t.started.unwrap_or(t.created))
}

/// What the top plate on Next shows (CONTEXT: The plate).
#[derive(Clone, Copy, Debug)]
pub struct Plate<'a> {
    pub task: &'a Task,
    /// `true`: the task in hand (started). `false`: up next, a ready row
    /// nobody has started; the plate offers *start* instead of *put back*.
    pub in_hand: bool,
}

/// The plate is never empty while something is ready (ADR 0014): the task
/// in hand, else *up next* — the first ready row in Next order, preferring
/// rows never started, so putting a task back brings up another one instead
/// of the same one again.
pub fn plate<'a>(
    tree: &'a Tree,
    now: &Zoned,
    ctx: Option<Context>,
    cfg: &RankConfig,
) -> Option<Plate<'a>> {
    if let Some(task) = in_hand(tree, now, ctx) {
        return Some(Plate {
            task,
            in_hand: true,
        });
    }
    let ids: Vec<TaskId> = plan(tree, now, ctx, cfg)
        .into_iter()
        .flat_map(|p| p.now)
        .map(|i| i.id)
        .collect();
    let rows = || ids.iter().filter_map(|id| tree.get(id));
    rows()
        .find(|t| t.started.is_none())
        .or_else(|| rows().next())
        .map(|task| Plate {
            task,
            in_hand: false,
        })
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
            Status::Waiting => {
                s.waiting_total += 1;
                if !d.followup {
                    s.waiting += 1;
                }
            }
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

// ---------- Day log (CONTEXT: Day log) ----------

/// One finished task in a day's outline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DayLogItem {
    pub id: TaskId,
    pub title: String,
    pub status: Status,
    pub completed: Timestamp,
    /// The ancestors between the nearest task shown above this one (the story
    /// heading, or an ancestor that finished the same day) and this task,
    /// story side first. Drawn dimmed after the title, so a leaf that finished
    /// alone still says what it was for.
    pub chain: Vec<Crumb>,
    /// Indent inside the story's outline for the day: how many ancestors are
    /// themselves items of the same `StoryLog`.
    pub depth: usize,
    pub tags: Vec<String>,
}

/// What one story got done on one day, in outline order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryLog {
    pub root_id: TaskId,
    pub root_title: String,
    pub project_tag: Option<ProjectTag>,
    /// The story itself finished this day; it is then `items[0]` at depth 0.
    pub root_done: bool,
    pub items: Vec<DayLogItem>,
}

/// Everything finished on one calendar day, grouped by story.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DayLog {
    pub day: Date,
    pub stories: Vec<StoryLog>,
    pub count: usize,
}

/// The day log (CONTEXT: Day log): for every day in `from..=to`, newest
/// first, the tasks closed that day in the zone of `now`, grouped by story
/// and kept in outline order so a parent closed by a cascade precedes its
/// subtasks. A flat list would lose what each row was for, which is the one
/// thing this view is about. Days with nothing finished are included with an
/// empty `stories`. Dropped tasks are left out unless `include_dropped`.
pub fn day_log(
    tree: &Tree,
    now: &Zoned,
    ctx: Option<Context>,
    from: Date,
    to: Date,
    include_dropped: bool,
) -> Vec<DayLog> {
    let tz = now.time_zone();
    let mut by_day: BTreeMap<Date, HashMap<TaskId, Timestamp>> = BTreeMap::new();
    for t in tree.iter() {
        let keep = match t.status {
            Status::Done => true,
            Status::Dropped => include_dropped,
            _ => false,
        };
        if !keep || !in_context(tree, t, ctx) {
            continue;
        }
        // A closed task without `completed` was edited by hand; it has no day.
        let Some(c) = t.completed else { continue };
        let day = c.to_zoned(tz.clone()).date();
        if day < from || day > to {
            continue;
        }
        by_day.entry(day).or_default().insert(t.id.clone(), c);
    }
    let mut out = Vec::new();
    let mut day = to;
    while day >= from {
        let picked = by_day.remove(&day).unwrap_or_default();
        // Stories in the order of their earliest completion that day.
        let mut roots: Vec<(Timestamp, TaskId)> = Vec::new();
        for (id, c) in &picked {
            let root = tree
                .root_of(id)
                .map(|r| r.id.clone())
                .unwrap_or_else(|| id.clone());
            match roots.iter_mut().find(|(_, r)| *r == root) {
                Some(e) => e.0 = e.0.min(*c),
                None => roots.push((*c, root)),
            }
        }
        roots.sort();
        let mut stories = Vec::new();
        for (_, root) in roots {
            let Some(r) = tree.get(&root) else { continue };
            let mut items = Vec::new();
            walk_day_log(tree, &root, &picked, &mut Vec::new(), &mut items);
            stories.push(StoryLog {
                root_id: root.clone(),
                root_title: r.title.clone(),
                project_tag: crate::project::tag_of(tree, &root),
                root_done: picked.contains_key(&root),
                items,
            });
        }
        let count = stories.iter().map(|s| s.items.len()).sum();
        out.push(DayLog {
            day,
            stories,
            count,
        });
        day = match day.yesterday() {
            Ok(d) => d,
            Err(_) => break,
        };
    }
    out
}

/// Depth-first in outline order. `path` holds the ancestors from the root
/// down (root first) with whether each is itself an item of this day.
fn walk_day_log(
    tree: &Tree,
    id: &TaskId,
    picked: &HashMap<TaskId, Timestamp>,
    path: &mut Vec<(Crumb, bool)>,
    items: &mut Vec<DayLogItem>,
) {
    let Some(t) = tree.get(id) else { return };
    let is_item = picked.contains_key(id);
    if is_item {
        let depth = path.iter().filter(|(_, item)| *item).count();
        // The chain starts after the nearest shown ancestor: the last one
        // that is an item, else the root (the story heading shows it).
        let cut = path
            .iter()
            .rposition(|(_, item)| *item)
            .map(|i| i + 1)
            .unwrap_or(1)
            .min(path.len());
        items.push(DayLogItem {
            id: id.clone(),
            title: t.title.clone(),
            status: t.status,
            completed: picked[id],
            chain: path[cut..].iter().map(|(c, _)| c.clone()).collect(),
            depth,
            tags: t.tags.clone(),
        });
    }
    path.push((
        Crumb {
            id: id.clone(),
            title: t.title.clone(),
        },
        is_item,
    ));
    let kids: Vec<TaskId> = tree.children(id).map(|c| c.id.clone()).collect();
    for k in &kids {
        walk_day_log(tree, k, picked, path, items);
    }
    path.pop();
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

    // ---------- day log ----------

    fn done_at(fx: &mut Fx, id: &str, parent: Option<&str>, at: &str) {
        fx_add(fx, id, parent, Status::Done, 1);
        let ts: Timestamp = at.parse().unwrap();
        fx.set(id, |t| t.completed = Some(ts));
    }
    fn log_titles(d: &DayLog) -> Vec<(String, usize, Vec<String>)> {
        d.stories
            .iter()
            .flat_map(|s| {
                s.items.iter().map(|i| {
                    (
                        i.title.clone(),
                        i.depth,
                        i.chain.iter().map(|c| c.title.clone()).collect(),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn day_log_buckets_by_the_local_day_of_now() {
        let mut fx = Fx::new();
        fx.now = "2026-09-29T15:00:00[America/Toronto]".parse().unwrap();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        // 02:00Z on the 29th is still the evening of the 28th in Toronto.
        done_at(&mut fx, "eve000", Some("stry00"), "2026-09-29T02:00:00Z");
        done_at(&mut fx, "mdday0", Some("stry00"), "2026-09-29T16:00:00Z");
        let log = day_log(
            &fx.tree,
            &fx.now,
            None,
            date(2026, 9, 27),
            date(2026, 9, 29),
            false,
        );
        assert_eq!(
            log.iter().map(|d| (d.day, d.count)).collect::<Vec<_>>(),
            [
                (date(2026, 9, 29), 1),
                (date(2026, 9, 28), 1),
                (date(2026, 9, 27), 0)
            ],
            "newest first, empty days kept"
        );
        assert_eq!(log[0].stories[0].items[0].title, "mdday0");
        assert_eq!(log[1].stories[0].items[0].title, "eve000");
        assert!(log[2].stories.is_empty());
    }

    #[test]
    fn day_log_is_an_outline_per_story_not_a_flat_list() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        fx_add(&mut fx, "sec000", None, Status::Todo, 48);
        // A cascade: the parent and both subtasks closed in one tick.
        done_at(&mut fx, "par000", Some("stry00"), "2026-09-29T10:00:00Z");
        done_at(&mut fx, "kd1000", Some("par000"), "2026-09-29T10:00:00Z");
        done_at(&mut fx, "kd2000", Some("par000"), "2026-09-29T10:00:00Z");
        // A leaf whose parent is still open: the chain names the parent.
        // (Created after par000, so it comes after it in outline order.)
        fx_add(&mut fx, "0pen00", Some("stry00"), Status::Todo, 0);
        fx_add(&mut fx, "mdd000", Some("0pen00"), Status::Todo, 0);
        done_at(&mut fx, "eaf000", Some("mdd000"), "2026-09-29T09:00:00Z");
        // Another story, finished later in the day, so it comes second.
        done_at(&mut fx, "ssss00", Some("sec000"), "2026-09-29T12:00:00Z");
        let log = day_log(
            &fx.tree,
            &fx.now,
            None,
            date(2026, 9, 29),
            date(2026, 9, 29),
            false,
        );
        assert_eq!(log.len(), 1);
        let d = &log[0];
        assert_eq!(d.count, 5);
        assert_eq!(
            d.stories
                .iter()
                .map(|s| s.root_title.as_str())
                .collect::<Vec<_>>(),
            ["stry00", "sec000"],
            "stories in the order of their earliest completion"
        );
        assert!(!d.stories[0].root_done);
        let o = |s: &str| s.to_string();
        assert_eq!(
            log_titles(d),
            [
                (o("par000"), 0, vec![]),
                (o("kd1000"), 1, vec![]),
                (o("kd2000"), 1, vec![]),
                (o("eaf000"), 0, vec![o("0pen00"), o("mdd000")]),
                (o("ssss00"), 0, vec![]),
            ],
            "outline order: parent then nested subtasks; a lone leaf carries its chain"
        );
    }

    #[test]
    fn day_log_chain_stops_at_an_ancestor_finished_the_same_day() {
        let mut fx = Fx::new();
        fx_add(&mut fx, "stry00", None, Status::Todo, 48);
        done_at(&mut fx, "par000", Some("stry00"), "2026-09-29T10:00:00Z");
        fx_add(&mut fx, "mdd000", Some("par000"), Status::Todo, 20);
        done_at(&mut fx, "eaf000", Some("mdd000"), "2026-09-29T11:00:00Z");
        // Finished yesterday: it is shown there, not today, so today's leaf
        // chain names it.
        done_at(&mut fx, "yest00", Some("stry00"), "2026-09-28T10:00:00Z");
        done_at(&mut fx, "ndr000", Some("yest00"), "2026-09-29T08:00:00Z");
        let log = day_log(
            &fx.tree,
            &fx.now,
            None,
            date(2026, 9, 28),
            date(2026, 9, 29),
            false,
        );
        let o = |s: &str| s.to_string();
        assert_eq!(
            log_titles(&log[0]),
            [
                (o("par000"), 0, vec![]),
                (o("eaf000"), 1, vec![o("mdd000")]),
                (o("ndr000"), 0, vec![o("yest00")]),
            ]
        );
        assert_eq!(log_titles(&log[1]), [(o("yest00"), 0, vec![])]);
    }

    #[test]
    fn day_log_root_finished_alone_and_filters() {
        let mut fx = Fx::new();
        done_at(&mut fx, "stry00", None, "2026-09-29T10:00:00Z");
        fx_add(&mut fx, "stry10", None, Status::Todo, 48);
        done_at(&mut fx, "drpp00", Some("stry10"), "2026-09-29T10:30:00Z");
        fx.set("drpp00", |t| t.status = Status::Dropped);
        done_at(&mut fx, "hand00", Some("stry10"), "2026-09-29T10:40:00Z");
        fx.set("hand00", |t| t.completed = None);
        fx_add(&mut fx, "pers00", None, Status::Todo, 48);
        fx.set("pers00", |t| t.context = Context::Personal);
        done_at(&mut fx, "pers10", Some("pers00"), "2026-09-29T11:00:00Z");
        let today = date(2026, 9, 29);
        let log = day_log(&fx.tree, &fx.now, None, today, today, false);
        assert_eq!(
            log[0].count, 2,
            "root alone + the personal leaf; dropped and hand-edited skipped"
        );
        let s = &log[0].stories[0];
        assert!(s.root_done);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].depth, 0);
        assert!(s.items[0].chain.is_empty());
        let with_dropped = day_log(&fx.tree, &fx.now, None, today, today, true);
        assert_eq!(with_dropped[0].count, 3);
        assert_eq!(with_dropped[0].stories[1].items[0].status, Status::Dropped);
        let work = day_log(&fx.tree, &fx.now, Some(Context::Work), today, today, false);
        assert_eq!(work[0].count, 1);
        assert!(
            day_log(&fx.tree, &fx.now, None, today, date(2026, 9, 28), false).is_empty(),
            "from after to is empty"
        );
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

/// "What do I have to do now, and how much is left?" Stories keep their
/// outline order (ADR 0013): finishing a row never moves its story, so the
/// boxes on Next stay where the eye left them. Stories with something to do
/// now come first; stuck stories follow, in the same order.
pub fn plan(tree: &Tree, now: &Zoned, ctx: Option<Context>, cfg: &RankConfig) -> Vec<StoryPlan> {
    let today = now.date();
    let ts = now.timestamp();
    let items = needs_action(tree, now, ctx, cfg);

    // Group actionable rows by story.
    let mut now_by_root: std::collections::HashMap<TaskId, Vec<NextItem>> = Default::default();
    for it in items {
        let root = tree
            .root_of(&it.id)
            .map(|r| r.id.clone())
            .unwrap_or(it.id.clone());
        now_by_root.entry(root).or_default().push(it);
    }
    // Stories in outline order, never by rank: a row's tier badges it, it
    // does not move its story. Those with something to do now first, then
    // the ones with open work but nothing actionable.
    let roots = tree.roots();
    let doable = roots.iter().filter(|r| now_by_root.contains_key(&r.id));
    let stuck = roots.iter().filter(|r| {
        !now_by_root.contains_key(&r.id)
            && r.status.is_open()
            && r.status != Status::Someday
            && in_context(tree, r, ctx)
    });
    let order: Vec<TaskId> = doable.chain(stuck).map(|r| r.id.clone()).collect();

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

    #[test]
    fn plan_skips_parked_stories_and_handles_an_empty_tree() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        assert!(plan(&Tree::new(), &now, None, &RankConfig::default()).is_empty());
        let mk = |id: &str, title: &str, parent: Option<&str>, status: Status| {
            let mut t = Task::new(
                id.parse::<TaskId>().unwrap(),
                title,
                Context::Work,
                now.timestamp(),
            );
            t.parent = parent.map(|p| p.parse().unwrap());
            t.status = status;
            t
        };
        let tree = Tree::from_tasks([
            mk("z1g000", "Learn Zig", None, Status::Someday),
            mk("ch4p00", "Read chapter one", Some("z1g000"), Status::Todo),
            mk("d0ne00", "Finished story", None, Status::Done),
            mk("1nb0x0", "Clear the inbox", None, Status::Todo),
        ]);
        let plans = plan(&tree, &now, None, &RankConfig::default());
        let titles: Vec<&str> = plans.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Clear the inbox"],
            "parked subtrees and closed stories are not plans"
        );
        assert!(plans[0].later.is_empty(), "a lone leaf has nothing later");
    }

    /// Reported 2026-10-07: ticking a row made the story boxes on Next swap
    /// places. Stories were ordered by their best-ranked row, so finishing
    /// the row that carried the rank re-ranked the story (ADR 0013).
    #[test]
    fn finishing_a_row_never_moves_its_story() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t
        };
        // Outline order: Homebrew first, then the shop. The shop has the
        // higher-ranked rows (overdue, then doing); Homebrew only plain todos.
        let mut overdue = mk("ebay00", "Get the eBay details", Some("shp000"), 30);
        overdue.due = Some(now.date().checked_sub(2.days()).unwrap());
        let mut doing = mk("adapt0", "Write the eBay adapter", Some("shp000"), 28);
        doing.status = Status::Doing;
        doing.started = Some(at(5));
        let mut tasks = vec![
            mk("brew00", "Get taskstaka on Homebrew", None, 100),
            mk("rst000", "Install Rust", Some("brew00"), 90),
            mk("c10ne0", "Clone the repo", Some("brew00"), 89),
            mk("shp000", "Sell the collection", None, 80),
            overdue,
            doing,
            mk("endpt0", "Register the endpoint", Some("shp000"), 20),
        ];
        let titles = |tree: &Tree| -> Vec<String> {
            plan(tree, &now, None, &RankConfig::default())
                .into_iter()
                .map(|p| p.title)
                .collect()
        };
        let before = titles(&Tree::from_tasks(tasks.clone()));
        assert_eq!(
            before,
            ["Get taskstaka on Homebrew", "Sell the collection"],
            "outline order, even though the shop holds the overdue and doing rows"
        );

        // Tick the overdue row, then the doing one: the boxes stay put.
        for id in ["ebay00", "adapt0"] {
            let t = tasks.iter_mut().find(|t| t.id.as_str() == id).unwrap();
            t.status = Status::Done;
            t.completed = Some(now.timestamp());
            assert_eq!(
                titles(&Tree::from_tasks(tasks.clone())),
                before,
                "after {id}"
            );
        }

        // Only running out of doable rows moves a story: stuck ones go last.
        let t = tasks
            .iter_mut()
            .find(|t| t.id.as_str() == "endpt0")
            .unwrap();
        t.status = Status::Waiting;
        t.waiting_on = Some("Dave".into());
        t.waiting_since = Some(now.timestamp());
        t.until = Some(now.date().checked_add(2.days()).unwrap());
        let plans = plan(
            &Tree::from_tasks(tasks.clone()),
            &now,
            None,
            &RankConfig::default(),
        );
        assert_eq!(plans[1].title, "Sell the collection");
        assert_eq!(plans[1].stuck.as_deref(), Some("waiting on Dave"));
    }
}

#[cfg(test)]
mod in_hand_tests {
    use super::*;
    use jiff::{Timestamp, ToSpan};

    fn fixture() -> (Vec<Task>, Zoned) {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, ctx: Context| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, ctx, at(100));
            t.parent = parent.map(|p| p.parse().unwrap());
            t
        };
        let mut a = mk("aaa000", "A", None, Context::Work);
        a.status = Status::Doing;
        a.started = Some(at(5));
        let mut b = mk("bbb000", "B", None, Context::Personal);
        b.status = Status::Doing;
        b.started = Some(at(2));
        (vec![a, b], now)
    }

    #[test]
    fn most_recently_started_wins_and_restarting_makes_it_newest() {
        let (mut tasks, now) = fixture();
        let tree = Tree::from_tasks(tasks.clone());
        assert_eq!(in_hand(&tree, &now, None).unwrap().id.as_str(), "bbb000");
        // A is put back and started again: its `started` is *this* start.
        tasks[0].started = Some(now.timestamp());
        let tree = Tree::from_tasks(tasks);
        assert_eq!(in_hand(&tree, &now, None).unwrap().id.as_str(), "aaa000");
    }

    #[test]
    fn context_filters_and_nothing_is_nothing() {
        let (tasks, now) = fixture();
        let tree = Tree::from_tasks(tasks);
        assert_eq!(
            in_hand(&tree, &now, Some(Context::Work))
                .unwrap()
                .id
                .as_str(),
            "aaa000"
        );
        assert_eq!(
            in_hand(&tree, &now, Some(Context::Personal))
                .unwrap()
                .id
                .as_str(),
            "bbb000"
        );
        assert!(in_hand(&Tree::new(), &now, None).is_none());
    }

    #[test]
    fn blocked_or_deferred_doing_tasks_are_not_in_hand() {
        let (mut tasks, now) = fixture();
        // B gets an open prerequisite: blocked, so A (older) is in hand.
        let mut kid = Task::new(
            "k1d000".parse().unwrap(),
            "first this",
            Context::Personal,
            now.timestamp(),
        );
        kid.parent = Some("bbb000".parse().unwrap());
        tasks.push(kid);
        let tree = Tree::from_tasks(tasks.clone());
        assert_eq!(in_hand(&tree, &now, None).unwrap().id.as_str(), "aaa000");
        // A doing task under a waiting parent is deferred: not on Next, so
        // not in hand either.
        tasks.pop();
        let mut parent = Task::new(
            "par000".parse().unwrap(),
            "Parent",
            Context::Work,
            now.timestamp(),
        );
        parent.status = Status::Waiting;
        parent.waiting_on = Some("Bo".into());
        tasks[0].parent = Some(parent.id.clone());
        tasks.push(parent);
        let tree = Tree::from_tasks(tasks);
        assert_eq!(in_hand(&tree, &now, None).unwrap().id.as_str(), "bbb000");
        assert!(in_hand(&tree, &now, Some(Context::Work)).is_none());
    }

    /// ADR 0014: the plate is never empty while something is ready.
    #[test]
    fn plate_falls_back_to_the_first_never_started_ready_row() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t
        };
        let cfg = RankConfig::default();
        let mut tasks = vec![
            mk("brew00", "Get taskstaka on Homebrew", None, 100),
            mk("rst000", "Install Rust", Some("brew00"), 90),
            mk("c10ne0", "Clone the repo", Some("brew00"), 89),
            mk("t3st00", "Test on macOS", Some("brew00"), 88),
        ];
        let who = |tasks: &[Task]| -> (String, bool) {
            let tree = Tree::from_tasks(tasks.iter().cloned());
            let p = plate(&tree, &now, None, &cfg).expect("something is ready");
            (p.task.title.clone(), p.in_hand)
        };
        assert_eq!(
            who(&tasks),
            ("Install Rust".into(), false),
            "nothing started: the first ready row, up next"
        );

        // Start it: in hand. Put it back: the next never-started row is up.
        tasks[1].status = Status::Doing;
        tasks[1].started = Some(at(1));
        assert_eq!(who(&tasks), ("Install Rust".into(), true));
        tasks[1].status = Status::Todo;
        assert_eq!(
            who(&tasks),
            ("Clone the repo".into(), false),
            "put back: another one comes up, not the same one"
        );
        tasks[2].started = Some(at(1));
        assert_eq!(who(&tasks), ("Test on macOS".into(), false));
        tasks[3].started = Some(at(1));
        assert_eq!(
            who(&tasks),
            ("Install Rust".into(), false),
            "everything was put back once: round again from the top"
        );

        // Nothing ready at all: no plate.
        for t in tasks.iter_mut().skip(1) {
            t.status = Status::Done;
            t.completed = Some(now.timestamp());
        }
        tasks[0].status = Status::Done;
        tasks[0].completed = Some(now.timestamp());
        let tree = Tree::from_tasks(tasks.clone());
        assert!(plate(&tree, &now, None, &cfg).is_none());
    }
}
