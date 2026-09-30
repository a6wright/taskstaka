//! Command-line surface. Kept free of behaviour so `cli.rs` stays readable.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tododo_core::{Context, Priority};

use crate::dates;

/// tododo: what needs action next, backed by markdown.
#[derive(Parser, Debug)]
#[command(name = "tododo", version, about, max_term_width = 100)]
pub struct Cli {
    /// Todo directory. Defaults to $TODODO_DIR, then ~/todo.
    #[arg(long, global = true, env = "TODODO_DIR")]
    pub dir: Option<PathBuf>,

    /// Daemon port (default 7407). 0 picks a free port when starting one.
    #[arg(long, global = true, env = "TODODO_PORT")]
    pub port: Option<u16>,

    /// Skip the daemon and operate on the files directly (exclusive lock).
    /// Also enabled by TODODO_DIRECT=1.
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
        #[arg(long, env = "TODODO_NO_NOTIFY")]
        no_notify: bool,
        /// Suppress desktop notifications between these local hours, e.g. 22-8.
        #[arg(long, env = "TODODO_QUIET", value_parser = parse_quiet)]
        quiet: Option<(u8, u8)>,
    },

    /// Ask the running daemon to shut down.
    Stop,

    /// Show whether a daemon is running and where.
    Status,

    /// Speak MCP on stdin/stdout for agents (Claude Code, Claude Desktop).
    Mcp,

    /// The always-on panel: what needs action, live.
    Tui {
        /// Start filtered to this context (c cycles at runtime).
        #[arg(short, long, env = "TODODO_CONTEXT")]
        context: Option<Context>,
    },

    /// Add a task. Words become the title; flags can go anywhere.
    Add(AddArgs),

    /// What needs action next, ranked.
    Next(NextArgs),

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

    /// Mark as doing (records `started`).
    Start { id: String },

    /// Mark done. Refuses if open subtasks remain unless --cascade.
    Done {
        id: String,
        /// Also drop every open subtask.
        #[arg(long)]
        cascade: bool,
    },

    /// Mark dropped (won't do). Same subtask rule as `done`.
    Drop {
        id: String,
        #[arg(long)]
        cascade: bool,
    },

    /// Back to todo from any status (reopens closed ancestors).
    Reopen { id: String },

    /// Park it. Someday tasks don't block their parent and don't rank.
    Someday { id: String },

    /// Waiting on something external. The reason is required.
    Wait {
        id: String,
        /// Follow-up date: YYYY-MM-DD, today, tomorrow, +Nd.
        #[arg(long, value_parser = dates::parse_date)]
        until: Option<jiff::civil::Date>,
        /// What you are waiting on.
        #[arg(required = true)]
        reason: Vec<String>,
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
    #[arg(required = true)]
    pub title: Vec<String>,
}

#[derive(Args, Debug)]
pub struct NextArgs {
    /// Only this context (default: all).
    #[arg(short, long, env = "TODODO_CONTEXT")]
    pub context: Option<Context>,
    /// Ignore the TODODO_CONTEXT default and show every context.
    #[arg(long, conflicts_with = "context")]
    pub all: bool,
    #[arg(short = 'n', long, default_value_t = 10)]
    pub limit: usize,
    /// Show the reason for each row's rank.
    #[arg(long)]
    pub explain: bool,
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

pub fn default_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("todo")
}
