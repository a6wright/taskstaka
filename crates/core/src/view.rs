//! Wire DTOs shared by the daemon and every client. `Task` itself skips its
//! body when serialized (that is the frontmatter shape), so the API carries a
//! `TaskView` that adds the body, the derived facts and the file path.

use std::path::PathBuf;

use jiff::Zoned;
use serde::{Deserialize, Serialize};

use crate::action::{Derived, derive};
use crate::model::{Task, TaskId};
use crate::tree::Tree;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub body: String,
    pub derived: Derived,
    /// Direct children, creation order.
    pub children: Vec<TaskId>,
    /// Root first, immediate parent last.
    pub ancestry: Vec<TaskId>,
    /// Absolute path of the note; only the daemon knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl TaskView {
    pub fn build(tree: &Tree, task: &Task, now: &Zoned, path: Option<PathBuf>) -> Self {
        let mut ancestry: Vec<TaskId> = tree
            .ancestors(&task.id)
            .into_iter()
            .map(|a| a.id.clone())
            .collect();
        ancestry.reverse();
        TaskView {
            task: task.clone(),
            body: task.body.clone(),
            derived: derive(tree, task, now.date()),
            children: tree.children(&task.id).map(|c| c.id.clone()).collect(),
            ancestry,
            path,
        }
    }

    /// Rebuild the stored task (body included) on the client side.
    pub fn into_task(self) -> Task {
        let mut t = self.task;
        t.body = self.body;
        t
    }
}

/// A `Tree` rebuilt from views; what a client works with locally.
pub fn tree_from_views(views: Vec<TaskView>) -> Tree {
    Tree::from_tasks(views.into_iter().map(TaskView::into_task))
}

/// `GET /next`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NextResponse {
    pub rev: u64,
    pub summary: crate::action::Summary,
    pub items: Vec<crate::action::NextItem>,
}

/// `GET /tree`: the nested shape for UIs that render the hierarchy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeNode {
    pub task: TaskView,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn build(tree: &Tree, task: &Task, now: &Zoned, include_closed: bool) -> Self {
        TreeNode {
            task: TaskView::build(tree, task, now, None),
            children: tree
                .children(&task.id)
                .filter(|c| include_closed || c.status.is_open())
                .map(|c| TreeNode::build(tree, c, now, include_closed))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Context;

    #[test]
    fn view_roundtrips_body_and_flattens_task_fields() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let mut t = Task::new(
            "abc123".parse().unwrap(),
            "Hello",
            Context::Work,
            now.timestamp(),
        );
        t.body = "notes\n".into();
        let tree = Tree::from_tasks([t.clone()]);
        let view = TaskView::build(&tree, &t, &now, None);
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"title\":\"Hello\""), "{json}");
        assert!(json.contains("\"body\":\"notes\\n\""), "{json}");
        let back: TaskView = serde_json::from_str(&json).unwrap();
        assert_eq!(back.into_task(), t);
    }
}
