//! Command-line surface. Kept free of behaviour so `cli.rs` stays readable.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use taskstaka_core::{Context, Priority};

use crate::dates;

/// taskstaka: what needs action next, backed by markdown.
#[derive(Parser, Debug)]
#[command(name = "taskstaka", version, about, max_term_width = 100)]
pub struct Cli {
    /// Todo directory. Defaults to $TASKSTAKA_DIR, then the saved setting, then ~/todo.
    #[arg(long, global = true, env = "TASKSTAKA_DIR")]
    pub dir: Option<PathBuf>,

    /// Daemon port (default 7407). 0 picks a free port when starting one.
    #[arg(long, global = true, env = "TASKSTAKA_PORT")]
    pub port: Option<u16>,

    /// Skip the daemon and operate on the files directly (exclusive lock).
    /// Also enabled by TASKSTAKA_DIRECT=1.
    #[arg(long, global = true)]
    pub direct: bool,

    /// Emit JSON instead of text where supported.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Create the todo directory layout.
    Init,

    /// Run the daemon in the foreground (or detached with --detach).
    Serve {
        /// Fork into the background and return immediately.
        #[arg(long)]
        detach: bool,
        /// Log nudges instead of sending desktop notifications.
        #[arg(long, env = "TASKSTAKA_NO_NOTIFY")]
        no_notify: bool,
        /// Suppress desktop notifications between these local hours, e.g. 22-8.
        #[arg(long, env = "TASKSTAKA_QUIET", value_parser = parse_quiet)]
        quiet: Option<(u8, u8)>,
    },

    /// Ask the running daemon to shut down.
    Stop,

    /// Show whether a daemon is running and where.
    Status,

    /// Speak MCP on stdin/stdout for agents (Claude Code, Claude Desktop).
    Mcp,

    /// Install an integration: `claude` (skill + MCP server) or `macos`
    /// (launchd agents for the daemon and the menu bar item).
    Setup {
        /// What to set up: `claude` or `macos`.
        target: String,
        /// Install into this project (.claude/skills, MCP scope project) instead of your home.
        #[arg(long)]
        project: bool,
        #[arg(long)]
        skip_skill: bool,
        #[arg(long)]
        skip_mcp: bool,
        /// Do not add the taskstaka section to ~/.claude/CLAUDE.md (or ./CLAUDE.md with --project).
        #[arg(long)]
        skip_claude_md: bool,
        /// macos: leave the menu bar item out.
        #[arg(long)]
        no_tray: bool,
        /// macos: the tray's global key for quick add.
        #[arg(long, default_value = "alt+space")]
        hotkey: String,
        /// macos: unload and delete the agents instead.
        #[arg(long)]
        uninstall: bool,
        /// Show what would be done without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// The always-on panel: what needs action, live.
    Tui {
        /// Start filtered to this context (c cycles at runtime).
        #[arg(short, long, env = "TASKSTAKA_CONTEXT")]
        context: Option<Context>,
    },

    /// Add a task. Words become the title; flags can go anywhere.
    Add(AddArgs),

    /// Quick capture: a new root task from one line. `@Name [+3d]` at the end
    /// waits on them; `#project` as the first or last word sets the project.
    Quick {
        /// The line to capture.
        #[arg(required = true)]
        text: Vec<String>,
        #[arg(short, long, env = "TASKSTAKA_CONTEXT")]
        context: Option<Context>,
    },

    /// What do I have to do now? The task in hand, then each story as a
    /// stack: what you can do now and what is still required.
    Next(NextArgs),

    /// The stack you are on: the task in hand and what finishing it unlocks.
    /// Defaults to the task most recently started.
    Stack { id: Option<String> },

    /// Show the open tree.
    Ls {
        /// Include closed tasks.
        #[arg(long)]
        closed: bool,
        #[arg(short, long)]
        context: Option<Context>,
    },

    /// Show one task with its ancestry, subtasks and notes.
    Show { id: String },

    /// Mark as doing (records `started`). `unblock` is an alias: the person
    /// you were waiting on got back to you and it is ready to go.
    #[command(alias = "unblock")]
    Start { id: String },

    /// Mark done. Refuses if open subtasks remain unless --cascade.
    Done {
        id: String,
        /// Also close every open subtask, with the same status.
        #[arg(long)]
        cascade: bool,
    },

    /// Mark dropped (won't do). Same subtask rule as `done`.
    Drop {
        id: String,
        #[arg(long)]
        cascade: bool,
    },

    /// Back to todo from any status (a closed parent stays closed; the task
    /// is then shown as orphaned). `put-back`
    /// is an alias: take the task in hand out of your hand without finishing it.
    #[command(alias = "put-back")]
    Reopen { id: String },

    /// Park it. Someday tasks don't block their parent and don't rank.
    Someday { id: String },

    /// Waiting on someone. Follow-up defaults to 2 days out.
    Wait {
        id: String,
        /// Follow-up date: YYYY-MM-DD, today, tomorrow, +Nd.
        #[arg(long, value_parser = dates::parse_date)]
        until: Option<jiff::civil::Date>,
        /// Link to the conversation (thread URL, message id).
        #[arg(long = "ref")]
        waiting_ref: Option<String>,
        /// Who (or what) you are waiting on.
        #[arg(required = true)]
        reason: Vec<String>,
    },

    /// You poked them again: records the chase and shortens the next follow-up.
    Chase {
        id: String,
        #[arg(long, value_parser = dates::parse_date)]
        until: Option<jiff::civil::Date>,
        note: Vec<String>,
    },

    /// Who am I waiting on, for what, how long? Grouped by person.
    Waiting {
        #[arg(short, long)]
        context: Option<Context>,
        /// Only follow-ups that are due.
        #[arg(long)]
        due: bool,
    },

    /// They got back to you: answered | declined | partial | redirected TO.
    Reply {
        id: String,
        /// answered, declined, partial, or redirected
        outcome: String,
        /// For `redirected`: the person to wait on now.
        #[arg(long)]
        to: Option<String>,
        #[arg(long = "ref")]
        waiting_ref: Option<String>,
        /// One line on what they said.
        #[arg(required = true)]
        summary: Vec<String>,
    },

    /// Stop the daemon and start a fresh one (after installing a new build).
    Restart {
        /// Start the new daemon on this directory instead (used when the
        /// notes directory is changed from Settings).
        #[arg(long)]
        to: Option<PathBuf>,
    },

    /// Desktop notifications: show the setting, or turn them `on` / `off`.
    /// Takes effect immediately; nudges still show in the panel.
    Notifications {
        #[arg(value_parser = ["on", "off"])]
        state: Option<String>,
    },

    /// Bump "last touched"; optionally journal a note into the body.
    Touch { id: String, note: Vec<String> },

    /// Reparent a task. Omit PARENT (or pass --root) to make it a root.
    Mv {
        id: String,
        parent: Option<String>,
        #[arg(long, conflicts_with = "parent")]
        root: bool,
    },

    /// Edit fields; with no flags, opens the note in $EDITOR.
    Edit(EditArgs),

    /// Permanently delete (prefer `drop`). Leaves only unless --force.
    Rm {
        id: String,
        #[arg(long)]
        force: bool,
    },

    /// Regenerate index.md.
    Reindex,

    /// Print the path of a task's note (or the todo dir with no id).
    Path { id: Option<String> },
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Parent task id (or unique prefix).
    #[arg(short, long)]
    pub parent: Option<String>,
    /// work | personal. Defaults to the parent's, else work.
    #[arg(short, long)]
    pub context: Option<Context>,
    /// YYYY-MM-DD, today, tomorrow, +Nd.
    #[arg(long, value_parser = dates::parse_date)]
    pub due: Option<jiff::civil::Date>,
    /// high | low.
    #[arg(long)]
    pub priority: Option<Priority>,
    #[arg(short, long = "tag")]
    pub tags: Vec<String>,
    /// Create parked.
    #[arg(long)]
    pub someday: bool,
    /// Create and start immediately.
    #[arg(long, conflicts_with = "someday")]
    pub start: bool,
    /// Create it already waiting on this person.
    #[arg(short = 'w', long = "waiting-on", conflicts_with_all = ["someday", "start"])]
    pub waiting_on: Option<String>,
    /// Follow-up date for --waiting-on (default +2d).
    #[arg(long, value_parser = dates::parse_date, requires = "waiting_on")]
    pub until: Option<jiff::civil::Date>,
    /// Conversation link for --waiting-on.
    #[arg(long = "ref", requires = "waiting_on")]
    pub waiting_ref: Option<String>,
    /// Project of a new root story (subtasks inherit their root's).
    #[arg(long)]
    pub project: Option<String>,
    #[arg(required = true)]
    pub title: Vec<String>,
}

#[derive(Args, Debug)]
pub struct NextArgs {
    /// Only this context (default: all).
    #[arg(short, long, env = "TASKSTAKA_CONTEXT")]
    pub context: Option<Context>,
    /// Ignore the TASKSTAKA_CONTEXT default and show every context.
    #[arg(long, conflicts_with = "context")]
    pub all: bool,
    #[arg(short = 'n', long, default_value_t = 10)]
    pub limit: usize,
    /// Show the reason for each row's rank.
    #[arg(long)]
    pub explain: bool,
    /// Expand each story's "later" list (what is still required, and why not yet).
    #[arg(long)]
    pub later: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    pub id: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(short, long)]
    pub context: Option<Context>,
    #[arg(long, value_parser = dates::parse_date, conflicts_with = "no_due")]
    pub due: Option<jiff::civil::Date>,
    #[arg(long)]
    pub no_due: bool,
    #[arg(long, conflicts_with = "no_priority")]
    pub priority: Option<Priority>,
    #[arg(long)]
    pub no_priority: bool,
    /// Replace the tag list.
    #[arg(short, long = "tag")]
    pub tags: Vec<String>,
    /// Handoff: wait on this person instead (task must be waiting).
    #[arg(long = "waiting-on")]
    pub waiting_on: Option<String>,
    /// Set the conversation link.
    #[arg(long = "ref")]
    pub waiting_ref: Option<String>,
    /// Set the project (root stories only; subtasks inherit it).
    #[arg(long, conflicts_with = "no_project")]
    pub project: Option<String>,
    /// Remove the project.
    #[arg(long)]
    pub no_project: bool,
}

impl EditArgs {
    pub fn has_field_flags(&self) -> bool {
        self.title.is_some()
            || self.context.is_some()
            || self.due.is_some()
            || self.no_due
            || self.priority.is_some()
            || self.no_priority
            || !self.tags.is_empty()
            || self.waiting_on.is_some()
            || self.waiting_ref.is_some()
            || self.project.is_some()
            || self.no_project
    }
}

/// "22-8" → (22, 8).
pub fn parse_quiet(s: &str) -> Result<(u8, u8), String> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| format!("{s:?}: expected START-END hours, e.g. 22-8"))?;
    let parse = |x: &str| {
        x.trim()
            .parse::<u8>()
            .ok()
            .filter(|h| *h < 24)
            .ok_or_else(|| format!("{x:?} is not an hour (0-23)"))
    };
    Ok((parse(a)?, parse(b)?))
}

/// The notes directory when neither `--dir` nor `$TASKSTAKA_DIR` says: the one
/// saved in the machine settings, else `~/todo`.
pub fn default_dir() -> PathBuf {
    if let Some(d) = taskstaka_core::settings::Settings::load().dir {
        return d;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("todo")
}
