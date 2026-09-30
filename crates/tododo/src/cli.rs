//! Command dispatch. Everything below `run` is written against `Backend`, so
//! the same code serves `--direct` and the daemon.

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use jiff::Zoned;
use tododo_core::action::{RankConfig, needs_action, summary};
use tododo_core::command::{AddTask, EditTask, Set};
use tododo_core::{Command, Event, Outcome, Status, Store, Task, TaskId, Tree};

use crate::args::{AddArgs, Cli, Cmd, EditArgs, NextArgs, default_dir};
use crate::backend::{Backend, Direct, Http};
use crate::client::{Client, DaemonInfo};
use crate::{render, server};

pub fn main() -> Result<()> {
    let cli = Cli::parse();
    let dir = cli.dir.clone().unwrap_or_else(default_dir);
    let json = cli.json;

    match cli.cmd {
        Cmd::Init => {
            let store =
                Store::init(&dir).with_context(|| format!("initializing {}", dir.display()))?;
            println!("initialized {}", store.dir().display());
            println!("  notes/     one markdown file per task");
            println!("  index.md   generated tree (open it in Obsidian)");
            return Ok(());
        }
        Cmd::Serve { detach } => {
            let port = cli.port.unwrap_or(crate::DEFAULT_PORT);
            if detach {
                crate::client::spawn_daemon(&dir, port)?;
                let c = Client::connect(&dir, Some(port))?;
                println!("daemon running at {}", c.base());
                return Ok(());
            }
            return server::run(server::ServeOpts { dir, port });
        }
        Cmd::Stop => {
            return match Client::discover(&dir) {
                Some(c) => {
                    c.shutdown()?;
                    println!("asked the daemon at {} to stop", c.base());
                    Ok(())
                }
                None => {
                    DaemonInfo::remove(&dir);
                    println!("no daemon running for {}", dir.display());
                    Ok(())
                }
            };
        }
        Cmd::Tui { context } => return crate::tui::run(&dir, cli.port, context),
        Cmd::Status => {
            return match Client::discover(&dir) {
                Some(c) => {
                    let h = c.health()?;
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"running": true, "url": c.base(), "pid": h.pid, "version": h.version, "rev": h.rev, "dir": h.todo_dir})
                        );
                    } else {
                        println!(
                            "running  {}  pid {}  v{}  rev {}  {}",
                            c.base(),
                            h.pid,
                            h.version,
                            h.rev,
                            h.todo_dir.display()
                        );
                    }
                    Ok(())
                }
                None => {
                    if json {
                        println!("{}", serde_json::json!({"running": false, "dir": dir}));
                    } else {
                        println!("not running  ({})", dir.display());
                    }
                    Ok(())
                }
            };
        }
        _ => {}
    }

    let direct = cli.direct || env_truthy("TODODO_DIRECT");
    let mut backend: Box<dyn Backend> = if direct {
        Box::new(Direct::open(&dir)?)
    } else {
        Box::new(Http::connect(&dir, cli.port)?)
    };
    for w in backend.warnings() {
        eprintln!("warning: {w}");
    }
    run(backend.as_mut(), cli.cmd, json)
}

/// `1`, `true`, `yes`, `on` (any case) count as set.
fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn run(b: &mut dyn Backend, cmd: Cmd, json: bool) -> Result<()> {
    let now = Zoned::now();
    match cmd {
        Cmd::Init | Cmd::Serve { .. } | Cmd::Stop | Cmd::Status | Cmd::Tui { .. } => {
            unreachable!("handled in main")
        }

        Cmd::Add(a) => {
            let title = a.title.join(" ");
            let parent = a.parent.as_deref().map(|p| b.resolve(p)).transpose()?;
            let out = b.apply(Command::Add(add_task(a, title, parent)))?;
            report(b, &out, json, &now)
        }

        Cmd::Next(n) => next(b, n, json, &now),

        Cmd::Ls { closed, context } => {
            let tree = b.tree()?;
            let roots: Vec<&Task> = tree
                .roots()
                .into_iter()
                .filter(|t| context.is_none_or(|c| t.context == c))
                .filter(|t| closed || t.status.is_open())
                .collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&roots)?);
            } else if roots.is_empty() {
                println!("nothing here");
            } else {
                print!("{}", render::tree_text(&tree, &now, &roots, closed));
            }
            Ok(())
        }

        Cmd::Show { id } => {
            let id = b.resolve(&id)?;
            let tree = b.tree()?;
            let t = tree
                .get(&id)
                .context("task vanished between resolve and read")?;
            if json {
                println!("{}", serde_json::to_string_pretty(t)?);
            } else {
                print!("{}", render::show(&tree, t, &now));
            }
            Ok(())
        }

        Cmd::Start { id } => set_status(b, &id, Status::Doing, None, None, false, json, &now),
        Cmd::Done { id, cascade } => {
            set_status(b, &id, Status::Done, None, None, cascade, json, &now)
        }
        Cmd::Drop { id, cascade } => {
            set_status(b, &id, Status::Dropped, None, None, cascade, json, &now)
        }
        Cmd::Reopen { id } => set_status(b, &id, Status::Todo, None, None, false, json, &now),
        Cmd::Someday { id } => set_status(b, &id, Status::Someday, None, None, false, json, &now),
        Cmd::Wait { id, until, reason } => set_status(
            b,
            &id,
            Status::Waiting,
            Some(reason.join(" ")),
            until,
            false,
            json,
            &now,
        ),

        Cmd::Touch { id, note } => {
            let id = b.resolve(&id)?;
            let note = if note.is_empty() {
                None
            } else {
                Some(note.join(" "))
            };
            let out = b.apply(Command::Touch { id, note })?;
            report(b, &out, json, &now)
        }

        Cmd::Mv {
            id,
            parent,
            root: _,
        } => {
            let id = b.resolve(&id)?;
            let parent = parent.as_deref().map(|p| b.resolve(p)).transpose()?;
            let out = b.apply(Command::Move { id, parent })?;
            report(b, &out, json, &now)
        }

        Cmd::Edit(e) => edit(b, e, json, &now),

        Cmd::Rm { id, force } => {
            let id = b.resolve(&id)?;
            let out = b.apply(Command::Delete { id, force })?;
            report(b, &out, json, &now)
        }

        Cmd::Reindex => {
            b.reindex()?;
            println!("wrote {}", b.dir().join("index.md").display());
            Ok(())
        }

        Cmd::Path { id } => {
            match id {
                Some(id) => {
                    let id = b.resolve(&id)?;
                    println!("{}", b.path_of(&id)?.display());
                }
                None => println!("{}", b.dir().display()),
            }
            Ok(())
        }
    }
}

fn add_task(a: AddArgs, title: String, parent: Option<TaskId>) -> AddTask {
    AddTask {
        title,
        parent,
        context: a.context,
        priority: a.priority,
        due: a.due,
        tags: a.tags,
        body: String::new(),
        status: if a.someday {
            Some(Status::Someday)
        } else if a.start {
            Some(Status::Doing)
        } else {
            None
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn set_status(
    b: &mut dyn Backend,
    id: &str,
    status: Status,
    waiting_on: Option<String>,
    until: Option<jiff::civil::Date>,
    cascade: bool,
    json: bool,
    now: &Zoned,
) -> Result<()> {
    let id = b.resolve(id)?;
    let out = b.apply(Command::SetStatus {
        id,
        status,
        waiting_on,
        until,
        cascade,
    })?;
    report(b, &out, json, now)
}

fn edit(b: &mut dyn Backend, e: EditArgs, json: bool, now: &Zoned) -> Result<()> {
    let id = b.resolve(&e.id)?;

    if !e.has_field_flags() {
        // Open the note itself, then have the backend re-read disk so the
        // index and every client see the change immediately.
        let path = b.path_of(&id)?;
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".into());
        let mut parts = editor.split_whitespace();
        let prog = parts.next().unwrap_or("vi");
        let status = std::process::Command::new(prog)
            .args(parts)
            .arg(&path)
            .status()
            .with_context(|| format!("launching editor {editor:?}"))?;
        if !status.success() {
            bail!("editor exited with {status}");
        }
        b.reload()?;
        if !json {
            println!("reloaded {}", path.display());
        }
        return Ok(());
    }

    let mut edit = EditTask::new(id);
    edit.title = e.title;
    edit.context = e.context;
    edit.due = if e.no_due {
        Set::Clear
    } else {
        e.due.map(Set::To).unwrap_or(Set::Keep)
    };
    edit.priority = if e.no_priority {
        Set::Clear
    } else {
        e.priority.map(Set::To).unwrap_or(Set::Keep)
    };
    edit.tags = if e.tags.is_empty() {
        None
    } else {
        Some(e.tags)
    };
    let out = b.apply(Command::Edit(edit))?;
    report(b, &out, json, now)
}

fn next(b: &mut dyn Backend, n: NextArgs, json: bool, now: &Zoned) -> Result<()> {
    let ctx = if n.all { None } else { n.context };
    let tree = b.tree()?;
    let items = needs_action(&tree, now, ctx, &RankConfig::default());
    let s = summary(&tree, now, ctx);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "summary": s, "items": items }))?
        );
        return Ok(());
    }
    println!("{}", render::summary_line(&s));
    if !items.is_empty() {
        println!();
        print!(
            "{}",
            render::next_table(&items[..items.len().min(n.limit)], n.explain, ctx.is_none())
        );
        if items.len() > n.limit {
            println!("… and {} more (use -n)", items.len() - n.limit);
        }
    }
    Ok(())
}

/// Print what happened. Events are where the product speaks: a resurfaced
/// parent is called out loudly because that is the whole point.
fn report(b: &mut dyn Backend, out: &Outcome, json: bool, now: &Zoned) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(out)?);
        return Ok(());
    }
    let tree: Tree = b.tree()?;
    let title = |id: &TaskId| tree.get(id).map(|t| t.title.as_str()).unwrap_or("?");
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }
    for ev in &out.events {
        match ev {
            Event::Created { id } => {
                let t = tree.get(id).context("created task missing from snapshot")?;
                let under = t
                    .parent
                    .as_ref()
                    .map(|p| format!("  under {} ({})", title(p), p))
                    .unwrap_or_default();
                println!("added {id}  {}{under}", t.title);
            }
            Event::StatusChanged { id, from, to } => println!("{id}  {}: {from} → {to}", title(id)),
            Event::Resurfaced { id } => {
                println!();
                println!(
                    "↩  {}  ({id}) is back: all its subtasks are done. It needs action.",
                    title(id)
                );
            }
            Event::AutoReopened { id } => println!("reopened parent {id}  {}", title(id)),
            Event::CascadeDropped { id } => println!("dropped subtask {id}  {}", title(id)),
            Event::Deleted { id } => println!("deleted {id}"),
            Event::ReloadedFromDisk { id } => println!("(reloaded {id} from disk first)"),
        }
    }
    if out.events.is_empty()
        && let Some(id) = &out.primary
    {
        let touched = tree
            .get(id)
            .map(|t| tododo_core::humanize::since(t.updated, now.timestamp()))
            .unwrap_or_default();
        println!("updated {id}  {}  (touched {touched})", title(id));
    }
    Ok(())
}
