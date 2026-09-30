//! In-memory task collection with parent/child indexing.

use std::collections::{BTreeMap, HashMap};

use crate::model::{Task, TaskId};

#[derive(Clone, Debug, Default)]
pub struct Tree {
    tasks: BTreeMap<TaskId, Task>,
    children: HashMap<TaskId, Vec<TaskId>>,
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_tasks(tasks: impl IntoIterator<Item = Task>) -> Self {
        let mut tree = Tree::new();
        for t in tasks {
            tree.insert(t);
        }
        tree
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn contains(&self, id: &TaskId) -> bool {
        self.tasks.contains_key(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    /// Insert or replace a task, keeping the child index consistent.
    pub fn insert(&mut self, task: Task) {
        if let Some(old) = self.tasks.get(&task.id)
            && old.parent != task.parent
            && let Some(p) = &old.parent
            && let Some(list) = self.children.get_mut(p)
        {
            list.retain(|c| c != &task.id);
        }
        let already_linked = self
            .tasks
            .get(&task.id)
            .is_some_and(|old| old.parent == task.parent);
        if !already_linked && let Some(p) = &task.parent {
            let list = self.children.entry(p.clone()).or_default();
            if !list.contains(&task.id) {
                list.push(task.id.clone());
            }
        }
        self.tasks.insert(task.id.clone(), task);
    }

    pub fn remove(&mut self, id: &TaskId) -> Option<Task> {
        let task = self.tasks.remove(id)?;
        if let Some(p) = &task.parent
            && let Some(list) = self.children.get_mut(p)
        {
            list.retain(|c| c != id);
        }
        self.children.remove(id);
        Some(task)
    }

    /// Direct children, in creation order.
    pub fn children(&self, id: &TaskId) -> impl Iterator<Item = &Task> {
        let mut ids: Vec<&TaskId> = self
            .children
            .get(id)
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        ids.sort_by(|a, b| {
            let (ta, tb) = (self.tasks.get(a), self.tasks.get(b));
            let ka = ta.map(|t| (t.order_key(), t.created));
            let kb = tb.map(|t| (t.order_key(), t.created));
            ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
        });
        ids.into_iter().filter_map(|c| self.tasks.get(c))
    }

    pub fn has_children(&self, id: &TaskId) -> bool {
        self.children.get(id).is_some_and(|v| !v.is_empty())
    }

    /// Ancestors from the immediate parent up to the root.
    pub fn ancestors(&self, id: &TaskId) -> Vec<&Task> {
        let mut out = Vec::new();
        let mut cur = self.tasks.get(id).and_then(|t| t.parent.as_ref());
        while let Some(pid) = cur {
            match self.tasks.get(pid) {
                Some(p) => {
                    out.push(p);
                    cur = p.parent.as_ref();
                }
                None => break, // dangling parent: treat as root
            }
            if out.len() > self.tasks.len() {
                break; // defensive against a cycle on disk
            }
        }
        out
    }

    pub fn root_of(&self, id: &TaskId) -> Option<&Task> {
        let anc = self.ancestors(id);
        anc.last().copied().or_else(|| self.tasks.get(id))
    }

    /// All descendants, depth-first, pre-order.
    pub fn descendants(&self, id: &TaskId) -> Vec<&Task> {
        let mut out = Vec::new();
        let mut stack: Vec<&TaskId> = self.children(id).map(|c| &c.id).collect();
        stack.reverse();
        while let Some(cid) = stack.pop() {
            if let Some(t) = self.tasks.get(cid) {
                out.push(t);
                let mut kids: Vec<&TaskId> = self.children(cid).map(|c| &c.id).collect();
                kids.reverse();
                stack.extend(kids);
            }
        }
        out
    }

    /// Tasks whose parent is missing or None, in creation order.
    pub fn roots(&self) -> Vec<&Task> {
        let mut roots: Vec<&Task> = self
            .tasks
            .values()
            .filter(|t| {
                t.parent
                    .as_ref()
                    .is_none_or(|p| !self.tasks.contains_key(p))
            })
            .collect();
        roots.sort_by(|a, b| {
            (a.order_key(), a.created)
                .partial_cmp(&(b.order_key(), b.created))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        roots
    }

    /// Would setting `id`'s parent to `new_parent` create a cycle?
    pub fn would_cycle(&self, id: &TaskId, new_parent: &TaskId) -> bool {
        if id == new_parent {
            return true;
        }
        self.ancestors(new_parent).iter().any(|a| &a.id == id)
    }

    /// Resolve a full id or unique prefix.
    pub fn resolve(&self, prefix: &str) -> Resolution {
        if let Ok(id) = prefix.parse::<TaskId>()
            && self.tasks.contains_key(&id)
        {
            return Resolution::One(id);
        }
        let matches: Vec<TaskId> = self
            .tasks
            .keys()
            .filter(|k| k.as_str().starts_with(prefix))
            .cloned()
            .collect();
        match matches.len() {
            0 => Resolution::None,
            1 => Resolution::One(matches.into_iter().next().unwrap()),
            _ => Resolution::Ambiguous(matches),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    None,
    One(TaskId),
    Ambiguous(Vec<TaskId>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Context;
    use jiff::Timestamp;

    fn t(id: &str, parent: Option<&str>, secs: i64) -> Task {
        let mut task = Task::new(
            id.parse().unwrap(),
            id,
            Context::Work,
            Timestamp::from_second(secs).unwrap(),
        );
        task.parent = parent.map(|p| p.parse().unwrap());
        task
    }

    #[test]
    fn ancestry_and_children() {
        let tree = Tree::from_tasks([
            t("r00t00", None, 0),
            t("chd000", Some("r00t00"), 1),
            t("grand0", Some("chd000"), 2),
        ]);
        let g: TaskId = "grand0".parse().unwrap();
        let anc: Vec<&str> = tree.ancestors(&g).iter().map(|a| a.id.as_str()).collect();
        assert_eq!(anc, ["chd000", "r00t00"]);
        assert_eq!(tree.root_of(&g).unwrap().id.as_str(), "r00t00");
        assert_eq!(tree.descendants(&"r00t00".parse().unwrap()).len(), 2);
        assert_eq!(tree.roots().len(), 1);
    }

    #[test]
    fn reparent_updates_index() {
        let mut tree = Tree::from_tasks([
            t("a00000", None, 0),
            t("b00000", None, 1),
            t("c00000", Some("a00000"), 2),
        ]);
        let mut c = tree.get(&"c00000".parse().unwrap()).unwrap().clone();
        c.parent = Some("b00000".parse().unwrap());
        tree.insert(c);
        assert_eq!(tree.children(&"a00000".parse().unwrap()).count(), 0);
        assert_eq!(tree.children(&"b00000".parse().unwrap()).count(), 1);
        assert!(tree.would_cycle(&"b00000".parse().unwrap(), &"c00000".parse().unwrap()));
    }

    #[test]
    fn resolve_prefixes() {
        let tree = Tree::from_tasks([t("abc123", None, 0), t("abd456", None, 1)]);
        assert_eq!(
            tree.resolve("abc"),
            Resolution::One("abc123".parse().unwrap())
        );
        assert!(matches!(tree.resolve("ab"), Resolution::Ambiguous(v) if v.len() == 2));
        assert_eq!(tree.resolve("zzz"), Resolution::None);
    }
}
