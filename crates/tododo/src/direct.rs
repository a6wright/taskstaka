//! Phase 1 execution: run commands against the store in-process. Once the
//! daemon exists this becomes the `--direct` fallback and the default path
//! goes over HTTP; the command → outcome shape stays identical.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use jiff::Zoned;
use tododo_core::action::{RankConfig, needs_action, summary};
use tododo_core::command::{AddTask, EditTask, Set};
use tododo_core::{Command, Event, Outcome, Status, Store, Task, TaskId};

use crate::render;
use crate::{AddArgs as AddCli, Cmd as Cli, EditArgs as EditCli, NextArgs as NextCli};

pub fn run(dir: &Path, cli: crate::Cli) -> Result<()> {
    let now = Zoned::now();
    let json = cli.json;

    if let Cli::Init = cli.cmd {
        let store = Store::init(dir).with_context(|| format!("initializing {}", dir.display()))?;
        println!("initialized {}", store.dir().display());
        println!("  notes/     one markdown file per task");
        println!("  index.md   generated tree (open it in Obsidian)");
        return Ok(());
    }

    let mut store = Store::open(dir)?;
    for w in &store.load_warnings {
        eprintln!("warning: {w}");
    }

    match cli.cmd {
        Cli::Init => unreachable!(),

        Cli::Add(a) => {
            let title = a.title.join(" ");
            let parent = a.parent.as_deref().map(|p| store.resolve(p)).transpose()?;
            let out = store.apply(Command::Add(add_task(a, title, parent)), &now)?;
            report(&store, &out, json, &now)
        }

        Cli::Next(n) => next(&store, n, json, &now),

        Cli::Ls { closed, context } => {
            let tree = store.tree();
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
                print!("{}", render::tree_text(tree, &now, &roots, closed));
            }
            Ok(())
        }

        Cli::Show { id } => {
            let id = store.resolve(&id)?;
            let t = store.tree().get(&id).expect("resolved");
            if json {
                println!("{}", serde_json::to_string_pretty(t)?);
            } else {
                print!("{}", render::show(store.tree(), t, &now));
            }
            Ok(())
        }

        Cli::Start { id } => set_status(
            &mut store,
            &id,
            Status::Doing,
            None,
            None,
            false,
            json,
            &now,
        ),
        Cli::Done { id, cascade } => set_status(
            &mut store,
            &id,
            Status::Done,
            None,
            None,
            cascade,
            json,
            &now,
        ),
        Cli::Drop { id, cascade } => set_status(
            &mut store,
            &id,
            Status::Dropped,
            None,
            None,
            cascade,
            json,
            &now,
        ),
        Cli::Reopen { id } => {
            set_status(&mut store, &id, Status::Todo, None, None, false, json, &now)
        }
        Cli::Someday { id } => set_status(
            &mut store,
            &id,
            Status::Someday,
            None,
            None,
            false,
            json,
            &now,
        ),
        Cli::Wait { id, until, reason } => set_status(
            &mut store,
            &id,
            Status::Waiting,
            Some(reason.join(" ")),
            until,
            false,
            json,
            &now,
        ),

        Cli::Touch { id, note } => {
            let id = store.resolve(&id)?;
            let note = if note.is_empty() {
                None
            } else {
                Some(note.join(" "))
            };
            let out = store.apply(Command::Touch { id, note }, &now)?;
            report(&store, &out, json, &now)
        }

        Cli::Mv {
            id,
            parent,
            root: _,
        } => {
            let id = store.resolve(&id)?;
            let parent = parent.as_deref().map(|p| store.resolve(p)).transpose()?;
            let out = store.apply(Command::Move { id, parent }, &now)?;
            report(&store, &out, json, &now)
        }

        Cli::Edit(e) => edit(&mut store, e, json, &now),

        Cli::Rm { id, force } => {
            let id = store.resolve(&id)?;
            let out = store.apply(Command::Delete { id, force }, &now)?;
            report(&store, &out, json, &now)
        }

        Cli::Reindex => {
            store.reindex(&now)?;
            println!("wrote {}", store.dir().join("index.md").display());
            Ok(())
        }

        Cli::Path { id } => {
            match id {
                Some(id) => {
                    let id = store.resolve(&id)?;
                    println!("{}", store.path_of(&id).expect("resolved").display());
                }
                None => println!("{}", store.dir().display()),
            }
            Ok(())
        }
    }
}

fn add_task(a: AddCli, title: String, parent: Option<TaskId>) -> AddTask {
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
    store: &mut Store,
    id: &str,
    status: Status,
    waiting_on: Option<String>,
    until: Option<jiff::civil::Date>,
    cascade: bool,
    json: bool,
    now: &Zoned,
) -> Result<()> {
    let id = store.resolve(id)?;
    let out = store.apply(
        Command::SetStatus {
            id,
            status,
            waiting_on,
            until,
            cascade,
        },
        now,
    )?;
    report(store, &out, json, now)
}

fn edit(store: &mut Store, e: EditCli, json: bool, now: &Zoned) -> Result<()> {
    let id = store.resolve(&e.id)?;
    let has_flags = e.title.is_some()
        || e.context.is_some()
        || e.due.is_some()
        || e.no_due
        || e.priority.is_some()
        || e.no_priority
        || !e.tags.is_empty();

    if !has_flags {
        // Open the note itself. The store's external-edit guard picks up
        // whatever changed on the next command; we reload now so `show`
        // and index.md are immediately right.
        let path = store.path_of(&id).expect("resolved").to_path_buf();
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
        store.reload()?;
        store.reindex(now)?;
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
    let out = store.apply(Command::Edit(edit), now)?;
    report(store, &out, json, now)
}

fn next(store: &Store, n: NextCli, json: bool, now: &Zoned) -> Result<()> {
    let ctx = if n.all { None } else { n.context };
    let tree = store.tree();
    let items = needs_action(tree, now, ctx, &RankConfig::default());
    let s = summary(tree, now, ctx);
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
fn report(store: &Store, out: &Outcome, json: bool, now: &Zoned) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(out)?);
        return Ok(());
    }
    let tree = store.tree();
    let title = |id: &TaskId| tree.get(id).map(|t| t.title.as_str()).unwrap_or("?");
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }
    for ev in &out.events {
        match ev {
            Event::Created { id } => {
                let t = tree.get(id).expect("created");
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
        println!(
            "updated {id}  {}  (touched {})",
            title(id),
            tree.get(id)
                .map(|t| tododo_core::humanize::since(t.updated, now.timestamp()))
                .unwrap_or_default()
        );
    }
    Ok(())
}
