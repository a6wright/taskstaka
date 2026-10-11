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
    // At login launchd is starting the daemon too: wait for it rather than
    // race it (the loser exits 0 and launchd would leave its job down). Spawn
    // at most once; if that fails, wait passively instead of respawning.
    let mut spawn_allowed = true;
    loop {
        let found = first_contact(
            || Client::discover(dir).is_some(),
            || {
                if !spawn_allowed {
                    return false;
                }
                spawn_allowed = false;
                match Client::connect(dir, port) {
                    Ok(_) => true,
                    Err(e) => {
                        eprintln!("taskstaka-tray: could not start the daemon: {e:#}");
                        false
                    }
                }
            },
            FIRST_CONTACT_POLLS,
        );
        let Some(client) = (if found { Client::discover(dir) } else { None }) else {
            if !send(Msg::Model(Model::Offline)) {
                return;
            }
            wait_for_daemon(dir);
            continue;
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
        wait_for_daemon(dir);
    }
}

/// How many 250 ms polls to give a daemon someone else is starting.
const FIRST_CONTACT_POLLS: u32 = 24;

/// Poll `discovered` up to `polls` times (250 ms apart); only then `spawn`.
/// Returns whether a daemon should now be there.
fn first_contact(
    mut discovered: impl FnMut() -> bool,
    mut spawn: impl FnMut() -> bool,
    polls: u32,
) -> bool {
    for i in 0..polls {
        if discovered() {
            return true;
        }
        if i + 1 < polls && !cfg!(test) {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    spawn()
}

/// Sleep until a daemon answers for `dir` again. Never starts one.
fn wait_for_daemon(dir: &Path) {
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if Client::discover(dir).is_some() {
            return;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn first_contact_waits_for_a_daemon_someone_else_is_starting() {
        // launchd's daemon answers on the third poll: nothing is spawned.
        let polls = Cell::new(0);
        let spawned = Cell::new(0);
        let found = first_contact(
            || {
                polls.set(polls.get() + 1);
                polls.get() >= 3
            },
            || {
                spawned.set(spawned.get() + 1);
                true
            },
            10,
        );
        assert!(found);
        assert_eq!(spawned.get(), 0);
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn first_contact_spawns_once_when_nobody_answers() {
        let spawned = Cell::new(0);
        let found = first_contact(
            || false,
            || {
                spawned.set(spawned.get() + 1);
                true
            },
            4,
        );
        assert!(found);
        assert_eq!(spawned.get(), 1);
    }

    #[test]
    fn first_contact_gives_up_when_the_spawn_fails_instead_of_retrying() {
        let spawned = Cell::new(0);
        let found = first_contact(
            || false,
            || {
                spawned.set(spawned.get() + 1);
                false
            },
            2,
        );
        assert!(!found);
        assert_eq!(spawned.get(), 1);
    }
}
