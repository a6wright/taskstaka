//! Panel state and key handling, with no terminal or network in sight so it
//! can be driven by tests. Mirrors the web app: Next (the in-hand plate and
//! each story's now/later stack), Outline, Waiting on, Someday, Nudges.

use std::collections::HashSet;

use jiff::Zoned;
use jiff::civil::Date;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use taskstaka_core::action::{
    LaterItem, NextItem, PersonGroup, RankConfig, Summary, WaitingItem, derive, in_hand,
    is_agent_step, plan, summary, waiting_on,
};
use taskstaka_core::nudge::Nudge;
use taskstaka_core::{Command, Context, Status, Task, TaskId, Tree};

use crate::dates;

/// Which page the panel shows. `1`–`5` jump; `v` cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Next,
    Outline,
    Waiting,
    Someday,
    Nudges,
}

impl View {
    pub const ALL: [View; 5] = [
        View::Next,
        View::Outline,
        View::Waiting,
        View::Someday,
        View::Nudges,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Next => "Next",
            View::Outline => "Outline",
            View::Waiting => "Waiting on",
            View::Someday => "Someday",
            View::Nudges => "Nudges",
        }
    }
}

/// How an outline row should read at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Free,
    Doing,
    Blocked,
    Waiting,
    Someday,
    Closed,
}

pub enum Row {
    /// The task in hand: the most recently started, open, unblocked task.
    Plate(TaskId),
    /// A story's base line on Next.
    Story {
        id: TaskId,
        title: String,
        now: usize,
        later: usize,
        /// `stuck` reason, or who it is waiting on.
        note: Option<String>,
    },
    /// A now row (actionable leaf) on Next.
    Item(NextItem),
    /// "… 3 later" — Enter or `l` unfolds.
    LaterToggle {
        story: TaskId,
        count: usize,
        agent: usize,
        expanded: bool,
        peek: String,
    },
    Later(LaterItem),
    /// One task in the outline (Outline and Someday views).
    Node {
        id: TaskId,
        depth: usize,
        title: String,
        state: String,
        kind: NodeKind,
        age_secs: i64,
    },
    /// A person header on Waiting on.
    Person {
        name: String,
        count: usize,
        due: bool,
        oldest_secs: i64,
    },
    Wait(WaitingItem),
    Nudge(Nudge),
    Section(String),
    /// A blank line between stories on Next.
    Gap,
}

impl Row {
    /// The task an action on this row applies to.
    pub fn id(&self) -> Option<&TaskId> {
        match self {
            Row::Plate(id) | Row::Story { id, .. } => Some(id),
            Row::Item(i) => Some(&i.id),
            Row::Later(l) => Some(&l.id),
            Row::Node { id, .. } => Some(id),
            Row::Wait(w) => Some(&w.wait.task_id),
            Row::Nudge(n) => n.task.as_ref(),
            Row::LaterToggle { .. } | Row::Person { .. } | Row::Section(_) | Row::Gap => None,
        }
    }

    pub fn selectable(&self) -> bool {
        !matches!(self, Row::Section(_) | Row::Person { .. } | Row::Gap)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// `status` is what the new task starts as (`Doing` for a push from the
    /// task in hand); `park` creates it `someday`.
    Add {
        parent: Option<TaskId>,
        status: Option<Status>,
        park: bool,
    },
    Wait {
        id: TaskId,
    },
    Touch {
        id: TaskId,
    },
}

#[derive(Clone, Debug)]
pub struct Prompt {
    pub kind: PromptKind,
    pub buf: String,
}

impl Prompt {
    pub fn label(&self) -> &'static str {
        match &self.kind {
            PromptKind::Add {
                parent: None,
                park: true,
                ..
            } => "park an idea",
            PromptKind::Add { parent: None, .. } => "add task",
            PromptKind::Add {
                status: Some(Status::Doing),
                ..
            } => "first I need to… (goes in hand)",
            PromptKind::Add { .. } => "first I need to…",
            PromptKind::Wait { .. } => "waiting on whom? (append ' @ +3d' for a follow-up date)",
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
    /// Outline / Someday: include tasks closed in the last two weeks.
    pub show_done: bool,
    /// The boxed detail pane under the list. Hidden until Space opens it.
    pub show_detail: bool,
    /// Next: stories whose later list is unfolded.
    pub expanded: HashSet<TaskId>,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub summary: Summary,
    pub mode: Mode,
    pub message: Option<(MessageKind, String)>,
    pub nudges: Vec<Nudge>,
    pub connected: bool,
    pub now: Zoned,
}

impl App {
    pub fn new(tree: Tree, ctx: Option<Context>, now: Zoned) -> App {
        let mut app = App {
            tree,
            ctx,
            view: View::Next,
            show_done: false,
            show_detail: false,
            expanded: HashSet::new(),
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

    pub fn in_hand(&self) -> Option<&Task> {
        in_hand(&self.tree, &self.now, self.ctx)
    }

    /// Recompute rows for the current view, keeping the selection on the
    /// same task where possible.
    pub fn rebuild(&mut self) {
        let keep = self.selected_id().cloned();
        self.summary = summary(&self.tree, &self.now, self.ctx);
        self.rows = match self.view {
            View::Next => self.next_rows(),
            View::Outline => self.outline_rows(false),
            View::Someday => self.outline_rows(true),
            View::Waiting => self.waiting_rows(),
            View::Nudges => self.nudges.iter().cloned().map(Row::Nudge).collect(),
        };
        self.selected = keep
            .and_then(|id| self.rows.iter().position(|r| r.id() == Some(&id)))
            .unwrap_or_else(|| self.first_selectable());
        self.clamp();
    }

    /// The plate, then each story as a stack: now rows, a folded later line.
    fn next_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if let Some(t) = self.in_hand() {
            rows.push(Row::Plate(t.id.clone()));
        }
        let stories = plan(&self.tree, &self.now, self.ctx, &RankConfig::default());
        let last = stories.len().saturating_sub(1);
        for (i, p) in stories.into_iter().enumerate() {
            // Reads top-down like the web's plates: now, later folded
            // beneath, the story at the base.
            let note = p.stuck.clone().or_else(|| {
                (!p.waiting_on.is_empty()).then(|| format!("✉ {}", p.waiting_on.join(", ")))
            });
            let now_n = p.now.len();
            for it in p.now {
                rows.push(Row::Item(it));
            }
            let later_n = p.later.len();
            if !p.later.is_empty() {
                let expanded = self.expanded.contains(&p.id);
                let mut peek = p
                    .later
                    .iter()
                    .take(2)
                    .map(|l| l.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if p.later.len() > 2 {
                    peek.push_str(&format!(", +{}", p.later.len() - 2));
                }
                rows.push(Row::LaterToggle {
                    story: p.id.clone(),
                    count: p.later.len(),
                    agent: p.later_agent,
                    expanded,
                    peek,
                });
                if expanded {
                    rows.extend(p.later.into_iter().map(Row::Later));
                }
            }
            rows.push(Row::Story {
                id: p.id,
                title: p.title,
                now: now_n,
                later: later_n,
                note,
            });
            if i < last {
                rows.push(Row::Gap);
            }
        }
        rows
    }

    /// The outline, or (`parked`) the Someday page: parked stories as an
    /// outline, then parked leaves that sit under live stories.
    fn outline_rows(&self, parked: bool) -> Vec<Row> {
        let today = self.now.date();
        let ts = self.now.timestamp();
        let recent = |t: &Task| {
            self.show_done
                && t.completed
                    .is_some_and(|c| ts.as_second() - c.as_second() < 14 * 24 * 3600)
        };
        let mut rows = Vec::new();
        let roots: Vec<&Task> = self
            .tree
            .roots()
            .into_iter()
            .filter(|t| self.in_ctx(t))
            .filter(|t| (t.status == Status::Someday) == parked)
            .filter(|t| t.status.is_open() || recent(t))
            .collect();
        if parked && !roots.is_empty() {
            rows.push(Row::Section(format!("parked stories ({})", roots.len())));
        }
        let mut stack: Vec<(&Task, usize)> = roots.into_iter().rev().map(|t| (t, 0)).collect();
        while let Some((t, depth)) = stack.pop() {
            let d = derive(&self.tree, t, today);
            let (kind, mut state) = match t.status {
                Status::Done | Status::Dropped => (NodeKind::Closed, t.status.to_string()),
                Status::Someday => (
                    NodeKind::Someday,
                    if parked && depth == 0 {
                        String::new()
                    } else {
                        "💤 someday".to_string()
                    },
                ),
                Status::Waiting => (
                    NodeKind::Waiting,
                    format!(
                        "✉ {} · {}{}{}",
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
                                "✉ {} · {}",
                                w.person,
                                humanize_secs(ts.as_second() - w.since.as_second())
                            ),
                        )
                    } else if d.blocked_by > 0 {
                        (NodeKind::Blocked, format!("▽ blocked by {}", d.blocked_by))
                    } else if d.deferred_by.is_some() {
                        (NodeKind::Someday, "deferred".to_string())
                    } else if d.resurfaced_at.is_some() {
                        (NodeKind::Free, "↑ ready".to_string())
                    } else {
                        (NodeKind::Free, String::new())
                    }
                }
            };
            if is_agent_step(t) {
                state = join(&state, "🤖 Claude");
            }
            if d.orphaned {
                state = join(&state, "⚠ parent finished");
            }
            if let Some(due) = t.due
                && t.status.is_open()
            {
                state = join(&state, &format!("due {due}"));
            }
            rows.push(Row::Node {
                id: t.id.clone(),
                depth,
                title: t.title.clone(),
                state,
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
        if parked {
            let leaves: Vec<&Task> = self
                .tree
                .iter()
                .filter(|t| t.status == Status::Someday && self.in_ctx(t))
                .filter(|t| {
                    t.parent
                        .as_ref()
                        .and_then(|p| self.tree.root_of(p))
                        .is_some_and(|r| r.status != Status::Someday)
                })
                .collect();
            if !leaves.is_empty() {
                rows.push(Row::Section(format!(
                    "parked under live stories ({})",
                    leaves.len()
                )));
                for t in leaves {
                    let root = self
                        .tree
                        .root_of(&t.id)
                        .map(|r| r.title.clone())
                        .unwrap_or_default();
                    rows.push(Row::Node {
                        id: t.id.clone(),
                        depth: 0,
                        title: t.title.clone(),
                        state: format!("‹ {root}"),
                        kind: NodeKind::Someday,
                        age_secs: ts.as_second() - t.created.as_second(),
                    });
                }
            }
        }
        rows
    }

    fn waiting_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let groups: Vec<PersonGroup> = waiting_on(&self.tree, &self.now, self.ctx);
        for g in groups {
            rows.push(Row::Person {
                name: g.person.clone(),
                count: g.items.len(),
                due: g.any_due,
                oldest_secs: g.oldest_secs,
            });
            rows.extend(g.items.into_iter().map(Row::Wait));
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
        self.rows.iter().position(Row::selectable).unwrap_or(0)
    }

    fn clamp(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.rows.len() {
            self.selected = self.rows.len() - 1;
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    pub fn selected_id(&self) -> Option<&TaskId> {
        self.selected_row().and_then(Row::id)
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
            if self.rows[i as usize].selectable() {
                self.selected = i as usize;
                return;
            }
        }
    }

    pub fn set_view(&mut self, view: View) {
        if self.view != view {
            self.view = view;
            self.selected = 0;
            self.rebuild();
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

    /// Fold/unfold the later list of the story the selected row belongs to.
    fn toggle_later(&mut self) {
        let story = match self.selected_row() {
            Some(Row::LaterToggle { story, .. }) => Some(story.clone()),
            Some(r) => r
                .id()
                .and_then(|id| self.tree.root_of(id))
                .map(|r| r.id.clone()),
            None => None,
        };
        if let Some(id) = story {
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.rebuild();
        }
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

    fn prompt(&mut self, kind: PromptKind) -> Action {
        self.mode = Mode::Prompt(Prompt {
            kind,
            buf: String::new(),
        });
        Action::None
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Action {
        self.message = None;
        let sel = self.selected_id().cloned();
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let outline = matches!(self.view, View::Outline | View::Someday);
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
            KeyCode::Char(d @ '1'..='5') => {
                self.set_view(View::ALL[(d as u8 - b'1') as usize]);
                Action::None
            }
            KeyCode::Char('v') => {
                let i = View::ALL.iter().position(|v| *v == self.view).unwrap_or(0);
                self.set_view(View::ALL[(i + 1) % View::ALL.len()]);
                Action::None
            }
            // Shift+arrows rearrange the outline: up/down among siblings,
            // right nests under the row above, left steps out.
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right if shift && outline => {
                match sel.and_then(|id| self.shift_move(&id, key.code)) {
                    Some(cmd) => Action::Apply(cmd),
                    None => Action::None,
                }
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
                    .is_some_and(|r| !r.selectable())
                {
                    self.move_sel(-1);
                }
                Action::None
            }
            KeyCode::Char('l') => {
                self.toggle_later();
                Action::None
            }
            KeyCode::Enter => match self.selected_row() {
                Some(Row::LaterToggle { .. }) => {
                    self.toggle_later();
                    Action::None
                }
                _ => match sel {
                    Some(id) => Action::OpenEditor(id),
                    None => Action::None,
                },
            },
            KeyCode::Char('e') => match sel {
                Some(id) => Action::OpenEditor(id),
                None => Action::None,
            },
            KeyCode::Char('c') => {
                self.cycle_context();
                Action::None
            }
            KeyCode::Char(' ') => {
                self.show_detail = !self.show_detail;
                Action::None
            }
            KeyCode::Esc if self.show_detail => {
                self.show_detail = false;
                Action::None
            }
            KeyCode::Char('h') => {
                self.show_done = !self.show_done;
                self.rebuild();
                Action::None
            }
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Char('a') => self.prompt(PromptKind::Add {
                parent: None,
                status: None,
                park: self.view == View::Someday,
            }),
            KeyCode::Char('A') => match sel {
                Some(id) => self.prompt(PromptKind::Add {
                    parent: Some(id),
                    status: None,
                    park: false,
                }),
                None => Action::None,
            },
            // Push: "first I need to…" — a prerequisite under the selected
            // task. From the task in hand it starts in hand and the store
            // pauses the task above.
            KeyCode::Char('i') => match sel {
                Some(id) => {
                    let in_hand = self.in_hand().is_some_and(|t| t.id == id);
                    self.prompt(PromptKind::Add {
                        parent: Some(id),
                        status: in_hand.then_some(Status::Doing),
                        park: false,
                    })
                }
                None => Action::None,
            },
            KeyCode::Char('d') => self.status(sel, Status::Done),
            KeyCode::Char('x') => self.status(sel, Status::Dropped),
            // start / unblock / revive all mean "it is ready and I am on it".
            KeyCode::Char('s') | KeyCode::Char('u') => self.status(sel, Status::Doing),
            // put back (plate) / reopen
            KeyCode::Char('p') | KeyCode::Char('o') => self.status(sel, Status::Todo),
            KeyCode::Char('z') => self.status(sel, Status::Someday),
            KeyCode::Char('w') => match sel {
                Some(id) => self.prompt(PromptKind::Wait { id }),
                None => Action::None,
            },
            KeyCode::Char('t') => match sel {
                Some(id) => self.prompt(PromptKind::Touch { id }),
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

    /// Build the Move for a Shift+arrow on `id`, or None if it cannot move
    /// that way (first/last sibling, root, no row above).
    fn shift_move(&self, id: &TaskId, dir: KeyCode) -> Option<Command> {
        let t = self.tree.get(id)?;
        let siblings: Vec<&Task> = match &t.parent {
            Some(p) => self.tree.children(p).collect(),
            None => self.tree.roots(),
        };
        let i = siblings.iter().position(|s| s.id == *id)?;
        let (parent, after, before) = match dir {
            KeyCode::Up => (
                t.parent.clone(),
                None,
                Some(siblings.get(i.checked_sub(1)?)?.id.clone()),
            ),
            KeyCode::Down => (
                t.parent.clone(),
                Some(siblings.get(i + 1)?.id.clone()),
                None,
            ),
            KeyCode::Right => (
                Some(siblings.get(i.checked_sub(1)?)?.id.clone()),
                None,
                None,
            ),
            KeyCode::Left => {
                let p = t.parent.clone()?;
                let gp = self.tree.get(&p)?.parent.clone();
                (gp, Some(p), None)
            }
            _ => return None,
        };
        Some(Command::Move {
            id: id.clone(),
            parent,
            after,
            before,
        })
    }

    fn submit_prompt(&mut self, p: Prompt) -> Action {
        let text = p.buf.trim().to_string();
        match p.kind {
            PromptKind::Add {
                parent,
                status,
                park,
            } => {
                if text.is_empty() {
                    return Action::None;
                }
                let context = if parent.is_none() { self.ctx } else { None };
                Action::Apply(Command::Add(taskstaka_core::command::AddTask {
                    title: text,
                    parent,
                    context,
                    status: if park { Some(Status::Someday) } else { status },
                    ..Default::default()
                }))
            }
            PromptKind::Wait { id } => {
                if text.is_empty() {
                    self.say(MessageKind::Error, "waiting on whom?");
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

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        format!("{a} · {b}")
    }
}

fn humanize_secs(secs: i64) -> String {
    taskstaka_core::humanize::duration(secs)
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

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    /// The DNS story as a stack (Verify › Create 🤖 › Log in › Install), an
    /// email in hand, a question out to Priya, and a parked idea.
    pub(crate) fn fixture() -> (Tree, Zoned) {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let at = |h: i64| -> Timestamp { now.timestamp() - (h * 3600).seconds() };
        let mk = |id: &str, title: &str, parent: Option<&str>, h: i64| {
            let mut t = Task::new(id.parse::<TaskId>().unwrap(), title, Context::Work, at(h));
            t.parent = parent.map(|p| p.parse().unwrap());
            t
        };
        let mut create = mk("cr8000", "Create the zone and record", Some("vrfy00"), 50);
        create.tags = vec!["agent".into()];
        let mut ask = mk(
            "askpr0",
            "Ask Priya about wildcard certs",
            Some("dns000"),
            40,
        );
        ask.status = Status::Waiting;
        ask.waiting_on = Some("Priya".into());
        ask.waiting_since = Some(at(72));
        ask.chases = vec![at(40)];
        ask.until = Some(now.date().checked_add(2.days()).unwrap());
        let mut email = mk(
            "ema110",
            "Email platform-eng to approve the zone",
            Some("dns000"),
            30,
        );
        email.status = Status::Doing;
        email.started = Some(at(1));
        email.body = "- 2026-09-29 14:00 [user] draft: hi team, could you approve…\n".into();
        let mut parked = mk("park00", "Learn Zig", None, 300);
        parked.status = Status::Someday;
        let tasks = vec![
            mk("dns000", "Get DNS address", None, 100),
            mk("vrfy00", "Verify the record resolves", Some("dns000"), 60),
            create,
            mk("10g1n0", "Log in: gcloud auth login", Some("cr8000"), 45),
            mk("1nst00", "Install the gcloud CLI", Some("10g1n0"), 44),
            email,
            ask,
            parked,
        ];
        (Tree::from_tasks(tasks), now)
    }

    fn kinds(app: &App) -> Vec<String> {
        app.rows
            .iter()
            .map(|r| match r {
                Row::Plate(id) => format!("plate:{id}"),
                Row::Story {
                    title, now, later, ..
                } => format!("story:{title}:{now}/{later}"),
                Row::Item(i) => format!("now:{}", i.title),
                Row::LaterToggle {
                    count, expanded, ..
                } => format!("later-toggle:{count}:{expanded}"),
                Row::Later(l) => format!("later:{}", l.title),
                Row::Node { title, depth, .. } => format!("node:{depth}:{title}"),
                Row::Person { name, .. } => format!("person:{name}"),
                Row::Wait(w) => format!("wait:{}", w.wait.title),
                Row::Nudge(n) => format!("nudge:{}", n.title),
                Row::Section(s) => format!("section:{s}"),
                Row::Gap => "gap".to_string(),
            })
            .collect()
    }

    #[test]
    fn next_is_the_plate_then_stacks_with_later_folded() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        assert_eq!(
            kinds(&app),
            [
                "plate:ema110",
                "now:Install the gcloud CLI",
                "now:Email platform-eng to approve the zone",
                "later-toggle:5:false",
                "story:Get DNS address:2/5",
            ]
        );
        assert_eq!(app.selected_id().unwrap().as_str(), "ema110");
        // `l` unfolds the story's later list; the selection stays put.
        app.handle_key(key('l'));
        let k = kinds(&app);
        assert!(
            k.contains(&"later:Log in: gcloud auth login".to_string()),
            "{k:?}"
        );
        assert!(k.contains(&"later:Ask Priya about wildcard certs".to_string()));
        assert_eq!(app.selected_id().unwrap().as_str(), "ema110");
        // A later row is a real task: Enter opens it, d closes it.
        app.selected = k
            .iter()
            .position(|x| x == "later:Log in: gcloud auth login")
            .unwrap();
        assert!(
            matches!(app.handle_key(enter()), Action::OpenEditor(id) if id.as_str() == "10g1n0")
        );
        assert!(matches!(
            app.handle_key(key('d')),
            Action::Apply(Command::SetStatus { id, status: Status::Done, .. }) if id.as_str() == "10g1n0"
        ));
        // Enter on the toggle row folds it again.
        app.selected = k
            .iter()
            .position(|x| x.starts_with("later-toggle"))
            .unwrap();
        app.handle_key(enter());
        assert_eq!(kinds(&app).len(), 5);
    }

    #[test]
    fn shift_move_refuses_the_edges() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        app.handle_key(key('2'));
        app.handle_key(key('g'));
        let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        // A root cannot step out.
        assert_eq!(app.selected_id().unwrap().as_str(), "dns000");
        assert!(matches!(app.handle_key(shift(KeyCode::Left)), Action::None));
        // The first subtask cannot go up or nest under a row above it.
        app.handle_key(key('j'));
        assert_eq!(app.selected_id().unwrap().as_str(), "vrfy00");
        assert!(matches!(app.handle_key(shift(KeyCode::Up)), Action::None));
        assert!(matches!(
            app.handle_key(shift(KeyCode::Right)),
            Action::None
        ));
        // But it can step out to become a root right after its story.
        assert!(matches!(
            app.handle_key(shift(KeyCode::Left)),
            Action::Apply(Command::Move { parent: None, after: Some(a), .. }) if a.as_str() == "dns000"
        ));
    }

    #[test]
    fn push_from_the_plate_starts_the_prerequisite_in_hand() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        app.handle_key(key('i'));
        for c in "Find the approval template".chars() {
            app.handle_key(key(c));
        }
        match app.handle_key(enter()) {
            Action::Apply(Command::Add(add)) => {
                assert_eq!(add.title, "Find the approval template");
                assert_eq!(add.parent.unwrap().as_str(), "ema110");
                assert_eq!(add.status, Some(Status::Doing), "goes in hand");
            }
            _ => panic!("expected an add"),
        }
        // `A` adds a sibling-style subtask that stays todo; `p` puts the
        // plate back; `d` finishes it.
        app.handle_key(key('A'));
        app.handle_key(key('x'));
        match app.handle_key(enter()) {
            Action::Apply(Command::Add(add)) => assert_eq!(add.status, None),
            _ => panic!("expected an add"),
        }
        assert!(matches!(
            app.handle_key(key('p')),
            Action::Apply(Command::SetStatus {
                status: Status::Todo,
                ..
            })
        ));
        assert!(matches!(
            app.handle_key(key('d')),
            Action::Apply(Command::SetStatus {
                status: Status::Done,
                ..
            })
        ));
    }

    #[test]
    fn views_show_the_right_rows_and_cycle() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        app.handle_key(key('2'));
        assert_eq!(app.view, View::Outline);
        let k = kinds(&app);
        assert_eq!(k[0], "node:0:Get DNS address");
        assert!(
            k.contains(&"node:4:Install the gcloud CLI".to_string()),
            "{k:?}"
        );
        assert!(
            !k.iter().any(|x| x.contains("Learn Zig")),
            "parked stories stay out of the outline"
        );
        let create = app
            .rows
            .iter()
            .find_map(|r| match r {
                Row::Node { title, state, .. } if title.starts_with("Create") => {
                    Some(state.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(create, "▽ blocked by 1 · 🤖 Claude");

        app.handle_key(key('3'));
        assert_eq!(
            kinds(&app),
            ["person:Priya", "wait:Ask Priya about wildcard certs"]
        );
        assert!(matches!(
            app.handle_key(key('f')),
            Action::Apply(Command::Chase { .. })
        ));

        app.handle_key(key('4'));
        assert_eq!(
            kinds(&app),
            ["section:parked stories (1)", "node:0:Learn Zig"]
        );
        // `a` on Someday parks; `u` revives.
        app.handle_key(key('a'));
        app.handle_key(key('x'));
        match app.handle_key(enter()) {
            Action::Apply(Command::Add(add)) => assert_eq!(add.status, Some(Status::Someday)),
            _ => panic!("expected an add"),
        }
        assert!(matches!(
            app.handle_key(key('u')),
            Action::Apply(Command::SetStatus {
                status: Status::Doing,
                ..
            })
        ));

        app.handle_key(key('v'));
        assert_eq!(app.view, View::Nudges);
        app.handle_key(key('v'));
        assert_eq!(app.view, View::Next);
    }

    #[test]
    fn wait_prompt_context_and_quit_confirmation() {
        let (tree, now) = fixture();
        let mut app = App::new(tree, None, now);
        app.handle_key(key('w'));
        for c in "Sam @ +3d".chars() {
            app.handle_key(key(c));
        }
        match app.handle_key(enter()) {
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
        app.handle_key(key('c'));
        assert_eq!(app.ctx, Some(Context::Personal));
        assert!(app.rows.is_empty(), "nothing personal in the fixture");
        assert!(matches!(app.handle_key(key('q')), Action::None));
        assert!(matches!(app.handle_key(key('y')), Action::Quit));
    }
}
