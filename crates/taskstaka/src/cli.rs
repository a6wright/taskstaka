//! Command dispatch. Everything below `run` is written against `Backend`, so
//! the same code serves `--direct` and the daemon.

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use jiff::Zoned;
use taskstaka_core::action::{RankConfig, summary};
use taskstaka_core::command::{AddTask, EditTask, Set};
use taskstaka_core::{Command, Event, Outcome, Status, Store, Task, TaskId, Tree};

use crate::args::{AddArgs, Cli, Cmd, EditArgs, NextArgs, default_dir};
use crate::backend::{Backend, Direct, Http};
use crate::client::{Client, DaemonInfo};
use crate::{render, server};

/// Bring the daemon up for `dir`: kick the launchd agent when one owns it,
/// else spawn our own detached daemon.
fn start_daemon(sup: &crate::service::Supervisor, dir: &std::path::Path, port: u16) -> Result<()> {
    match sup.label() {
        Some(label) => crate::service::Launchctl::kickstart(&crate::service::System, label),
        None => crate::client::spawn_daemon(dir, port),
    }
}

pub fn main() -> Result<()> {
    let cli = Cli::parse();
    // Canonical, so `./todo`, a trailing slash or a symlink all name the
    // same store as the daemon's `todo_dir`.
    let dir = cli.dir.clone().unwrap_or_else(default_dir);
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
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
                let c = Client::wait_for(&dir)?;
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
                    let pid = c.health().ok().map(|h| h.pid);
                    let sup = crate::service::detect(&dir, &default_dir(), pid);
                    c.shutdown()?;
                    println!("asked the daemon at {} to stop", c.base());
                    if let Some(label) = sup.label() {
                        println!(
                            "it is a launchd agent ({label}); it stays down until `taskstaka restart` or login"
                        );
                    }
                    Ok(())
                }
                None => {
                    DaemonInfo::remove(&dir);
                    println!("no daemon running for {}", dir.display());
                    Ok(())
                }
            };
        }
        Cmd::Restart { to } => {
            let port = cli.port.unwrap_or(crate::DEFAULT_PORT);
            // Validate before stopping anything: a bad folder must not cost a restart.
            let plan = match &to {
                Some(t) => Some(crate::notes_dir::plan(&dir, &t.to_string_lossy())?),
                None => None,
            };
            let pid = Client::discover(&dir)
                .and_then(|c| c.health().ok())
                .map(|h| h.pid);
            let sup = crate::service::detect(&dir, &default_dir(), pid);
            let was_running = Client::stop_and_wait(&dir)?;
            let mut target = dir.clone();
            if let Some(plan) = plan {
                // The old daemon is stopped, so the copy is final. If it fails,
                // bring the old one back rather than leave nothing running.
                let moved = crate::notes_dir::apply(&dir, &plan).and_then(|n| {
                    let mut settings = taskstaka_core::settings::Settings::load();
                    settings.dir = Some(plan.target.clone());
                    settings.save().context("saving the notes folder setting")?;
                    Ok(n)
                });
                match moved {
                    Ok(n) => {
                        if plan.kind == crate::notes_dir::Kind::CopyInto {
                            println!("copied {n} note(s) to {}", plan.target.display());
                        }
                        target = plan.target;
                    }
                    Err(e) => {
                        start_daemon(&sup, &dir, port)?;
                        return Err(
                            e.context("could not move the notes; the old folder is still in use")
                        );
                    }
                }
            }
            // Under launchd the agent is kicked instead of spawning our own
            // daemon, so it stays supervised (and the setting it just saved
            // is what it reads on the way up).
            start_daemon(&sup, &target, port)?;
            let c = Client::wait_for(&target)?;
            let h = c.health()?;
            println!(
                "{} daemon at {}  pid {}  v{}  {}{}",
                if was_running { "restarted" } else { "started" },
                c.base(),
                h.pid,
                h.version,
                target.display(),
                sup.label()
                    .map(|l| format!("  (launchd {l})"))
                    .unwrap_or_default()
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
            skip_claude_md,
            dry_run,
        } => {
            if target != "claude" {
                bail!("unknown setup target {target:?}; try `taskstaka setup claude`");
            }
            let opts = crate::setup::ClaudeSetup {
                project,
                skip_skill,
                skip_mcp,
                skip_claude_md,
                dry_run,
            };
            let report = crate::setup::claude(&opts)?;
            print!("{}", crate::setup::describe(&report, &opts));
            return Ok(());
        }
        Cmd::Notifications { state } => {
            use taskstaka_core::settings::Settings;
            let mut settings = Settings::load();
            if let Some(state) = state {
                settings.notifications = state == "on";
                settings.save().context("saving settings")?;
            }
            println!(
                "desktop notifications are {}",
                if settings.notifications { "on" } else { "off" }
            );
            return Ok(());
        }
        Cmd::Status => {
            return match Client::discover(&dir) {
                Some(c) => {
                    let h = c.health()?;
                    let stale = c.is_stale();
                    let sup = crate::service::detect(&dir, &default_dir(), Some(h.pid));
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"running": true, "url": c.base(), "pid": h.pid, "version": h.version, "rev": h.rev, "dir": h.todo_dir, "stale": stale, "supervisor": sup.label()})
                        );
                    } else {
                        println!(
                            "running  {}  pid {}  v{}  rev {}  {}{}{}",
                            c.base(),
                            h.pid,
                            h.version,
                            h.rev,
                            h.todo_dir.display(),
                            sup.label()
                                .map(|l| format!("  launchd {l}"))
                                .unwrap_or_default(),
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
                    let sup = crate::service::detect(&dir, &default_dir(), None);
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"running": false, "dir": dir, "supervisor": sup.label()})
                        );
                    } else {
                        println!(
                            "not running  ({}){}",
                            dir.display(),
                            sup.label()
                                .map(|l| format!(
                                    "  launchd {l} loaded; `taskstaka restart` starts it"
                                ))
                                .unwrap_or_default()
                        );
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
        | Cmd::Restart { .. }
        | Cmd::Notifications { .. }
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

        Cmd::Quick { text, context } => {
            let add = crate::quick::parse(&text.join(" "), context)?;
            let out = b.apply(Command::Add(add))?;
            report(b, &out, json, &now)
        }

        Cmd::Next(n) => next(b, n, json, &now),

        Cmd::Stack { id } => {
            let tree = b.tree()?;
            let target = match id {
                Some(id) => Some(b.resolve(&id)?),
                None => taskstaka_core::action::plate(
                    &tree,
                    &now,
                    None,
                    &taskstaka_core::RankConfig::default(),
                )
                .map(|p| p.task.id.clone()),
            };
            let Some(id) = target else {
                println!("nothing in hand and nothing ready — add something with `taskstaka add`");
                return Ok(());
            };
            let t = tree.get(&id).context("task vanished")?;
            if json {
                let chain: Vec<&Task> = tree.ancestors(&id);
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({ "task": t, "chain": chain })
                    )?
                );
            } else {
                print!("{}", render::stack(&tree, t, &now));
            }
            Ok(())
        }

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
        project: a.project,
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
    edit.project = if e.no_project {
        Set::Clear
    } else {
        e.project.map(Set::To).unwrap_or(Set::Keep)
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
    let plans = taskstaka_core::action::plan(&tree, now, ctx, &RankConfig::default());
    let s = summary(&tree, now, ctx);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "summary": s, "stories": plans }))?
        );
        return Ok(());
    }
    println!("{}", render::summary_line(&s));
    let hand = taskstaka_core::action::plate(&tree, now, ctx, &RankConfig::default());
    if !plans.is_empty() || hand.is_some() {
        println!();
        print!(
            "{}",
            render::plan_text(
                &tree,
                &plans,
                hand,
                now,
                render::PlanOpts {
                    explain: n.explain,
                    later: n.later,
                    limit: n.limit,
                    show_context: ctx.is_none(),
                },
            )
        );
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
                    "↑  {}  ({id}) is ready: all its prerequisites are done. It needs action.",
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
