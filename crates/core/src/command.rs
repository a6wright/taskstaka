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
        }
    }
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
        /// When closing: also drop every open descendant.
        #[serde(default)]
        cascade: bool,
    },
    Edit(EditTask),
    Move {
        id: TaskId,
        parent: Option<TaskId>,
    },
    Touch {
        id: TaskId,
        /// Appended to the body as a timestamped journal line.
        note: Option<String>,
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
    /// Reopening a child reopened a closed ancestor.
    AutoReopened {
        id: TaskId,
    },
    CascadeDropped {
        id: TaskId,
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
