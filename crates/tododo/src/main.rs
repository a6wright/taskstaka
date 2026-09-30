mod dates;
mod direct;
mod render;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tododo_core::{Context, Priority};

/// tododo: what needs action next, backed by markdown.
#[derive(Parser, Debug)]
#[command(name = "tododo", version, about, max_term_width = 100)]
struct Cli {
    /// Todo directory. Defaults to $TODODO_DIR, then ~/todo.
    #[arg(long, global = true, env = "TODODO_DIR")]
    dir: Option<PathBuf>,

    /// Emit JSON instead of text where supported.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create the todo directory layout.
    Init,

    /// Add a task. Words after the flags become the title.
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

    /// Show one task with its ancestry, children and notes.
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
struct AddArgs {
    /// Parent task id (or unique prefix).
    #[arg(short, long)]
    parent: Option<String>,
    /// work | personal. Defaults to the parent's, else work.
    #[arg(short, long)]
    context: Option<Context>,
    /// YYYY-MM-DD, today, tomorrow, +Nd.
    #[arg(long, value_parser = dates::parse_date)]
    due: Option<jiff::civil::Date>,
    /// high | low.
    #[arg(long)]
    priority: Option<Priority>,
    #[arg(short, long = "tag")]
    tags: Vec<String>,
    /// Create parked.
    #[arg(long)]
    someday: bool,
    /// Create and start immediately.
    #[arg(long, conflicts_with = "someday")]
    start: bool,
    #[arg(required = true)]
    title: Vec<String>,
}

#[derive(Args, Debug)]
struct NextArgs {
    /// Only this context (default: all).
    #[arg(short, long, env = "TODODO_CONTEXT")]
    context: Option<Context>,
    /// Ignore the TODODO_CONTEXT default and show every context.
    #[arg(long, conflicts_with = "context")]
    all: bool,
    #[arg(short = 'n', long, default_value_t = 10)]
    limit: usize,
    /// Show the reason for each row's rank.
    #[arg(long)]
    explain: bool,
}

#[derive(Args, Debug)]
struct EditArgs {
    id: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(short, long)]
    context: Option<Context>,
    #[arg(long, value_parser = dates::parse_date, conflicts_with = "no_due")]
    due: Option<jiff::civil::Date>,
    #[arg(long)]
    no_due: bool,
    #[arg(long, conflicts_with = "no_priority")]
    priority: Option<Priority>,
    #[arg(long)]
    no_priority: bool,
    /// Replace the tag list.
    #[arg(short, long = "tag")]
    tags: Vec<String>,
}

fn default_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("todo")
}

fn main() {
    let cli = Cli::parse();
    let dir = cli.dir.clone().unwrap_or_else(default_dir);
    if let Err(e) = direct::run(&dir, cli) {
        eprintln!("error: {e}");
        let mut src = e.source();
        while let Some(s) = src {
            eprintln!("  caused by: {s}");
            src = s.source();
        }
        std::process::exit(1);
    }
}
