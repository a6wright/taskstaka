//! The markdown-backed store. Holds the whole tree in memory, writes through
//! to `notes/<id>-<slug>.md` atomically, and regenerates `index.md` after
//! every mutation. One `Store` per directory, enforced with an advisory lock.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jiff::Zoned;
use thiserror::Error;

use crate::command::{AddTask, Command, EditTask, Event, Outcome};
use crate::frontmatter::{self, FrontmatterError};
use crate::index_md;
use crate::model::{Status, Task, TaskId};
use crate::tree::{Resolution, Tree};

pub const NOTES_DIR: &str = "notes";
pub const INDEX_FILE: &str = "index.md";
pub const META_DIR: &str = ".tododo";
pub const LOCK_FILE: &str = "lock";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: FrontmatterError,
    },
    #[error("{0} is not a tododo directory (run `tododo init`)")]
    NotInitialized(PathBuf),
    #[error("{0} is already initialized")]
    AlreadyInitialized(PathBuf),
    #[error("another tododo process holds the lock on {0}")]
    Locked(PathBuf),
    #[error("duplicate id {id} in {a} and {b}")]
    DuplicateId { id: TaskId, a: PathBuf, b: PathBuf },
    #[error("no task matches {0:?}")]
    NotFound(String),
    #[error("{prefix:?} is ambiguous: {}", candidates.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", "))]
    Ambiguous {
        prefix: String,
        candidates: Vec<TaskId>,
    },
    #[error("cannot move {id} from {from} to {to}: {reason}")]
    InvalidTransition {
        id: TaskId,
        from: Status,
        to: Status,
        reason: String,
    },
    #[error("{id} has {count} open descendant(s); close them first or use --cascade")]
    HasOpenDescendants { id: TaskId, count: usize },
    #[error("{id} has {count} descendant(s); use --force to delete them too")]
    HasDescendants { id: TaskId, count: usize },
    #[error("moving {id} under {parent} would create a cycle")]
    Cycle { id: TaskId, parent: TaskId },
    #[error("{0}")]
    Validation(String),
}

type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    mtime: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    fn of(path: &Path) -> Result<Stamp> {
        let md = fs::metadata(path).map_err(|e| io_err(path, e))?;
        Ok(Stamp {
            path: path.to_path_buf(),
            mtime: md.modified().ok(),
            len: md.len(),
        })
    }
}

pub struct Store {
    dir: PathBuf,
    tree: Tree,
    stamps: HashMap<TaskId, Stamp>,
    /// Held for the lifetime of the store; the OS releases it on drop.
    _lock: File,
    /// Non-fatal problems found at load (e.g. dangling parents).
    pub load_warnings: Vec<String>,
}

fn io_err(path: &Path, source: std::io::Error) -> StoreError {
    StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl Store {
    /// Create the directory layout. Fails if already initialized.
    pub fn init(dir: &Path) -> Result<Store> {
        if dir.join(META_DIR).exists() {
            return Err(StoreError::AlreadyInitialized(dir.to_path_buf()));
        }
        for sub in [NOTES_DIR, META_DIR] {
            let p = dir.join(sub);
            fs::create_dir_all(&p).map_err(|e| io_err(&p, e))?;
        }
        Self::open(dir)
    }

    /// Open an initialized directory: take the lock, load every note, and
    /// regenerate `index.md` (which covers a crash between a note write and
    /// the index write).
    pub fn open(dir: &Path) -> Result<Store> {
        let dir = dir.to_path_buf();
        if !dir.join(META_DIR).is_dir() || !dir.join(NOTES_DIR).is_dir() {
            return Err(StoreError::NotInitialized(dir));
        }
        let lock_path = dir.join(META_DIR).join(LOCK_FILE);
        let lock = File::create(&lock_path).map_err(|e| io_err(&lock_path, e))?;
        if lock.try_lock().is_err() {
            return Err(StoreError::Locked(lock_path));
        }
        let mut store = Store {
            dir,
            tree: Tree::new(),
            stamps: HashMap::new(),
            _lock: lock,
            load_warnings: Vec::new(),
        };
        store.reload()?;
        store.reindex(&Zoned::now())?;
        Ok(store)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn notes_dir(&self) -> PathBuf {
        self.dir.join(NOTES_DIR)
    }

    pub fn path_of(&self, id: &TaskId) -> Option<&Path> {
        self.stamps.get(id).map(|s| s.path.as_path())
    }

    /// Drop memory and re-read every note from disk.
    pub fn reload(&mut self) -> Result<()> {
        let notes = self.notes_dir();
        let mut tree = Tree::new();
        let mut stamps: HashMap<TaskId, Stamp> = HashMap::new();
        let mut entries: Vec<PathBuf> = fs::read_dir(&notes)
            .map_err(|e| io_err(&notes, e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|x| x == "md")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| !n.starts_with('.'))
            })
            .collect();
        entries.sort();
        for path in entries {
            let text = fs::read_to_string(&path).map_err(|e| io_err(&path, e))?;
            let task = frontmatter::parse(&text).map_err(|source| StoreError::Parse {
                path: path.clone(),
                source,
            })?;
            if let Some(prev) = stamps.get(&task.id) {
                return Err(StoreError::DuplicateId {
                    id: task.id,
                    a: prev.path.clone(),
                    b: path,
                });
            }
            stamps.insert(task.id.clone(), Stamp::of(&path)?);
            tree.insert(task);
        }
        let mut warnings = Vec::new();
        for t in tree.iter() {
            if let Some(p) = &t.parent
                && !tree.contains(p)
            {
                warnings.push(format!(
                    "{}: parent {p} does not exist; treating as a root",
                    t.id
                ));
            }
        }
        self.tree = tree;
        self.stamps = stamps;
        self.load_warnings = warnings;
        Ok(())
    }

    pub fn reindex(&self, now: &Zoned) -> Result<()> {
        let text = index_md::render(&self.tree, now);
        atomic_write(&self.dir.join(INDEX_FILE), text.as_bytes())
    }

    pub fn resolve(&self, prefix: &str) -> Result<TaskId> {
        match self.tree.resolve(prefix) {
            Resolution::One(id) => Ok(id),
            Resolution::None => Err(StoreError::NotFound(prefix.to_string())),
            Resolution::Ambiguous(ids) => Err(StoreError::Ambiguous {
                prefix: prefix.to_string(),
                candidates: ids,
            }),
        }
    }

    fn get(&self, id: &TaskId) -> Result<&Task> {
        self.tree
            .get(id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// If the file behind `id` changed since we loaded it, re-read it so the
    /// command applies on top of the external edit instead of clobbering it.
    fn refresh_if_changed(&mut self, id: &TaskId, outcome: &mut Outcome) -> Result<()> {
        let Some(stamp) = self.stamps.get(id) else {
            return Ok(());
        };
        if !stamp.path.exists() {
            return Ok(()); // deleted externally; our write will recreate it
        }
        let current = Stamp::of(&stamp.path)?;
        if current == *stamp {
            return Ok(());
        }
        let text = fs::read_to_string(&stamp.path).map_err(|e| io_err(&stamp.path, e))?;
        let mut task = frontmatter::parse(&text).map_err(|source| StoreError::Parse {
            path: stamp.path.clone(),
            source,
        })?;
        task.id = id.clone(); // the filename decides which task this is
        self.tree.insert(task);
        self.stamps.insert(id.clone(), current);
        outcome
            .events
            .push(Event::ReloadedFromDisk { id: id.clone() });
        outcome.warnings.push(format!(
            "{id} had changed on disk; reloaded before applying"
        ));
        Ok(())
    }

    fn fresh_id(&self) -> TaskId {
        loop {
            let id = TaskId::random();
            if !self.tree.contains(&id) {
                return id;
            }
        }
    }

    /// Apply one command. Files are written before memory is updated, so
    /// memory never claims a state that disk does not have.
    pub fn apply(&mut self, cmd: Command, now: &Zoned) -> Result<Outcome> {
        let mut outcome = Outcome::default();
        // Whole seconds: readable frontmatter, stable diffs.
        let ts = now
            .timestamp()
            .round(jiff::Unit::Second)
            .unwrap_or_else(|_| now.timestamp());

        // Step 1: external-edit guard for the target.
        if let Some(id) = cmd_target(&cmd) {
            self.refresh_if_changed(&id, &mut outcome)?;
        }

        // Step 2: validate and compute new values.
        let mut writes: Vec<Task> = Vec::new();
        let mut deletes: Vec<TaskId> = Vec::new();

        match cmd {
            Command::Add(add) => {
                let task = self.plan_add(add, ts)?;
                outcome.primary = Some(task.id.clone());
                outcome.events.push(Event::Created {
                    id: task.id.clone(),
                });
                writes.push(task);
            }
            Command::SetStatus {
                id,
                status,
                waiting_on,
                until,
                cascade,
            } => {
                outcome.primary = Some(id.clone());
                self.plan_set_status(
                    &id,
                    status,
                    waiting_on,
                    until,
                    cascade,
                    ts,
                    &mut writes,
                    &mut outcome,
                )?;
            }
            Command::Edit(edit) => {
                outcome.primary = Some(edit.id.clone());
                writes.push(self.plan_edit(edit, ts)?);
            }
            Command::Move { id, parent } => {
                outcome.primary = Some(id.clone());
                writes.push(self.plan_move(&id, parent)?);
            }
            Command::Touch { id, note } => {
                outcome.primary = Some(id.clone());
                let mut t = self.get(&id)?.clone();
                t.updated = ts;
                if let Some(note) = note.filter(|n| !n.trim().is_empty()) {
                    if !t.body.is_empty() && !t.body.ends_with('\n') {
                        t.body.push('\n');
                    }
                    t.body.push_str(&format!(
                        "- {} {}\n",
                        now.strftime("%Y-%m-%d %H:%M"),
                        note.trim()
                    ));
                }
                writes.push(t);
            }
            Command::Delete { id, force } => {
                outcome.primary = Some(id.clone());
                self.get(&id)?;
                let desc = self.tree.descendants(&id);
                if !desc.is_empty() && !force {
                    return Err(StoreError::HasDescendants {
                        id,
                        count: desc.len(),
                    });
                }
                deletes.extend(desc.iter().map(|t| t.id.clone()));
                deletes.push(id);
            }
        }

        // Step 3: disk.
        for t in &writes {
            let path = self.path_for_write(t);
            atomic_write(&path, frontmatter::render(t).as_bytes())?;
            self.stamps.insert(t.id.clone(), Stamp::of(&path)?);
        }
        for id in &deletes {
            if let Some(stamp) = self.stamps.remove(id)
                && stamp.path.exists()
            {
                fs::remove_file(&stamp.path).map_err(|e| io_err(&stamp.path, e))?;
            }
        }

        // Step 4: memory.
        for t in writes {
            outcome.affected.push(t.id.clone());
            self.tree.insert(t);
        }
        for id in deletes {
            self.tree.remove(&id);
            outcome.events.push(Event::Deleted { id: id.clone() });
            outcome.affected.push(id);
        }

        // Step 5: derived artefacts.
        self.reindex(now)?;
        Ok(outcome)
    }

    fn path_for_write(&self, t: &Task) -> PathBuf {
        match self.stamps.get(&t.id) {
            Some(s) => s.path.clone(), // slug frozen at creation
            None => self.notes_dir().join(format!("{}.md", t.file_stem())),
        }
    }

    fn plan_add(&self, add: AddTask, ts: jiff::Timestamp) -> Result<Task> {
        let title = add.title.trim();
        if title.is_empty() {
            return Err(StoreError::Validation("title must not be empty".into()));
        }
        let parent = match &add.parent {
            Some(p) => {
                let parent = self.get(p)?;
                if parent.status.is_closed() {
                    return Err(StoreError::Validation(format!(
                        "parent {p} is {}; reopen it first",
                        parent.status
                    )));
                }
                Some(parent)
            }
            None => None,
        };
        let context = add
            .context
            .or_else(|| {
                parent
                    .and_then(|p| self.tree.root_of(&p.id))
                    .map(|r| r.context)
            })
            .unwrap_or_default();
        let mut task = Task::new(self.fresh_id(), title, context, ts);
        task.parent = add.parent;
        task.priority = add.priority;
        task.due = add.due;
        task.tags = add.tags;
        task.body = add.body;
        if let Some(status) = add.status {
            if status.is_closed() {
                return Err(StoreError::Validation(
                    "cannot create a task already closed".into(),
                ));
            }
            if status == Status::Waiting {
                return Err(StoreError::Validation(
                    "create the task first, then `wait` it with a reason".into(),
                ));
            }
            task.status = status;
            if status == Status::Doing {
                task.started = Some(ts);
            }
        }
        Ok(task)
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_set_status(
        &self,
        id: &TaskId,
        to: Status,
        waiting_on: Option<String>,
        until: Option<jiff::civil::Date>,
        cascade: bool,
        ts: jiff::Timestamp,
        writes: &mut Vec<Task>,
        outcome: &mut Outcome,
    ) -> Result<()> {
        let mut t = self.get(id)?.clone();
        let from = t.status;

        if to == Status::Waiting {
            let who = waiting_on
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(who) = who else {
                return Err(StoreError::Validation(
                    "waiting needs a reason: what are you waiting on?".into(),
                ));
            };
            t.waiting_on = Some(who.to_string());
            t.until = until;
            let open_kids = self
                .tree
                .children(id)
                .filter(|c| c.status.is_blocking())
                .count();
            if open_kids > 0 {
                outcome.warnings.push(format!(
                    "{id} has {open_kids} open subtask(s); consider marking the blocked subtask as waiting instead (block at the leaf)"
                ));
            }
        } else {
            t.waiting_on = None;
            t.until = None;
        }

        if to.is_closed() {
            let open: Vec<&Task> = self
                .tree
                .descendants(id)
                .into_iter()
                .filter(|d| d.status.is_open())
                .collect();
            if !open.is_empty() {
                if !cascade {
                    return Err(StoreError::HasOpenDescendants {
                        id: id.clone(),
                        count: open.len(),
                    });
                }
                for d in open {
                    let mut d = d.clone();
                    d.status = Status::Dropped;
                    d.completed = Some(ts);
                    d.updated = ts;
                    d.waiting_on = None;
                    d.until = None;
                    outcome
                        .events
                        .push(Event::CascadeDropped { id: d.id.clone() });
                    writes.push(d);
                }
            }
            t.completed = Some(ts);
        } else {
            t.completed = None;
        }

        if to == Status::Doing && t.started.is_none() {
            t.started = Some(ts);
        }

        // I4: reopening under a closed parent reopens the ancestors.
        if from.is_closed() && to.is_open() {
            for a in self.tree.ancestors(id) {
                if a.status.is_closed() {
                    let mut a = a.clone();
                    a.status = Status::Todo;
                    a.completed = None;
                    // Derived change: do not bump `updated`.
                    outcome
                        .events
                        .push(Event::AutoReopened { id: a.id.clone() });
                    writes.push(a);
                }
            }
        }

        // Resurfacing: did we just close the parent's last blocking child?
        if to.is_closed()
            && from.is_blocking()
            && let Some(pid) = &t.parent
            && let Some(parent) = self.tree.get(pid)
            && parent.status.is_open()
        {
            let others_blocking = self
                .tree
                .children(pid)
                .any(|c| c.id != *id && c.status.is_blocking());
            if !others_blocking {
                outcome.events.push(Event::Resurfaced { id: pid.clone() });
            }
        }

        if from != to {
            outcome.events.push(Event::StatusChanged {
                id: id.clone(),
                from,
                to,
            });
        }
        t.status = to;
        t.updated = ts;
        writes.push(t);
        Ok(())
    }

    fn plan_edit(&self, edit: EditTask, ts: jiff::Timestamp) -> Result<Task> {
        let mut t = self.get(&edit.id)?.clone();
        let mut changed = false;
        if let Some(title) = edit.title {
            let title = title.trim();
            if title.is_empty() {
                return Err(StoreError::Validation("title must not be empty".into()));
            }
            changed |= t.title != title;
            t.title = title.to_string();
        }
        if let Some(ctx) = edit.context {
            changed |= t.context != ctx;
            t.context = ctx;
        }
        changed |= edit.priority.apply_to(&mut t.priority);
        changed |= edit.due.apply_to(&mut t.due);
        if let Some(tags) = edit.tags {
            changed |= t.tags != tags;
            t.tags = tags;
        }
        if let Some(body) = edit.body {
            changed |= t.body != body;
            t.body = body;
        }
        if changed {
            t.updated = ts;
        }
        Ok(t)
    }

    fn plan_move(&self, id: &TaskId, parent: Option<TaskId>) -> Result<Task> {
        let mut t = self.get(id)?.clone();
        if let Some(p) = &parent {
            let target = self.get(p)?;
            if target.status.is_closed() {
                return Err(StoreError::Validation(format!(
                    "parent {p} is {}; reopen it first",
                    target.status
                )));
            }
            if self.tree.would_cycle(id, p) {
                return Err(StoreError::Cycle {
                    id: id.clone(),
                    parent: p.clone(),
                });
            }
        }
        // I5: reparenting is structural, not a touch.
        t.parent = parent;
        Ok(t)
    }
}

fn cmd_target(cmd: &Command) -> Option<TaskId> {
    match cmd {
        Command::Add(_) => None,
        Command::SetStatus { id, .. }
        | Command::Move { id, .. }
        | Command::Touch { id, .. }
        | Command::Delete { id, .. } => Some(id.clone()),
        Command::Edit(e) => Some(e.id.clone()),
    }
}

/// Write via a temp file in the same directory, fsync, rename, fsync the
/// directory. Readers never see a partial file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io_err(path, std::io::Error::other("no parent directory")))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .suffix(".md")
        .tempfile_in(dir)
        .map_err(|e| io_err(dir, e))?;
    tmp.write_all(bytes).map_err(|e| io_err(path, e))?;
    tmp.as_file().sync_all().map_err(|e| io_err(path, e))?;
    tmp.persist(path).map_err(|e| io_err(path, e.error))?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Set;
    use crate::model::{Context, Priority};
    use jiff::ToSpan;

    fn now() -> Zoned {
        "2026-09-29T15:00:00[UTC]".parse().unwrap()
    }

    fn add(store: &mut Store, title: &str, parent: Option<&TaskId>) -> TaskId {
        store
            .apply(
                Command::Add(AddTask {
                    title: title.into(),
                    parent: parent.cloned(),
                    ..Default::default()
                }),
                &now(),
            )
            .unwrap()
            .primary
            .unwrap()
    }

    fn status(store: &mut Store, id: &TaskId, to: Status, at: &Zoned) -> Result<Outcome> {
        store.apply(
            Command::SetStatus {
                id: id.clone(),
                status: to,
                waiting_on: None,
                until: None,
                cascade: false,
            },
            at,
        )
    }

    #[test]
    fn init_open_and_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        assert!(dir.path().join("index.md").exists());
        assert!(matches!(
            Store::open(dir.path()),
            Err(StoreError::Locked(_))
        ));
        drop(store);
        assert!(Store::open(dir.path()).is_ok());
        assert!(matches!(
            Store::init(dir.path()),
            Err(StoreError::AlreadyInitialized(_))
        ));
    }

    #[test]
    fn add_writes_file_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let (story, child) = {
            let mut store = Store::init(dir.path()).unwrap();
            let story = add(&mut store, "Ship auth story", None);
            let child = add(&mut store, "Fix JWT expiry", Some(&story));
            let path = store.path_of(&child).unwrap().to_path_buf();
            assert!(
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .ends_with("-fix-jwt-expiry.md")
            );
            (story, child)
        };
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.tree().len(), 2);
        assert_eq!(
            store.tree().get(&child).unwrap().parent.as_ref(),
            Some(&story)
        );
        assert_eq!(store.tree().get(&child).unwrap().context, Context::Work);
    }

    #[test]
    fn close_refuses_open_descendants_unless_cascade() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let story = add(&mut store, "story", None);
        let kid = add(&mut store, "kid", Some(&story));
        let err = status(&mut store, &story, Status::Done, &now()).unwrap_err();
        assert!(matches!(
            err,
            StoreError::HasOpenDescendants { count: 1, .. }
        ));
        let out = store
            .apply(
                Command::SetStatus {
                    id: story.clone(),
                    status: Status::Done,
                    waiting_on: None,
                    until: None,
                    cascade: true,
                },
                &now(),
            )
            .unwrap();
        assert!(
            out.events
                .contains(&Event::CascadeDropped { id: kid.clone() })
        );
        assert_eq!(store.tree().get(&kid).unwrap().status, Status::Dropped);
    }

    #[test]
    fn closing_last_child_emits_resurfaced_and_reopen_cascades_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let story = add(&mut store, "story", None);
        let a = add(&mut store, "a", Some(&story));
        let b = add(&mut store, "b", Some(&story));
        let out = status(&mut store, &a, Status::Done, &now()).unwrap();
        assert!(
            !out.events
                .contains(&Event::Resurfaced { id: story.clone() }),
            "b still blocks"
        );
        let later = now() + 1.hour();
        let out = status(&mut store, &b, Status::Done, &later).unwrap();
        assert!(
            out.events
                .contains(&Event::Resurfaced { id: story.clone() })
        );

        // Parent done, then reopen a child: parent auto-reopens.
        status(&mut store, &story, Status::Done, &later).unwrap();
        let out = status(&mut store, &a, Status::Todo, &later).unwrap();
        assert!(
            out.events
                .contains(&Event::AutoReopened { id: story.clone() })
        );
        assert_eq!(store.tree().get(&story).unwrap().status, Status::Todo);
    }

    #[test]
    fn waiting_requires_reason_and_clears_on_leave() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let t = add(&mut store, "t", None);
        assert!(matches!(
            status(&mut store, &t, Status::Waiting, &now()),
            Err(StoreError::Validation(_))
        ));
        store
            .apply(
                Command::SetStatus {
                    id: t.clone(),
                    status: Status::Waiting,
                    waiting_on: Some("Sam".into()),
                    until: None,
                    cascade: false,
                },
                &now(),
            )
            .unwrap();
        assert_eq!(
            store.tree().get(&t).unwrap().waiting_on.as_deref(),
            Some("Sam")
        );
        status(&mut store, &t, Status::Todo, &now()).unwrap();
        assert_eq!(store.tree().get(&t).unwrap().waiting_on, None);
    }

    #[test]
    fn move_rejects_cycles_and_does_not_touch() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let a = add(&mut store, "a", None);
        let b = add(&mut store, "b", Some(&a));
        assert!(matches!(
            store.apply(
                Command::Move {
                    id: a.clone(),
                    parent: Some(b.clone())
                },
                &now()
            ),
            Err(StoreError::Cycle { .. })
        ));
        let before = store.tree().get(&b).unwrap().updated;
        store
            .apply(
                Command::Move {
                    id: b.clone(),
                    parent: None,
                },
                &(now() + 1.hour()),
            )
            .unwrap();
        assert_eq!(store.tree().get(&b).unwrap().updated, before);
        assert_eq!(store.tree().roots().len(), 2);
    }

    #[test]
    fn edit_and_touch_bump_updated_and_journal() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let t = add(&mut store, "t", None);
        let later = now() + 2.hours();
        store
            .apply(
                Command::Edit(EditTask {
                    priority: Set::To(Priority::High),
                    ..EditTask::new(t.clone())
                }),
                &later,
            )
            .unwrap();
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.priority, Some(Priority::High));
        assert_eq!(task.updated, later.timestamp());
        store
            .apply(
                Command::Touch {
                    id: t.clone(),
                    note: Some("pinged Sam".into()),
                },
                &later,
            )
            .unwrap();
        assert!(store.tree().get(&t).unwrap().body.contains("pinged Sam"));
    }

    #[test]
    fn external_edit_is_reloaded_not_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let t = add(&mut store, "t", None);
        let path = store.path_of(&t).unwrap().to_path_buf();
        // Simulate Obsidian appending notes to the body.
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("\nedited in obsidian\n");
        // Ensure the stamp differs even on coarse mtime filesystems: length changed.
        fs::write(&path, text).unwrap();
        let out = status(&mut store, &t, Status::Doing, &now()).unwrap();
        assert!(
            out.events
                .contains(&Event::ReloadedFromDisk { id: t.clone() })
        );
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.status, Status::Doing);
        assert!(task.body.contains("edited in obsidian"));
    }

    #[test]
    fn delete_leaf_only_unless_force() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let a = add(&mut store, "a", None);
        let b = add(&mut store, "b", Some(&a));
        assert!(matches!(
            store.apply(
                Command::Delete {
                    id: a.clone(),
                    force: false
                },
                &now()
            ),
            Err(StoreError::HasDescendants { .. })
        ));
        let path_b = store.path_of(&b).unwrap().to_path_buf();
        store
            .apply(
                Command::Delete {
                    id: a.clone(),
                    force: true,
                },
                &now(),
            )
            .unwrap();
        assert!(store.tree().is_empty());
        assert!(!path_b.exists());
    }

    #[test]
    fn load_fails_loud_on_corrupt_note() {
        let dir = tempfile::tempdir().unwrap();
        drop(Store::init(dir.path()).unwrap());
        fs::write(dir.path().join("notes/bad.md"), "not a note").unwrap();
        assert!(matches!(
            Store::open(dir.path()),
            Err(StoreError::Parse { .. })
        ));
    }
}
