//! The only mutation API. Every client (CLI, HTTP, MCP) turns its input into
//! a `Command`; `Store::apply` validates it and returns an `Outcome`.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::model::{Context, Priority, Status, TaskId};

/// Three-state field for partial edits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Set<T> {
    #[default]
    Keep,
    Clear,
    To(T),
}

impl<T> Set<T> {
    pub fn apply_to(self, slot: &mut Option<T>) -> bool {
        match self {
            Set::Keep => false,
            Set::Clear => {
                let changed = slot.is_some();
                *slot = None;
                changed
            }
            Set::To(v) => {
                *slot = Some(v);
                true
            }
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AddTask {
    pub title: String,
    pub parent: Option<TaskId>,
    /// Defaults to the parent's root context, else `Context::default()`.
    pub context: Option<Context>,
    pub priority: Option<Priority>,
    pub due: Option<Date>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub body: String,
    /// Create directly in this status (e.g. `someday`). Defaults to `todo`.
    pub status: Option<Status>,
    /// Place the new task right after this sibling. Omit to append at the end.
    pub after: Option<TaskId>,
    /// Create it already waiting on this person (`status` is then ignored).
    pub waiting_on: Option<String>,
    /// Follow-up date for `waiting_on`; defaults per store config.
    pub until: Option<Date>,
    pub waiting_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditTask {
    pub id: TaskId,
    pub title: Option<String>,
    pub context: Option<Context>,
    #[serde(default)]
    pub priority: Set<Priority>,
    #[serde(default)]
    pub due: Set<Date>,
    pub tags: Option<Vec<String>>,
    pub body: Option<String>,
    /// Handoff: change who a waiting task waits on (journaled). Only valid
    /// while waiting; `Clear` is refused (use a status change instead).
    #[serde(default)]
    pub waiting_on: Set<String>,
    #[serde(default)]
    pub waiting_ref: Set<String>,
}

impl EditTask {
    /// An edit that changes nothing; set the fields you want.
    pub fn new(id: TaskId) -> Self {
        EditTask {
            id,
            title: None,
            context: None,
            priority: Set::Keep,
            due: Set::Keep,
            tags: None,
            body: None,
            waiting_on: Set::Keep,
            waiting_ref: Set::Keep,
        }
    }
}

/// What came back from the person you were waiting on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplyOutcome {
    /// They answered: the task is yours again (`todo`).
    Answered,
    /// They pointed you at someone else: keep waiting, on that person.
    Redirected { to: String },
    /// Something, but not enough: keep waiting, follow-up reset.
    Partial,
    /// They said no: the task is yours again to decide what's next.
    Declined,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    Add(AddTask),
    SetStatus {
        id: TaskId,
        status: Status,
        /// Required when `status == Waiting`.
        waiting_on: Option<String>,
        until: Option<Date>,
        /// When closing: also close every open descendant with the same status.
        #[serde(default)]
        cascade: bool,
    },
    Edit(EditTask),
    Move {
        id: TaskId,
        parent: Option<TaskId>,
        /// Place it right after this sibling. Omit to append at the end.
        #[serde(default)]
        after: Option<TaskId>,
    },
    Touch {
        id: TaskId,
        /// Appended to the body as a timestamped journal line.
        note: Option<String>,
    },
    /// You poked the person again. Requires `waiting`. Shortens the next
    /// follow-up and records the chase.
    Chase {
        id: TaskId,
        until: Option<Date>,
        note: Option<String>,
        /// Who is acting, e.g. "agent"; tagged in the journal.
        #[serde(default)]
        source: Option<String>,
        /// Idempotency key (e.g. today's date): a repeat with the same key
        /// changes nothing and reports `deduped`.
        #[serde(default)]
        dedupe: Option<String>,
    },
    /// The person got back to you. Requires `waiting`.
    RecordReply {
        id: TaskId,
        outcome: ReplyOutcome,
        /// One line on what they said; always journaled.
        summary: String,
        #[serde(default)]
        waiting_ref: Option<String>,
        #[serde(default)]
        source: Option<String>,
    },
    Delete {
        id: TaskId,
        /// Also delete descendants. Without it, only leaves may be deleted.
        #[serde(default)]
        force: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Created {
        id: TaskId,
    },
    StatusChanged {
        id: TaskId,
        from: Status,
        to: Status,
    },
    /// A parent's last blocking child closed; it is actionable again.
    Resurfaced {
        id: TaskId,
    },
    /// A descendant was closed along with its parent (`cascade`).
    CascadeClosed {
        id: TaskId,
        status: Status,
    },
    Chased {
        id: TaskId,
        person: String,
        /// Total chases during this wait, after this one.
        count: usize,
        /// The dedupe key matched an earlier chase; nothing changed.
        deduped: bool,
    },
    ReplyReceived {
        id: TaskId,
        person: String,
        outcome: ReplyOutcome,
    },
    /// `waiting_on` changed while still waiting.
    HandedOff {
        id: TaskId,
        from: String,
        to: String,
    },
    Deleted {
        id: TaskId,
    },
    /// The task's file had changed on disk since load; disk won.
    ReloadedFromDisk {
        id: TaskId,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Outcome {
    /// The task the command was about (for Add: the new task).
    pub primary: Option<TaskId>,
    /// Every task whose file was written or removed.
    pub affected: Vec<TaskId>,
    pub events: Vec<Event>,
    /// Non-fatal advice, e.g. "waiting on a task with open children".
    pub warnings: Vec<String>,
}
