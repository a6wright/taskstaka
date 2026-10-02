//! Command dispatch. Everything below `run` is written against `Backend`, so
//! the same code serves `--direct` and the daemon.

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use jiff::Zoned;
use taskstaka_core::action::{RankConfig, needs_action, summary};
use taskstaka_core::command::{AddTask, EditTask, Set};
use taskstaka_core::{Command, Event, Outcome, Status, Store, Task, TaskId, Tree};

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
            // Idempotent, like `git init`: a second run is not an error.
            if dir.join(taskstaka_core::store::META_DIR).is_dir() {
                println!("already initialized {}", dir.display());
                return Ok(());
            }
            let store =
                Store::init(&dir).with_context(|| format!("initializing {}", dir.display()))?;
            println!("initialized {}", store.dir().display());
            println!("  notes/     one markdown file per task");
            println!("  index.md   generated tree (open it in Obsidian)");
            return Ok(());
        }
        Cmd::Serve {
            detach,
            no_notify,
            quiet,
        } => {
            let port = cli.port.unwrap_or(crate::DEFAULT_PORT);
            if detach {
                crate::client::spawn_daemon(&dir, port)?;
                let c = Client::connect(&dir, Some(port))?;
                println!("daemon running at {}", c.base());
                return Ok(());
            }
            return server::run(server::ServeOpts {
                dir,
                port,
                notify: !no_notify,
                quiet_hours: quiet,
            });
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
        Cmd::Restart => {
            let was_running = Client::stop_and_wait(&dir)?;
            let port = cli.port.unwrap_or(crate::DEFAULT_PORT);
            crate::client::spawn_daemon(&dir, port)?;
            let c = Client::connect(&dir, Some(port))?;
            let h = c.health()?;
            println!(
                "{} daemon at {}  pid {}  v{}",
                if was_running { "restarted" } else { "started" },
                c.base(),
                h.pid,
                h.version
            );
            return Ok(());
        }
        Cmd::Tui { context } => return crate::tui::run(&dir, cli.port, context),
        Cmd::Mcp => return crate::mcp::run(&dir, cli.port),
        Cmd::Setup {
            target,
            project,
            skip_skill,
            skip_mcp,
            dry_run,
        } => {
            if target != "claude" {
                bail!("unknown setup target {target:?}; try `taskstaka setup claude`");
            }
            let opts = crate::setup::ClaudeSetup {
                project,
                skip_skill,
                skip_mcp,
                dry_run,
            };
            let report = crate::setup::claude(&opts)?;
            print!("{}", crate::setup::describe(&report, &opts));
            return Ok(());
        }
        Cmd::Status => {
            return match Client::discover(&dir) {
                Some(c) => {
                    let h = c.health()?;
                    let stale = c.is_stale();
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"running": true, "url": c.base(), "pid": h.pid, "version": h.version, "rev": h.rev, "dir": h.todo_dir, "stale": stale})
                        );
                    } else {
                        println!(
                            "running  {}  pid {}  v{}  rev {}  {}{}",
                            c.base(),
                            h.pid,
                            h.version,
                            h.rev,
                            h.todo_dir.display(),
                            if stale {
                                "  (stale: started before this binary was built — run `taskstaka restart`)"
                            } else {
                                ""
                            }
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

    let direct = cli.direct || env_truthy("TASKSTAKA_DIRECT");
    let mut backend: Box<dyn Backend> = if direct {
        Box::new(Direct::open(&dir)?)
    } else {
        Box::new(Http::connect(&dir, cli.port)?)
    };
    for w in backend.warnings() {
        eprintln!("warning: {w}");
    }
    if !direct
        && !env_truthy("TASKSTAKA_NO_STALE_WARNING")
        && Client::discover(&dir).is_some_and(|c| c.is_stale())
    {
        eprintln!(
            "warning: the daemon started before this binary was built — run `taskstaka restart`"
        );
    }
    run(backend.as_mut(), cli.cmd, json)
}

fn reply_label(o: &taskstaka_core::command::ReplyOutcome) -> String {
    use taskstaka_core::command::ReplyOutcome as R;
    match o {
        R::Answered => "answered".into(),
        R::Declined => "declined".into(),
        R::Partial => "partial".into(),
        R::Redirected { to } => format!("redirected to {to}"),
    }
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
        Cmd::Init
        | Cmd::Serve { .. }
        | Cmd::Stop
        | Cmd::Restart
        | Cmd::Status
        | Cmd::Tui { .. }
        | Cmd::Mcp
        | Cmd::Setup { .. } => {
            unreachable!("handled in main")
        }

        Cmd::Chase { id, until, note } => {
            let id = b.resolve(&id)?;
            let note = if note.is_empty() {
                None
            } else {
                Some(note.join(" "))
            };
            let out = b.apply(Command::Chase {
                id,
                until,
                note,
                source: None,
                dedupe: None,
            })?;
            report(b, &out, json, &now)
        }

        Cmd::Waiting { context, due } => {
            let tree = b.tree()?;
            let mut groups = taskstaka_core::action::waiting_on(&tree, &now, context);
            if due {
                for g in &mut groups {
                    g.items.retain(|i| i.due_now);
                }
                groups.retain(|g| !g.items.is_empty());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
            } else if groups.is_empty() {
                println!("not waiting on anyone");
            } else {
                print!("{}", render::waiting_text(&groups, &now));
            }
            Ok(())
        }

        Cmd::Reply {
            id,
            outcome,
            to,
            waiting_ref,
            summary,
        } => {
            use taskstaka_core::command::ReplyOutcome as R;
            let id = b.resolve(&id)?;
            let outcome = match outcome.to_ascii_lowercase().as_str() {
                "answered" | "yes" | "replied" => R::Answered,
                "declined" | "no" => R::Declined,
                "partial" => R::Partial,
                "redirected" | "redirect" => R::Redirected {
                    to: to
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("redirected: say who with --to PERSON"))?,
                },
                other => bail!(
                    "unknown outcome {other:?}: answered | declined | partial | redirected --to X"
                ),
            };
            let out = b.apply(Command::RecordReply {
                id,
                outcome,
                summary: summary.join(" "),
                waiting_ref,
                source: None,
            })?;
            report(b, &out, json, &now)
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
        Cmd::Wait {
            id,
            until,
            waiting_ref,
            reason,
        } => {
            let rid = b.resolve(&id)?;
            set_status(
                b,
                &id,
                Status::Waiting,
                Some(reason.join(" ")),
                until,
                false,
                json,
                &now,
            )?;
            if let Some(r) = waiting_ref {
                let mut edit = EditTask::new(rid);
                edit.waiting_ref = Set::To(r);
                b.apply(Command::Edit(edit))?;
            }
            Ok(())
        }

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
            let out = b.apply(Command::Move {
                id,
                parent,
                after: None,
                before: None,
            })?;
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
        after: None,
        waiting_on: a.waiting_on,
        until: a.until,
        waiting_ref: a.waiting_ref,
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
    edit.waiting_on = e.waiting_on.map(Set::To).unwrap_or(Set::Keep);
    edit.waiting_ref = e.waiting_ref.map(Set::To).unwrap_or(Set::Keep);
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
            Event::CascadeClosed { id, status } => {
                println!("{status} subtask {id}  {}", title(id))
            }
            Event::Chased {
                id,
                person,
                count,
                deduped,
            } => {
                if *deduped {
                    println!("{id}  already chased {person} today; nothing changed");
                } else {
                    let until = tree
                        .get(id)
                        .and_then(|t| t.until)
                        .map(|u| format!("; follow up again {u}"))
                        .unwrap_or_default();
                    println!("{id}  chased {person} ({count}×){until}");
                }
            }
            Event::ReplyReceived {
                id,
                person,
                outcome,
            } => println!(
                "{id}  {person} replied ({}): {}",
                reply_label(outcome),
                title(id)
            ),
            Event::HandedOff { id, from, to } => {
                println!("{id}  handed off: {from} → {to}  (follow-up reset)")
            }
            Event::Deleted { id } => println!("deleted {id}"),
            Event::ReloadedFromDisk { id } => println!("(reloaded {id} from disk first)"),
        }
    }
    if out.events.is_empty()
        && let Some(id) = &out.primary
    {
        let touched = tree
            .get(id)
            .map(|t| taskstaka_core::humanize::since(t.updated, now.timestamp()))
            .unwrap_or_default();
        println!("updated {id}  {}  (touched {touched})", title(id));
    }
    Ok(())
}
