//! Panel state and key handling, with no terminal or network in sight so it
//! can be driven by tests.

use jiff::Zoned;
use jiff::civil::Date;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tododo_core::action::{RankConfig, Summary, derive, needs_action, summary};
use tododo_core::{Command, Context, NextItem, Status, Task, TaskId, Tree};

use crate::dates;

pub enum Row {
    Item(NextItem),
    Section(String),
    /// Open but not actionable: shown when `show_stuck` so the user can see
    /// what is blocking what.
    Stuck {
        id: TaskId,
        title: String,
        why: String,
        root: Option<String>,
    },
    /// One task in the tree view, with its computed state spelled out.
    Node {
        id: TaskId,
        depth: usize,
        title: String,
        state: String,
        kind: NodeKind,
        age_secs: i64,
    },
}

/// How a tree row should read at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// Yours to do right now.
    Free,
    Doing,
    /// Waiting on subtasks (or, via them, on a person).
    Blocked,
    /// Waiting on a person directly.
    Waiting,
    Someday,
    Closed,
}

/// Which list the panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The whole tree: every branch, free or stuck, and why.
    Tree,
    /// The ranked "what now?" list.
    Next,
}

impl Row {
    pub fn id(&self) -> Option<&TaskId> {
        match self {
            Row::Item(i) => Some(&i.id),
            Row::Stuck { id, .. } | Row::Node { id, .. } => Some(id),
            Row::Section(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    Add { parent: Option<TaskId> },
    Wait { id: TaskId },
    Touch { id: TaskId },
}

#[derive(Clone, Debug)]
pub struct Prompt {
    pub kind: PromptKind,
    pub buf: String,
}

impl Prompt {
    pub fn label(&self) -> &'static str {
        match self.kind {
            PromptKind::Add { parent: None } => "add task",
            PromptKind::Add { parent: Some(_) } => "add subtask",
            PromptKind::Wait { .. } => "waiting on (append ' @ +3d' for a follow-up date)",
            PromptKind::Touch { .. } => "note",
        }
    }
}

pub enum Mode {
    Normal,
    Help,
    Prompt(Prompt),
    ConfirmQuit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Success,
    Error,
}

/// What the event loop should do after a key.
pub enum Action {
    None,
    Apply(Command),
    Refresh,
    OpenEditor(TaskId),
    Quit,
}

pub struct App {
    pub tree: Tree,
    pub ctx: Option<Context>,
    pub view: View,
    pub show_stuck: bool,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub summary: Summary,
    pub mode: Mode,
    pub message: Option<(MessageKind, String)>,
    pub nudges: Vec<String>,
    pub connected: bool,
    pub now: Zoned,
}

impl App {
    pub fn new(tree: Tree, ctx: Option<Context>, now: Zoned) -> App {
        let mut app = App {
            tree,
            ctx,
            view: View::Tree,
            show_stuck: false,
            rows: Vec::new(),
            selected: 0,
            summary: Summary::default(),
            mode: Mode::Normal,
            message: None,
            nudges: Vec::new(),
            connected: true,
            now,
        };
        app.rebuild();
        app
    }

    pub fn set_tree(&mut self, tree: Tree, now: Zoned) {
        self.tree = tree;
        self.now = now;
        self.rebuild();
    }

    pub fn tick(&mut self, now: Zoned) {
        self.now = now;
        self.rebuild();
    }

    /// Recompute rows from the tree, keeping the selection on the same task
    /// where possible.
    pub fn rebuild(&mut self) {
        let keep = self.selected_id().cloned();
        let items = needs_action(&self.tree, &self.now, self.ctx, &RankConfig::default());
        self.summary = summary(&self.tree, &self.now, self.ctx);
        let mut rows: Vec<Row> = items.into_iter().map(Row::Item).collect();

        if self.show_stuck {
            let today = self.now.date();
            let mut stuck: Vec<Row> = Vec::new();
            for t in self.tree.iter().filter(|t| t.status.is_open()) {
                if !self.in_ctx(t) {
                    continue;
                }
                let d = derive(&self.tree, t, today);
                if d.actionable {
                    continue;
                }
                let why = if d.blocked_by > 0 {
                    format!("blocked by {}", d.blocked_by)
                } else if let Some(by) = &d.deferred_by {
                    if *by == t.id {
                        match t.status {
                            Status::Someday => "someday".to_string(),
                            _ => "deferred".to_string(),
                        }
                    } else {
                        format!("under {}", by)
                    }
                } else if t.status == Status::Waiting {
                    format!(
                        "waiting on {}{}",
                        t.waiting_on.as_deref().unwrap_or("?"),
                        t.until.map(|u| format!(" until {u}")).unwrap_or_default()
                    )
                } else {
                    t.status.to_string()
                };
                let root = self
                    .tree
                    .root_of(&t.id)
                    .filter(|r| r.id != t.id)
                    .map(|r| r.title.clone());
                stuck.push(Row::Stuck {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    why,
                    root,
                });
            }
            if !stuck.is_empty() {
                rows.push(Row::Section(format!("waiting & blocked ({})", stuck.len())));
                rows.extend(stuck);
            }
        }
        self.rows = if self.view == View::Tree {
            self.tree_rows()
        } else {
            rows
        };

        self.selected = keep
            .and_then(|id| self.rows.iter().position(|r| r.id() == Some(&id)))
            .unwrap_or_else(|| self.first_selectable());
        self.clamp();
    }

    /// Every open task (and anything closed in the last day), in outline
    /// order, each saying what it is waiting for.
    fn tree_rows(&self) -> Vec<Row> {
        let today = self.now.date();
        let ts = self.now.timestamp();
        let recent = |t: &Task| {
            t.completed
                .is_some_and(|c| ts.as_second() - c.as_second() < 24 * 3600)
        };
        let mut rows = Vec::new();
        let mut stack: Vec<(&Task, usize)> = self
            .tree
            .roots()
            .into_iter()
            .filter(|t| self.in_ctx(t))
            .filter(|t| t.status.is_open() || recent(t))
            .rev()
            .map(|t| (t, 0))
            .collect();
        while let Some((t, depth)) = stack.pop() {
            let d = derive(&self.tree, t, today);
            let (kind, mut state) = match t.status {
                Status::Done | Status::Dropped => (NodeKind::Closed, t.status.to_string()),
                Status::Someday => (NodeKind::Someday, "💤 someday".to_string()),
                Status::Waiting => (
                    NodeKind::Waiting,
                    format!(
                        "⌛ {} · {}{}{}",
                        t.waiting_on.as_deref().unwrap_or("?"),
                        humanize_secs(
                            ts.as_second() - t.waiting_since.unwrap_or(t.updated).as_second()
                        ),
                        match t.chases.len() {
                            0 => String::new(),
                            n => format!(" · {n}×"),
                        },
                        if d.followup { " · follow up!" } else { "" }
                    ),
                ),
                Status::Doing => (
                    NodeKind::Doing,
                    format!(
                        "▶ doing {}",
                        humanize_secs(ts.as_second() - t.started.unwrap_or(t.created).as_second())
                    ),
                ),
                Status::Todo => {
                    if let Some(w) = &d.waiting_on_summary {
                        (
                            NodeKind::Blocked,
                            format!(
                                "⌛ {} · {}",
                                w.person,
                                humanize_secs(ts.as_second() - w.since.as_second())
                            ),
                        )
                    } else if d.blocked_by > 0 {
                        (NodeKind::Blocked, format!("⏳ blocked by {}", d.blocked_by))
                    } else if d.deferred_by.is_some() {
                        (NodeKind::Someday, "deferred".to_string())
                    } else if d.resurfaced_at.is_some() {
                        (NodeKind::Free, "↩ back".to_string())
                    } else {
                        (NodeKind::Free, String::new())
                    }
                }
            };
            if d.orphaned {
                state.push_str(" · ⚠ parent finished");
            }
            if let Some(due) = t.due
                && t.status.is_open()
            {
                state.push_str(&format!(" · due {due}"));
            }
            rows.push(Row::Node {
                id: t.id.clone(),
                depth,
                title: t.title.clone(),
                state: state.trim_start_matches(" · ").to_string(),
                kind,
                age_secs: ts.as_second() - t.created.as_second(),
            });
            let kids: Vec<&Task> = self
                .tree
                .children(&t.id)
                .filter(|c| c.status.is_open() || recent(c))
                .collect();
            for c in kids.into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
        rows
    }

    fn in_ctx(&self, t: &Task) -> bool {
        match self.ctx {
            None => true,
            Some(c) => {
                self.tree
                    .root_of(&t.id)
                    .map(|r| r.context)
                    .unwrap_or(t.context)
                    == c
            }
        }
    }

    fn first_selectable(&self) -> usize {
        self.rows.iter().position(|r| r.id().is_some()).unwrap_or(0)
    }

    fn clamp(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.rows.len() {
            self.selected = self.rows.len() - 1;
        }
    }

    pub fn selected_id(&self) -> Option<&TaskId> {
        self.rows.get(self.selected).and_then(Row::id)
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.selected_id().and_then(|id| self.tree.get(id))
    }

    pub fn move_sel(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let mut i = self.selected as i32;
        loop {
            i += delta;
            if i < 0 || i as usize >= self.rows.len() {
                return;
            }
            if self.rows[i as usize].id().is_some() {
                self.selected = i as usize;
                return;
            }
        }
    }

    pub fn cycle_context(&mut self) {
        self.ctx = match self.ctx {
            None => Some(Context::Work),
            Some(Context::Work) => Some(Context::Personal),
            Some(Context::Personal) => None,
        };
        self.rebuild();
    }

    pub fn say(&mut self, kind: MessageKind, msg: impl Into<String>) {
        self.message = Some((kind, msg.into()));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        match &mut self.mode {
            Mode::Help => {
                self.mode = Mode::Normal;
                Action::None
            }
            Mode::ConfirmQuit => {
                self.mode = Mode::Normal;
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('q') | KeyCode::Enter => Action::Quit,
                    _ => Action::None,
                }
            }
            Mode::Prompt(p) => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    Action::None
                }
                KeyCode::Enter => {
                    let p = p.clone();
                    self.mode = Mode::Normal;
                    self.submit_prompt(p)
                }
                KeyCode::Backspace => {
                    p.buf.pop();
                    Action::None
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    p.buf.clear();
                    Action::None
                }
                KeyCode::Char(c) => {
                    p.buf.push(c);
                    Action::None
                }
                _ => Action::None,
            },
            Mode::Normal => self.handle_normal(key),
        }
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Action {
        self.message = None;
        let sel = self.selected_id().cloned();
        match key.code {
            KeyCode::Char('q') => {
                self.mode = Mode::ConfirmQuit;
                Action::None
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_sel(1);
                Action::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_sel(-1);
                Action::None
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.selected = self.first_selectable();
                Action::None
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.selected = self.rows.len().saturating_sub(1);
                if self
                    .rows
                    .get(self.selected)
                    .is_some_and(|r| r.id().is_none())
                {
                    self.move_sel(-1);
                }
                Action::None
            }
            KeyCode::Char('c') => {
                self.cycle_context();
                Action::None
            }
            KeyCode::Char('v') => {
                self.view = match self.view {
                    View::Tree => View::Next,
                    View::Next => View::Tree,
                };
                self.rebuild();
                Action::None
            }
            KeyCode::Char('b') => {
                self.show_stuck = !self.show_stuck;
                self.rebuild();
                Action::None
            }
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Char('a') => {
                self.mode = Mode::Prompt(Prompt {
                    kind: PromptKind::Add { parent: None },
                    buf: String::new(),
                });
                Action::None
            }
            KeyCode::Char('A') => match sel {
                Some(id) => {
                    self.mode = Mode::Prompt(Prompt {
                        kind: PromptKind::Add { parent: Some(id) },
                        buf: String::new(),
                    });
                    Action::None
                }
                None => Action::None,
            },
            KeyCode::Char('d') => self.status(sel, Status::Done),
            KeyCode::Char('x') => self.status(sel, Status::Dropped),
            KeyCode::Char('s') => self.status(sel, Status::Doing),
            KeyCode::Char('o') => self.status(sel, Status::Todo),
            KeyCode::Char('z') => self.status(sel, Status::Someday),
            KeyCode::Char('w') => match sel {
                Some(id) => {
                    self.mode = Mode::Prompt(Prompt {
                        kind: PromptKind::Wait { id },
                        buf: String::new(),
                    });
                    Action::None
                }
                None => Action::None,
            },
            KeyCode::Char('f') => {
                // Chase: the selected waiting task, or the waiting subtask a
                // blocked parent ultimately depends on.
                let target = sel.and_then(|id| {
                    let t = self.tree.get(&id)?;
                    if t.status == Status::Waiting {
                        return Some(id);
                    }
                    derive(&self.tree, t, self.now.date())
                        .waiting_on_summary
                        .map(|w| w.task_id)
                });
                match target {
                    Some(id) => Action::Apply(Command::Chase {
                        id,
                        until: None,
                        note: None,
                        source: None,
                        dedupe: None,
                    }),
                    None => {
                        self.say(
                            MessageKind::Info,
                            "nothing to chase here: not waiting on anyone",
                        );
                        Action::None
                    }
                }
            }
            KeyCode::Char('t') => match sel {
                Some(id) => {
                    self.mode = Mode::Prompt(Prompt {
                        kind: PromptKind::Touch { id },
                        buf: String::new(),
                    });
                    Action::None
                }
                None => Action::None,
            },
            KeyCode::Char('e') | KeyCode::Enter => match sel {
                Some(id) => Action::OpenEditor(id),
                None => Action::None,
            },
            _ => Action::None,
        }
    }

    fn status(&mut self, id: Option<TaskId>, status: Status) -> Action {
        match id {
            Some(id) => Action::Apply(Command::SetStatus {
                id,
                status,
                waiting_on: None,
                until: None,
                cascade: false,
            }),
            None => Action::None,
        }
    }

    fn submit_prompt(&mut self, p: Prompt) -> Action {
        let text = p.buf.trim().to_string();
        match p.kind {
            PromptKind::Add { parent } => {
                if text.is_empty() {
                    return Action::None;
                }
                let context = if parent.is_none() { self.ctx } else { None };
                Action::Apply(Command::Add(tododo_core::command::AddTask {
                    title: text,
                    parent,
                    context,
                    ..Default::default()
                }))
            }
            PromptKind::Wait { id } => {
                if text.is_empty() {
                    self.say(MessageKind::Error, "waiting needs a reason");
                    return Action::None;
                }
                let (reason, until) = split_until(&text);
                let until: Option<Date> = match until {
                    Some(u) => match dates::parse_date(u) {
                        Ok(d) => Some(d),
                        Err(e) => {
                            self.say(MessageKind::Error, e);
                            return Action::None;
                        }
                    },
                    None => None,
                };
                Action::Apply(Command::SetStatus {
                    id,
                    status: Status::Waiting,
                    waiting_on: Some(reason.to_string()),
                    until,
                    cascade: false,
                })
            }
            PromptKind::Touch { id } => Action::Apply(Command::Touch {
                id,
                note: if text.is_empty() { None } else { Some(text) },
            }),
        }
    }
}

/// "Sam's review @ +3d" → ("Sam's review", Some("+3d")).
fn split_until(s: &str) -> (&str, Option<&str>) {
    match s.rsplit_once(" @ ") {
        Some((reason, until)) if !until.trim().is_empty() => (reason.trim(), Some(until.trim())),
        _ => (s, None),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use jiff::{Timestamp, ToSpan};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    pub(crate) fn fixture() -> (Tree, Zoned) {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, status: Status, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t.status = status;
            if status == Status::Doing {
                t.started = Some(at(h));
            }
            if status.is_closed() {
                t.completed = Some(at(1));
            }
            t
        };
        let mut tasks = vec![
            mk("stry00", "Ship auth story", None, Status::Todo, 72),
            mk(
                "jwt000",
                "Fix JWT expiry",
                Some("stry00"),
                Status::Doing,
                48,
            ),
            mk(
                "m1gr00",
                "Write migration",
                Some("stry00"),
                Status::Todo,
                24,
            ),
            mk("back00", "Renew domain", None, Status::Todo, 200),
            mk(
                "k1d000",
                "Pick registrar",
                Some("back00"),
                Status::Done,
                100,
            ),
            mk("wa1t00", "Renew passport", None, Status::Waiting, 96),
            mk("1atr00", "Learn Zig", None, Status::Someday, 300),
        ];
        tasks[5].context = Context::Personal;
        tasks[5].waiting_on = Some("Service Canada".into());
        tasks[2].due = Some(jiff::civil::date(2026, 9, 20));
        (Tree::from_tasks(tasks), now)
    }

    #[test]
    fn rows_rank_and_selection_survives_rebuild() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        app.view = View::Next;
        app.rebuild();
        let ids: Vec<&str> = app
            .rows
            .iter()
            .filter_map(|r| r.id().map(|i| i.as_str()))
            .collect();
        assert_eq!(
            ids,
            ["m1gr00", "back00", "jwt000"],
            "overdue, resurfaced, doing"
        );

        app.move_sel(1);
        assert_eq!(app.selected_id().unwrap().as_str(), "back00");
        app.show_stuck = true;
        app.rebuild();
        assert_eq!(app.selected_id().unwrap().as_str(), "back00");
        assert!(app.rows.iter().any(|r| matches!(r, Row::Section(_))));
        assert!(
            app.rows
                .iter()
                .any(|r| matches!(r, Row::Stuck { id, .. } if id.as_str() == "stry00"))
        );
    }

    #[test]
    fn keys_produce_commands() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        app.view = View::Next;
        app.rebuild();
        assert!(matches!(
            app.handle_key(key('d')),
            Action::Apply(Command::SetStatus {
                status: Status::Done,
                ..
            })
        ));

        assert!(matches!(app.handle_key(key('A')), Action::None));
        for c in "Sub task".chars() {
            app.handle_key(key(c));
        }
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        match app.handle_key(enter) {
            Action::Apply(Command::Add(add)) => {
                assert_eq!(add.title, "Sub task");
                assert_eq!(add.parent.unwrap().as_str(), "m1gr00");
            }
            _ => panic!("expected add"),
        }

        app.handle_key(key('w'));
        for c in "Sam @ +3d".chars() {
            app.handle_key(key(c));
        }
        match app.handle_key(enter) {
            Action::Apply(Command::SetStatus {
                status: Status::Waiting,
                waiting_on,
                until,
                ..
            }) => {
                assert_eq!(waiting_on.as_deref(), Some("Sam"));
                assert!(until.is_some());
            }
            _ => panic!("expected wait"),
        }

        app.handle_key(key('c'));
        assert_eq!(app.ctx, Some(Context::Work));
        assert!(matches!(app.handle_key(key('q')), Action::None));
        assert!(matches!(app.handle_key(key('y')), Action::Quit));
    }
}

fn humanize_secs(secs: i64) -> String {
    tododo_core::humanize::duration(secs)
}

#[cfg(test)]
mod tree_view_tests {
    use super::*;
    use jiff::{Timestamp, ToSpan};

    /// story ─ A ─ (ask B, waiting) ; story ─ write CR (free)
    fn fixture() -> (Tree, Zoned) {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t
        };
        let mut ask = mk("ask000", "Ask B about wildcard certs", Some("f1g000"), 90);
        ask.status = Status::Waiting;
        ask.waiting_on = Some("B".into());
        ask.waiting_since = Some(at(90));
        ask.chases = vec![at(40)];
        ask.until = Some(now.date());
        let tasks = vec![
            mk("dns000", "Get DNS address", None, 100),
            mk("f1g000", "Figure out what's possible", Some("dns000"), 95),
            ask,
            mk("wr1te0", "Write the change request", Some("dns000"), 80),
        ];
        (Tree::from_tasks(tasks), now)
    }

    #[test]
    fn tree_view_shows_every_branch_and_who_it_waits_on() {
        let (tree, now) = fixture();
        let app = App::new(tree, None, now);
        assert_eq!(app.view, View::Tree);
        let rows: Vec<(usize, &str, &str, NodeKind)> = app
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Node {
                    depth,
                    title,
                    state,
                    kind,
                    ..
                } => Some((*depth, title.as_str(), state.as_str(), *kind)),
                _ => None,
            })
            .collect();
        assert_eq!(
            rows,
            [
                (0, "Get DNS address", "⌛ B · 3d", NodeKind::Blocked),
                (
                    1,
                    "Figure out what's possible",
                    "⌛ B · 3d",
                    NodeKind::Blocked
                ),
                (
                    2,
                    "Ask B about wildcard certs",
                    "⌛ B · 3d · 1× · follow up!",
                    NodeKind::Waiting
                ),
                (1, "Write the change request", "", NodeKind::Free),
            ]
        );
        // `v` flips to the ranked list, which shows only what is yours to do.
        let mut app = app;
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_eq!(app.view, View::Next);
        let ids: Vec<&str> = app
            .rows
            .iter()
            .filter_map(|r| r.id().map(|i| i.as_str()))
            .collect();
        assert_eq!(
            ids,
            ["ask000", "wr1te0"],
            "follow-up due + the free sibling"
        );
    }
}
