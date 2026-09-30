//! tododo-core: the task model, the markdown-backed store, and the needs-action
//! computation. This crate is synchronous and free of any server/UI framework so
//! that the product logic can be unit-tested with an injected clock.

pub mod action;
pub mod command;
pub mod frontmatter;
pub mod humanize;
pub mod index_md;
pub mod model;
pub mod store;
pub mod tree;
pub mod view;

pub use action::{NextItem, RankConfig, Tier, needs_action};
pub use command::{Command, Event, Outcome};
pub use model::{Context, Priority, Status, Task, TaskId};
pub use store::{Store, StoreError};
pub use tree::Tree;
pub use view::TaskView;
