//! The thread that follows the daemon: connect (starting it if need be),
//! then long-poll `/api/v1/changes` and republish the model after each change.
//! Mirrors the TUI's watcher; the tray never computes anything itself.

use std::path::Path;
use std::time::Duration;

use taskstaka_client::Client;
use taskstaka_core::Context;

use crate::menu::Model;

pub enum Msg {
    /// A daemon answered; `base` is its URL for the menu's links and actions.
    Connected {
        base: String,
    },
    Model(Model),
}

/// Blocks forever; run it on its own thread. `send` returns `false` once the
/// UI is gone, which ends the loop.
pub fn follow(dir: &Path, port: Option<u16>, ctx: Option<Context>, send: impl Fn(Msg) -> bool) {
    loop {
        // First contact starts the daemon like any other client would.
        let client = match Client::connect(dir, port) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("taskstaka-tray: no daemon: {e:#}");
                if !send(Msg::Model(Model::Offline)) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        if !send(Msg::Connected {
            base: client.base().to_string(),
        }) {
            return;
        }
        let mut rev = match publish(&client, ctx, &send) {
            Some(rev) => rev,
            None => return,
        };
        let mut failures = 0;
        loop {
            match client.changes(rev) {
                Ok(new_rev) => {
                    failures = 0;
                    // `!=`: the rev restarts at 1 when the daemon restarts.
                    if new_rev != rev {
                        match publish(&client, ctx, &send) {
                            Some(r) => rev = r,
                            None => return,
                        }
                    }
                }
                Err(_) => {
                    failures += 1;
                    if failures >= 2 {
                        if !send(Msg::Model(Model::Offline)) {
                            return;
                        }
                        // Back to discovery; an explicit `taskstaka stop` is
                        // respected (no auto-start) until the user acts.
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if Client::discover(dir).is_some() {
                break;
            }
        }
    }
}

/// Fetch the plate and the plan, publish the model; returns the rev they
/// were read at, or `None` when the UI has gone.
fn publish(client: &Client, ctx: Option<Context>, send: &impl Fn(Msg) -> bool) -> Option<u64> {
    let rev = client.health().map(|h| h.rev).unwrap_or(0);
    let model = match (client.plate(ctx), client.plan(ctx)) {
        (Ok(plate), Ok(stories)) => Model::online(plate, &stories),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("taskstaka-tray: {e:#}");
            Model::Offline
        }
    };
    send(Msg::Model(model)).then_some(rev)
}
