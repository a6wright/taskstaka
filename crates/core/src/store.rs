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

use crate::command::{AddTask, Command, EditTask, Event, Outcome, Set};
use crate::frontmatter::{self, FrontmatterError};
use crate::index_md;
use crate::model::{Status, Task, TaskId};
use crate::tree::{Resolution, Tree};

pub const NOTES_DIR: &str = "notes";
pub const INDEX_FILE: &str = "index.md";
pub const META_DIR: &str = ".taskstaka";
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
    #[error("{0} is not a taskstaka directory (run `taskstaka init`)")]
    NotInitialized(PathBuf),
    #[error("{0} is already initialized")]
    AlreadyInitialized(PathBuf),
    #[error("another taskstaka process holds the lock on {0}")]
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
    /// Default follow-up window when a task enters `waiting` without `until`.
    pub follow_up_days: i64,
}

pub const DEFAULT_FOLLOW_UP_DAYS: i64 = 2;

/// The project was renamed from "tododo"; directories initialized under the
/// old name carry a `.tododo/` metadata folder. Rename it in place so the
/// notes keep working without a re-init. Only metadata lives there (lock,
/// daemon.json, nudge log), so this is safe at any time nothing is running.
pub const LEGACY_META_DIR: &str = ".tododo";

fn migrate_legacy_meta_dir(dir: &Path) -> Result<()> {
    let old = dir.join(LEGACY_META_DIR);
    let new = dir.join(META_DIR);
    if old.is_dir() && !new.exists() {
        fs::rename(&old, &new).map_err(|e| io_err(&old, e))?;
    }
    Ok(())
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
        migrate_legacy_meta_dir(dir)?;
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
        migrate_legacy_meta_dir(&dir)?;
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
            follow_up_days: DEFAULT_FOLLOW_UP_DAYS,
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
                let task = self.plan_add(add, now)?;
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
                    now,
                    &mut writes,
                    &mut outcome,
                )?;
            }
            Command::Edit(edit) => {
                outcome.primary = Some(edit.id.clone());
                writes.push(self.plan_edit(edit, now, &mut outcome)?);
            }
            Command::Move { id, parent, after } => {
                outcome.primary = Some(id.clone());
                writes.push(self.plan_move(&id, parent, after)?);
            }
            Command::Touch { id, note } => {
                outcome.primary = Some(id.clone());
                let mut t = self.get(&id)?.clone();
                t.updated = ts;
                if let Some(note) = note.filter(|n| !n.trim().is_empty()) {
                    journal(&mut t, now, None, note.trim());
                }
                writes.push(t);
            }
            Command::Chase {
                id,
                until,
                note,
                source,
                dedupe,
            } => {
                outcome.primary = Some(id.clone());
                let mut t = self.get(&id)?.clone();
                if t.status != Status::Waiting {
                    return Err(StoreError::InvalidTransition {
                        id: id.clone(),
                        from: t.status,
                        to: Status::Waiting,
                        reason: "only a waiting task can be chased".into(),
                    });
                }
                let person = t.waiting_on.clone().unwrap_or_else(|| "someone".into());
                // Idempotency: the same dedupe key within this wait is a no-op.
                let key_line = dedupe.as_ref().map(|k| format!("[chase:{k}]"));
                if let Some(k) = &key_line
                    && t.body.contains(k.as_str())
                {
                    outcome.events.push(Event::Chased {
                        id: id.clone(),
                        person,
                        count: t.chases.len(),
                        deduped: true,
                    });
                    return Ok(outcome);
                }
                t.chases.push(ts);
                t.updated = ts;
                let today = now.date();
                let n = t.chases.len() as i64;
                t.until = Some(match until {
                    Some(u) => u,
                    None => today
                        .checked_add(jiff::Span::new().days((self.follow_up_days - n).max(1)))
                        .unwrap_or(today),
                });
                let mut line = format!("chased {person}; follow up {}", t.until.unwrap_or(today));
                if let Some(n) = note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    line.push_str(": ");
                    line.push_str(n);
                }
                if let Some(k) = key_line {
                    line.push(' ');
                    line.push_str(&k);
                }
                journal(&mut t, now, source.as_deref(), &line);
                outcome.events.push(Event::Chased {
                    id: id.clone(),
                    person,
                    count: t.chases.len(),
                    deduped: false,
                });
                writes.push(t);
            }
            Command::RecordReply {
                id,
                outcome: reply,
                summary,
                waiting_ref,
                source,
            } => {
                outcome.primary = Some(id.clone());
                let mut t = self.get(&id)?.clone();
                if t.status != Status::Waiting {
                    return Err(StoreError::InvalidTransition {
                        id: id.clone(),
                        from: t.status,
                        to: Status::Todo,
                        reason: "only a waiting task can receive a reply".into(),
                    });
                }
                let summary = summary.trim();
                if summary.is_empty() {
                    return Err(StoreError::Validation(
                        "record_reply needs a one-line summary of what they said".into(),
                    ));
                }
                let person = t.waiting_on.clone().unwrap_or_else(|| "someone".into());
                if let Some(r) = waiting_ref {
                    t.waiting_ref = Some(r);
                }
                let today = now.date();
                use crate::command::ReplyOutcome as R;
                match &reply {
                    R::Answered => {
                        journal(
                            &mut t,
                            now,
                            source.as_deref(),
                            &format!("{person} replied: {summary}"),
                        );
                        t.clear_waiting();
                        t.status = Status::Todo;
                        outcome.events.push(Event::StatusChanged {
                            id: id.clone(),
                            from: Status::Waiting,
                            to: Status::Todo,
                        });
                    }
                    R::Declined => {
                        journal(
                            &mut t,
                            now,
                            source.as_deref(),
                            &format!("{person} declined: {summary}"),
                        );
                        t.clear_waiting();
                        t.status = Status::Todo;
                        outcome.events.push(Event::StatusChanged {
                            id: id.clone(),
                            from: Status::Waiting,
                            to: Status::Todo,
                        });
                    }
                    R::Redirected { to } => {
                        let to = to.trim().to_string();
                        if to.is_empty() {
                            return Err(StoreError::Validation("redirected: to whom?".into()));
                        }
                        journal(
                            &mut t,
                            now,
                            source.as_deref(),
                            &format!("{person} redirected to {to}: {summary}"),
                        );
                        outcome.events.push(Event::HandedOff {
                            id: id.clone(),
                            from: person.clone(),
                            to: to.clone(),
                        });
                        t.waiting_on = Some(to);
                        t.waiting_since = Some(ts);
                        t.chases.clear();
                        t.until = Some(self.default_until(today));
                    }
                    R::Partial => {
                        journal(
                            &mut t,
                            now,
                            source.as_deref(),
                            &format!("{person} partially: {summary}"),
                        );
                        t.until = Some(self.default_until(today));
                    }
                }
                t.updated = ts;
                outcome.events.push(Event::ReplyReceived {
                    id: id.clone(),
                    person,
                    outcome: reply,
                });
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

    fn plan_add(&self, add: AddTask, now: &Zoned) -> Result<Task> {
        let ts = now
            .timestamp()
            .round(jiff::Unit::Second)
            .unwrap_or_else(|_| now.timestamp());
        let today = now.date();
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
        task.order = Some(self.order_for(
            add.parent.as_ref(),
            add.after.as_ref(),
            None,
            ts.as_second() as f64,
        )?);
        task.parent = add.parent;
        task.priority = add.priority;
        task.due = add.due;
        task.tags = add.tags;
        task.body = add.body;
        if let Some(who) = add
            .waiting_on
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty())
        {
            task.status = Status::Waiting;
            task.waiting_on = Some(who.to_string());
            task.waiting_since = Some(ts);
            task.waiting_ref = add.waiting_ref.clone();
            let until = add.until.unwrap_or_else(|| self.default_until(today));
            task.until = Some(until);
            let line = format!("waiting on {who}; follow up {until}");
            journal(&mut task, now, None, &line);
            return Ok(task);
        }
        if let Some(status) = add.status {
            if status.is_closed() {
                return Err(StoreError::Validation(
                    "cannot create a task already closed".into(),
                ));
            }
            if status == Status::Waiting {
                return Err(StoreError::Validation(
                    "use waiting_on to create a task that is waiting on someone".into(),
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
        now: &Zoned,
        writes: &mut Vec<Task>,
        outcome: &mut Outcome,
    ) -> Result<()> {
        let ts = now
            .timestamp()
            .round(jiff::Unit::Second)
            .unwrap_or_else(|_| now.timestamp());
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
            let today = now.date();
            if from != Status::Waiting || t.waiting_on.as_deref() != Some(who) {
                // A new wait (or a handoff via status): fresh clock and count.
                if let (Status::Waiting, Some(prev)) = (from, t.waiting_on.as_deref())
                    && prev != who
                {
                    outcome.events.push(Event::HandedOff {
                        id: id.clone(),
                        from: prev.to_string(),
                        to: who.to_string(),
                    });
                }
                t.waiting_since = Some(ts);
                t.chases.clear();
            }
            t.waiting_on = Some(who.to_string());
            // Invariant: a waiting task always has a follow-up date.
            let until = until.unwrap_or_else(|| self.default_until(today));
            t.until = Some(until);
            let line = format!("waiting on {who}; follow up {until}");
            journal(&mut t, now, None, &line);
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
        } else if from == Status::Waiting {
            t.clear_waiting();
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
                    d.status = to;
                    d.completed = Some(ts);
                    d.updated = ts;
                    d.waiting_on = None;
                    d.until = None;
                    outcome.events.push(Event::CascadeClosed {
                        id: d.id.clone(),
                        status: to,
                    });
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

        // Reopening under a closed parent does NOT reopen the parent: the
        // task becomes "orphaned" (derived) and stays actionable on its own.

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

    /// `today + follow_up_days`, the standard follow-up horizon.
    fn default_until(&self, today: jiff::civil::Date) -> jiff::civil::Date {
        today
            .checked_add(jiff::Span::new().days(self.follow_up_days.max(0)))
            .unwrap_or(today)
    }

    fn plan_edit(&self, edit: EditTask, now: &Zoned, outcome: &mut Outcome) -> Result<Task> {
        let ts = now
            .timestamp()
            .round(jiff::Unit::Second)
            .unwrap_or_else(|_| now.timestamp());
        let mut t = self.get(&edit.id)?.clone();
        let mut changed = false;
        match edit.waiting_on {
            Set::Keep => {}
            Set::Clear => {
                return Err(StoreError::Validation(
                    "to stop waiting, change the status (reopen/done) instead".into(),
                ));
            }
            Set::To(to) => {
                let to = to.trim().to_string();
                if t.status != Status::Waiting {
                    return Err(StoreError::InvalidTransition {
                        id: t.id.clone(),
                        from: t.status,
                        to: Status::Waiting,
                        reason: "not waiting; use `wait` to start waiting on someone".into(),
                    });
                }
                if to.is_empty() {
                    return Err(StoreError::Validation("waiting on whom?".into()));
                }
                let from = t.waiting_on.clone().unwrap_or_default();
                if from != to {
                    journal(&mut t, now, None, &format!("handed off: {from} → {to}"));
                    outcome.events.push(Event::HandedOff {
                        id: t.id.clone(),
                        from,
                        to: to.clone(),
                    });
                    t.waiting_on = Some(to);
                    t.waiting_since = Some(ts);
                    t.chases.clear();
                    t.until = Some(self.default_until(now.date()));
                    changed = true;
                }
            }
        }
        changed |= edit.waiting_ref.apply_to(&mut t.waiting_ref);
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

    fn plan_move(
        &self,
        id: &TaskId,
        parent: Option<TaskId>,
        after: Option<TaskId>,
    ) -> Result<Task> {
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
        t.order = Some(self.order_for(parent.as_ref(), after.as_ref(), Some(id), t.order_key())?);
        t.parent = parent;
        Ok(t)
    }

    /// Fractional position among the siblings under `parent`: after `after`
    /// (which must be a sibling), or at the end. `exclude` is the task being
    /// moved, so it does not count as its own neighbour.
    fn order_for(
        &self,
        parent: Option<&TaskId>,
        after: Option<&TaskId>,
        exclude: Option<&TaskId>,
        default: f64,
    ) -> Result<f64> {
        let siblings: Vec<&Task> = match parent {
            Some(p) => self.tree.children(p).collect(),
            None => self.tree.roots(),
        };
        let siblings: Vec<&Task> = siblings
            .into_iter()
            .filter(|s| exclude.is_none_or(|x| s.id != *x))
            .collect();
        match after {
            None => Ok(siblings
                .last()
                .map(|s| s.order_key() + 1.0)
                .unwrap_or(default)),
            Some(a) => {
                let idx = siblings.iter().position(|s| s.id == *a).ok_or_else(|| {
                    StoreError::Validation(format!("{a} is not a sibling under the target parent"))
                })?;
                let prev = siblings[idx].order_key();
                Ok(match siblings.get(idx + 1) {
                    Some(next) => (prev + next.order_key()) / 2.0,
                    None => prev + 1.0,
                })
            }
        }
    }
}

fn cmd_target(cmd: &Command) -> Option<TaskId> {
    match cmd {
        Command::Add(_) => None,
        Command::SetStatus { id, .. }
        | Command::Move { id, .. }
        | Command::Touch { id, .. }
        | Command::Chase { id, .. }
        | Command::RecordReply { id, .. }
        | Command::Delete { id, .. } => Some(id.clone()),
        Command::Edit(e) => Some(e.id.clone()),
    }
}

/// Append a dated line to the body. Lines from non-human sources are tagged,
/// so the history says who did what.
fn journal(t: &mut Task, now: &Zoned, source: Option<&str>, line: &str) {
    if !t.body.is_empty() && !t.body.ends_with('\n') {
        t.body.push('\n');
    }
    let tag = match source {
        Some(s) if !s.is_empty() => format!(" [{s}]"),
        _ => String::new(),
    };
    t.body.push_str(&format!(
        "- {}{tag} {line}\n",
        now.strftime("%Y-%m-%d %H:%M")
    ));
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
        assert!(out.events.contains(&Event::CascadeClosed {
            id: kid.clone(),
            status: Status::Done
        }));
        assert_eq!(store.tree().get(&kid).unwrap().status, Status::Done);
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

        // Parent done, then reopen a child: the parent stays done and the
        // child is orphaned (see order_tests for the derived facts).
        status(&mut store, &story, Status::Done, &later).unwrap();
        status(&mut store, &a, Status::Todo, &later).unwrap();
        assert_eq!(store.tree().get(&story).unwrap().status, Status::Done);
        assert_eq!(store.tree().get(&a).unwrap().status, Status::Todo);
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
                    parent: Some(b.clone()),
                    after: None,
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
                    after: None,
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

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::command::AddTask;

    fn now() -> Zoned {
        "2026-09-29T15:00:00[UTC]".parse().unwrap()
    }

    fn add_after(
        store: &mut Store,
        title: &str,
        parent: Option<&TaskId>,
        after: Option<&TaskId>,
    ) -> TaskId {
        store
            .apply(
                Command::Add(AddTask {
                    title: title.into(),
                    parent: parent.cloned(),
                    after: after.cloned(),
                    ..Default::default()
                }),
                &now(),
            )
            .unwrap()
            .primary
            .unwrap()
    }

    fn titles(store: &Store, parent: Option<&TaskId>) -> Vec<String> {
        match parent {
            Some(p) => store.tree().children(p).map(|t| t.title.clone()).collect(),
            None => store
                .tree()
                .roots()
                .iter()
                .map(|t| t.title.clone())
                .collect(),
        }
    }

    #[test]
    fn insert_after_and_move_keep_outline_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let a = add_after(&mut store, "a", None, None);
        let c = add_after(&mut store, "c", None, None);
        // Enter after `a` inserts between a and c (same creation second).
        let b = add_after(&mut store, "b", None, Some(&a));
        assert_eq!(titles(&store, None), ["a", "b", "c"]);

        // Tab: b becomes a's subtask (appended). Shift+Tab: back out after a.
        store
            .apply(
                Command::Move {
                    id: b.clone(),
                    parent: Some(a.clone()),
                    after: None,
                },
                &now(),
            )
            .unwrap();
        assert_eq!(titles(&store, None), ["a", "c"]);
        assert_eq!(titles(&store, Some(&a)), ["b"]);
        store
            .apply(
                Command::Move {
                    id: b.clone(),
                    parent: None,
                    after: Some(a.clone()),
                },
                &now(),
            )
            .unwrap();
        assert_eq!(titles(&store, None), ["a", "b", "c"]);

        // Move to the end, then after a non-sibling is refused.
        store
            .apply(
                Command::Move {
                    id: a.clone(),
                    parent: None,
                    after: Some(c.clone()),
                },
                &now(),
            )
            .unwrap();
        assert_eq!(titles(&store, None), ["b", "c", "a"]);
        let err = store
            .apply(
                Command::Move {
                    id: c.clone(),
                    parent: Some(a.clone()),
                    after: Some(b.clone()),
                },
                &now(),
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::Validation(_)), "{err}");

        // Order survives a reload (it is in the frontmatter).
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(titles(&store, None), ["b", "c", "a"]);
    }

    #[test]
    fn reopening_a_child_does_not_reopen_a_done_parent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let story = add_after(&mut store, "story", None, None);
        let kid = add_after(&mut store, "kid", Some(&story), None);
        store
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
        assert_eq!(store.tree().get(&kid).unwrap().status, Status::Done);
        let out = store
            .apply(
                Command::SetStatus {
                    id: kid.clone(),
                    status: Status::Todo,
                    waiting_on: None,
                    until: None,
                    cascade: false,
                },
                &now(),
            )
            .unwrap();
        assert!(
            out.events
                .iter()
                .all(|e| !matches!(e, Event::Resurfaced { .. }))
        );
        assert_eq!(
            store.tree().get(&story).unwrap().status,
            Status::Done,
            "parent stays done"
        );
        let d = crate::action::derive(store.tree(), store.tree().get(&kid).unwrap(), now().date());
        assert!(d.orphaned && d.actionable);
    }
}

#[cfg(test)]
mod waiting_tests {
    use super::*;
    use crate::command::{AddTask, ReplyOutcome, Set};
    use jiff::ToSpan;
    use jiff::civil::date;

    fn now() -> Zoned {
        "2026-09-29T15:00:00[UTC]".parse().unwrap()
    }

    fn add(store: &mut Store, title: &str) -> TaskId {
        store
            .apply(
                Command::Add(AddTask {
                    title: title.into(),
                    ..Default::default()
                }),
                &now(),
            )
            .unwrap()
            .primary
            .unwrap()
    }

    fn wait(
        store: &mut Store,
        id: &TaskId,
        who: &str,
        until: Option<jiff::civil::Date>,
    ) -> Outcome {
        store
            .apply(
                Command::SetStatus {
                    id: id.clone(),
                    status: Status::Waiting,
                    waiting_on: Some(who.into()),
                    until,
                    cascade: false,
                },
                &now(),
            )
            .unwrap()
    }

    #[test]
    fn waiting_always_has_a_follow_up_and_a_clock() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let t = add(&mut store, "DNS: what's possible?");
        wait(&mut store, &t, "A", None);
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.until, Some(date(2026, 10, 1)), "today + 2");
        assert_eq!(task.waiting_since, Some(now().timestamp()));
        assert!(
            task.body.contains("waiting on A; follow up 2026-10-01"),
            "{}",
            task.body
        );

        // Created already waiting, with an explicit date and a ref.
        let out = store
            .apply(
                Command::Add(AddTask {
                    title: "TTL question".into(),
                    waiting_on: Some("B".into()),
                    until: Some(date(2026, 10, 5)),
                    waiting_ref: Some("https://teams/thread/1".into()),
                    ..Default::default()
                }),
                &now(),
            )
            .unwrap();
        let t2 = store.tree().get(out.primary.as_ref().unwrap()).unwrap();
        assert_eq!(t2.status, Status::Waiting);
        assert_eq!(t2.until, Some(date(2026, 10, 5)));
        assert_eq!(t2.waiting_ref.as_deref(), Some("https://teams/thread/1"));

        // Leaving waiting forgets the wait entirely.
        store
            .apply(
                Command::SetStatus {
                    id: t.clone(),
                    status: Status::Todo,
                    waiting_on: None,
                    until: None,
                    cascade: false,
                },
                &now(),
            )
            .unwrap();
        let task = store.tree().get(&t).unwrap();
        assert!(task.waiting_on.is_none() && task.until.is_none() && task.waiting_since.is_none());
    }

    #[test]
    fn chase_escalates_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let t = add(&mut store, "DNS");
        assert!(matches!(
            store.apply(
                Command::Chase {
                    id: t.clone(),
                    until: None,
                    note: None,
                    source: None,
                    dedupe: None
                },
                &now()
            ),
            Err(StoreError::InvalidTransition { .. })
        ));
        wait(&mut store, &t, "B", None);

        let chase = |store: &mut Store, dedupe: Option<&str>, at: &Zoned| {
            store
                .apply(
                    Command::Chase {
                        id: t.clone(),
                        until: None,
                        note: Some("pinged again".into()),
                        source: Some("agent".into()),
                        dedupe: dedupe.map(str::to_string),
                    },
                    at,
                )
                .unwrap()
        };
        let day1 = now() + 2.days();
        let out = chase(&mut store, Some("2026-10-01"), &day1);
        assert!(out.events.iter().any(|e| matches!(
            e,
            Event::Chased {
                count: 1,
                deduped: false,
                ..
            }
        )));
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.chases.len(), 1);
        assert_eq!(task.until, Some(date(2026, 10, 2)), "2 - 1 = 1 day out");
        assert!(
            task.body.contains(
                "[agent] chased B; follow up 2026-10-02: pinged again [chase:2026-10-01]"
            ),
            "{}",
            task.body
        );

        // Same dedupe key: no-op, reported.
        let out = chase(&mut store, Some("2026-10-01"), &day1);
        assert!(
            out.events
                .iter()
                .any(|e| matches!(e, Event::Chased { deduped: true, .. }))
        );
        assert_eq!(store.tree().get(&t).unwrap().chases.len(), 1);

        // Next day: another chase, follow-up never shorter than 1 day.
        let day2 = now() + 3.days();
        chase(&mut store, Some("2026-10-02"), &day2);
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.chases.len(), 2);
        assert_eq!(task.until, Some(date(2026, 10, 3)));
    }

    #[test]
    fn handoff_and_replies() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        let t = add(&mut store, "DNS");
        wait(&mut store, &t, "A", None);
        store
            .apply(
                Command::Chase {
                    id: t.clone(),
                    until: None,
                    note: None,
                    source: None,
                    dedupe: None,
                },
                &now(),
            )
            .unwrap();

        // Handoff via edit: new person, fresh clock and count, journaled.
        let later = now() + 1.day();
        let out = store
            .apply(
                Command::Edit(EditTask {
                    waiting_on: Set::To("B".into()),
                    ..EditTask::new(t.clone())
                }),
                &later,
            )
            .unwrap();
        assert!(out.events.contains(&Event::HandedOff {
            id: t.clone(),
            from: "A".into(),
            to: "B".into()
        }));
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.waiting_on.as_deref(), Some("B"));
        assert!(task.chases.is_empty());
        assert_eq!(task.waiting_since, Some(later.timestamp()));
        assert!(task.body.contains("handed off: A → B"));

        // Redirected reply: same effect, with the summary journaled.
        let out = store
            .apply(
                Command::RecordReply {
                    id: t.clone(),
                    outcome: ReplyOutcome::Redirected { to: "C".into() },
                    summary: "C owns DNS now".into(),
                    waiting_ref: Some("https://teams/thread/9".into()),
                    source: Some("agent".into()),
                },
                &later,
            )
            .unwrap();
        assert!(
            out.events
                .iter()
                .any(|e| matches!(e, Event::ReplyReceived { .. }))
        );
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.waiting_on.as_deref(), Some("C"));
        assert!(
            task.body
                .contains("[agent] B redirected to C: C owns DNS now")
        );
        assert_eq!(task.waiting_ref.as_deref(), Some("https://teams/thread/9"));

        // Empty summary is refused; answered makes it todo again.
        assert!(matches!(
            store.apply(
                Command::RecordReply {
                    id: t.clone(),
                    outcome: ReplyOutcome::Answered,
                    summary: "  ".into(),
                    waiting_ref: None,
                    source: None
                },
                &later
            ),
            Err(StoreError::Validation(_))
        ));
        store
            .apply(
                Command::RecordReply {
                    id: t.clone(),
                    outcome: ReplyOutcome::Answered,
                    summary: "use the wildcard".into(),
                    waiting_ref: None,
                    source: None,
                },
                &later,
            )
            .unwrap();
        let task = store.tree().get(&t).unwrap();
        assert_eq!(task.status, Status::Todo);
        assert!(task.waiting_on.is_none());
        assert!(task.body.contains("C replied: use the wildcard"));
    }
}

#[cfg(test)]
mod rename_migration_tests {
    use super::*;

    #[test]
    fn legacy_tododo_meta_dir_is_migrated_on_open() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(NOTES_DIR)).unwrap();
        fs::create_dir_all(dir.path().join(LEGACY_META_DIR)).unwrap();
        fs::write(dir.path().join(LEGACY_META_DIR).join("nudges.json"), "{}").unwrap();
        let store = Store::open(dir.path()).expect("opens a tododo-era directory");
        assert!(dir.path().join(META_DIR).join("nudges.json").exists());
        assert!(!dir.path().join(LEGACY_META_DIR).exists());
        drop(store);
        assert!(matches!(
            Store::init(dir.path()),
            Err(StoreError::AlreadyInitialized(_))
        ));
    }
}
