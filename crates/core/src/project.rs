//! Projects: a free-text label on a *root* task that every task under it
//! inherits, so several roots can share one ("devops") and each name gets a
//! colour of its own. The colour is chosen when the name first appears (the
//! first palette entry no other project uses) and stored on the root, so it
//! never shifts when other projects come and go.

use serde::{Deserialize, Serialize};

use crate::model::{Task, TaskId};
use crate::tree::Tree;

/// Distinct on a dark background; ordered so neighbours differ the most.
pub const PALETTE: [(&str, &str); 10] = [
    ("orange", "#f97316"),
    ("blue", "#3b82f6"),
    ("green", "#22c55e"),
    ("pink", "#ec4899"),
    ("cyan", "#06b6d4"),
    ("purple", "#a855f7"),
    ("yellow", "#eab308"),
    ("red", "#ef4444"),
    ("teal", "#14b8a6"),
    ("lime", "#84cc16"),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTag {
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
}

impl ProjectTag {
    pub fn rgb(&self) -> (u8, u8, u8) {
        rgb(&self.color)
    }
}

/// `#rrggbb` to its parts; anything malformed is grey.
pub fn rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    match (p(0), p(2), p(4)) {
        (Some(r), Some(g), Some(b)) => (r, g, b),
        _ => (128, 128, 128),
    }
}

fn fnv(name: &str) -> usize {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.to_lowercase().bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h as usize
}

/// The root's swatch: the stored one, else a stable pick from the name (a
/// hand-written note with a project and no swatch still gets a colour).
fn swatch_of(root: &Task, name: &str) -> usize {
    root.swatch.map(usize::from).unwrap_or_else(|| fnv(name)) % PALETTE.len()
}

/// The project of any task: its root's.
pub fn tag_of(tree: &Tree, id: &TaskId) -> Option<ProjectTag> {
    let root = tree.root_of(id)?;
    let name = root.project.as_deref()?;
    Some(ProjectTag {
        name: name.to_string(),
        color: PALETTE[swatch_of(root, name)].1.to_string(),
    })
}

/// Every project in use, once each, by name.
pub fn all(tree: &Tree) -> Vec<ProjectTag> {
    let mut out: Vec<ProjectTag> = Vec::new();
    for root in tree.roots() {
        if let Some(tag) = tag_of(tree, &root.id)
            && !out.iter().any(|t| t.name.eq_ignore_ascii_case(&tag.name))
        {
            out.push(tag);
        }
    }
    out.sort_by_key(|t| t.name.to_lowercase());
    out
}

/// Resolve a requested name against the existing projects: the same name in
/// any case reuses its spelling and colour; a new name takes the first colour
/// nobody uses (the least used one once the palette is exhausted).
pub fn assign(tree: &Tree, requested: &str, excluding: &TaskId) -> (String, u8) {
    let requested = requested.trim();
    let mut used = [0usize; PALETTE.len()];
    for root in tree.roots() {
        if &root.id == excluding {
            continue;
        }
        if let Some(name) = root.project.as_deref() {
            let s = swatch_of(root, name);
            if name.eq_ignore_ascii_case(requested) {
                return (name.to_string(), s as u8);
            }
            used[s] += 1;
        }
    }
    let least = used.iter().copied().min().unwrap_or(0);
    let pick = used.iter().position(|&n| n == least).unwrap_or(0);
    (requested.to_string(), pick as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Context;

    fn root(id: &str, project: Option<(&str, Option<u8>)>) -> Task {
        let now = "2026-09-29T15:00:00[UTC]"
            .parse::<jiff::Zoned>()
            .unwrap()
            .timestamp();
        let mut t = Task::new(id.parse().unwrap(), id, Context::Work, now);
        if let Some((p, s)) = project {
            t.project = Some(p.into());
            t.swatch = s;
        }
        t
    }

    #[test]
    fn children_inherit_and_roots_share_one_colour() {
        let mut child = root("bbbbb2", None);
        child.parent = Some("aaaaa1".parse().unwrap());
        let tree = Tree::from_tasks([
            root("aaaaa1", Some(("devops", Some(3)))),
            child,
            root("ccccc3", Some(("DevOps", Some(3)))),
        ]);
        let tag = tag_of(&tree, &"bbbbb2".parse().unwrap()).unwrap();
        assert_eq!(tag.name, "devops");
        assert_eq!(tag.color, PALETTE[3].1);
        assert_eq!(all(&tree).len(), 1);
    }

    #[test]
    fn a_new_name_gets_an_unused_colour_and_a_known_one_keeps_its_own() {
        let tree = Tree::from_tasks([
            root("aaaaa1", Some(("devops", Some(0)))),
            root("bbbbb2", Some(("taskstaka", Some(1)))),
            root("ccccc3", None),
        ]);
        let new = &"ccccc3".parse().unwrap();
        assert_eq!(assign(&tree, "web", new), ("web".to_string(), 2));
        assert_eq!(assign(&tree, "DEVOPS", new), ("devops".to_string(), 0));
    }

    #[test]
    fn a_hand_written_project_without_a_swatch_still_has_a_stable_colour() {
        let tree = Tree::from_tasks([root("aaaaa1", Some(("x", None)))]);
        let id = "aaaaa1".parse().unwrap();
        assert_eq!(tag_of(&tree, &id), tag_of(&tree, &id));
        assert!(tag_of(&tree, &id).is_some());
    }

    #[test]
    fn hex_parses_and_garbage_is_grey() {
        assert_eq!(rgb("#f97316"), (0xf9, 0x73, 0x16));
        assert_eq!(rgb("nope"), (128, 128, 128));
    }
}
