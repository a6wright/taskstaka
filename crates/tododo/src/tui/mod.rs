//! The panel's event loop: terminal events on this thread, a long-poll
//! watcher on another, both feeding `App`.

pub mod app;
pub mod ui;

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use jiff::Zoned;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tododo_core::view::tree_from_views;
use tododo_core::{Context, Event as CoreEvent, Outcome, TaskId};

use crate::client::Client;
use app::{Action, App, MessageKind};

enum Msg {
    Changed,
    Disconnected,
}

pub fn run(dir: &Path, port_hint: Option<u16>, ctx: Option<Context>) -> Result<()> {
    let client = Client::connect(dir, port_hint)?;
    let rev = client.health()?.rev;
    let tree = tree_from_views(client.tasks(true)?);
    let mut app = App::new(tree, ctx, Zoned::now());

    let (tx, rx) = mpsc::channel::<Msg>();
    spawn_watcher(client.clone(), rev, tx);

    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &client, &rx);
    ratatui::restore();
    result
}

fn spawn_watcher(client: Client, mut rev: u64, tx: mpsc::Sender<Msg>) {
    std::thread::spawn(move || {
        let mut failures = 0;
        loop {
            match client.changes(rev) {
                Ok(new_rev) => {
                    failures = 0;
                    if new_rev > rev {
                        rev = new_rev;
                        if tx.send(Msg::Changed).is_err() {
                            return;
                        }
                    }
                }
                Err(_) => {
                    failures += 1;
                    if failures >= 2 && tx.send(Msg::Disconnected).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    if let Ok(h) = client.health() {
                        rev = h.rev;
                        failures = 0;
                        let _ = tx.send(Msg::Changed);
                    }
                }
            }
        }
    });
}

fn refresh(app: &mut App, client: &Client) {
    match client.tasks(true) {
        Ok(views) => {
            app.set_tree(tree_from_views(views), Zoned::now());
            app.connected = true;
            app.nudges = client
                .nudges()
                .map(|ns| ns.into_iter().map(|n| n.message).collect())
                .unwrap_or_default();
        }
        Err(e) => {
            app.connected = false;
            app.say(MessageKind::Error, format!("refresh failed: {e}"));
        }
    }
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &Client,
    rx: &mpsc::Receiver<Msg>,
) -> Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Wake for keys, the watcher, or a periodic age refresh.
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match app.handle_key(k) {
                    Action::None => {}
                    Action::Quit => return Ok(()),
                    Action::Refresh => refresh(app, client),
                    Action::Apply(cmd) => match client.apply(&cmd) {
                        Ok(out) => {
                            announce(app, &out);
                            refresh(app, client);
                        }
                        Err(e) => app.say(MessageKind::Error, e.to_string()),
                    },
                    Action::OpenEditor(id) => {
                        open_editor(terminal, app, client, &id)?;
                    }
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        let mut changed = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Changed => changed = true,
                Msg::Disconnected => app.connected = false,
            }
        }
        if changed {
            refresh(app, client);
        } else if last_tick.elapsed() > Duration::from_secs(30) {
            app.tick(Zoned::now());
            last_tick = Instant::now();
        }
    }
}

/// Turn an outcome into the one line the header shows.
fn announce(app: &mut App, out: &Outcome) {
    let title = |id: &TaskId| {
        app.tree
            .get(id)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| id.to_string())
    };
    if let Some(back) = out.events.iter().find_map(|e| match e {
        CoreEvent::Resurfaced { id } => Some(id.clone()),
        _ => None,
    }) {
        let t = title(&back);
        app.say(
            MessageKind::Success,
            format!("↩ {t} is back: all its subtasks are done"),
        );
        return;
    }
    if let Some(w) = out.warnings.first() {
        app.say(MessageKind::Info, w.clone());
        return;
    }
    for e in &out.events {
        match e {
            CoreEvent::Created { id } => {
                app.say(MessageKind::Success, format!("added {id}"));
                return;
            }
            CoreEvent::StatusChanged { id, from, to } => {
                app.say(
                    MessageKind::Success,
                    format!("{}: {from} → {to}", title(id)),
                );
                return;
            }
            CoreEvent::AutoReopened { id } => {
                app.say(MessageKind::Info, format!("reopened parent {}", title(id)));
                return;
            }
            _ => {}
        }
    }
    if let Some(id) = &out.primary {
        app.say(MessageKind::Info, format!("touched {}", title(id)));
    }
}

fn open_editor(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &Client,
    id: &TaskId,
) -> Result<()> {
    let path = client
        .task(id.as_str())?
        .path
        .context("daemon did not report a path")?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or("vi").to_string();
    let args: Vec<String> = parts.map(str::to_string).collect();

    ratatui::restore();
    let status = std::process::Command::new(&prog)
        .args(&args)
        .arg(&path)
        .status();
    *terminal = ratatui::init();
    terminal.clear()?;

    match status {
        Ok(s) if s.success() => {
            if let Err(e) = client.reload() {
                app.say(MessageKind::Error, format!("reload failed: {e}"));
            } else {
                app.say(MessageKind::Success, "note saved and reloaded");
            }
            refresh(app, client);
        }
        Ok(s) => app.say(MessageKind::Error, format!("{prog} exited with {s}")),
        Err(e) => app.say(MessageKind::Error, format!("could not run {prog}: {e}")),
    }
    Ok(())
}
