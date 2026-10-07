<p align="center"><img src="assets/brand/logo.png" alt="A pyramid of orange cubes with two tumbling off" width="260"></p>

# taskstaka

A "what needs action next" task server, backed by markdown.

Work is a tree: a story spawns subtasks, the subtasks block the story, and every
list-shaped tool loses the parent once you descend into a child. taskstaka stores what
you *said* about each task and computes what *needs action*: a task with open
subtasks is blocked and stays out of your way; when its last subtask closes, the
parent comes back to the top of the list, flagged **resurfaced**. That moment is
the whole product.

```
$ taskstaka done z9v416

↑  Ship auth story  (12atqg) is ready: all its prerequisites are done. It needs action.
z9v416  Write migration: todo → done
```

Every task is a markdown note with YAML frontmatter under `~/todo/notes/`, plus a
generated `~/todo/index.md` tree you can open in Obsidian. Sync the directory with
git, Drive, whatever — the files are yours.

## Install

```
cargo install --path crates/taskstaka
taskstaka init                 # creates ~/todo (or $TASKSTAKA_DIR)
```

## Updating and running

Day to day the daemon starts itself; you rarely think about it.

Rebuild and install after pulling changes:

```
cargo install --path crates/taskstaka
taskstaka restart              # the old daemon keeps running old code until you do this
```

Any command will also warn `the daemon started before this binary was built` until you
restart (silence with `TASKSTAKA_NO_STALE_WARNING=1`).

Run without installing (development):

```
cargo run -p taskstaka -- next
cargo run -p taskstaka -- serve   # foreground daemon; logs to stderr, RUST_LOG=debug for more
cargo test                     # cargo clippy --all-targets before committing
cd e2e && npm install && npx playwright install chromium && npm test   # browser scenario, screenshots in e2e/shots/
```

Where things are:

```
~/todo/notes/*.md      your tasks (TASKSTAKA_DIR moves it)
~/todo/index.md        generated tree — open in Obsidian
~/todo/.taskstaka/        daemon.json (pid, port), lock, nudge log — safe to delete when stopped
```

Daemon controls:

```
taskstaka status | stop | restart
taskstaka serve --port 7407 --quiet 22-8 --no-notify
systemctl --user restart taskstaka                    # if you installed the unit below
launchctl kickstart -k gui/$(id -u)/dev.taskstaka     # macOS
```

If a command says the directory is locked, a daemon is running: drop `--direct`, or stop it.

## Daily use

```
taskstaka next                                  # what do I have to do now? (below)
taskstaka add "Get DNS address" -c work
taskstaka add "Create the zone" -p <id>          # a prerequisite: the story is blocked until it closes (and pauses if it was doing)
taskstaka add "Log in first" -p <id> --start     # push: the prerequisite goes straight in hand
taskstaka start <id> · done <id> · drop <id>
taskstaka put-back <id>                         # out of your hand without finishing it
taskstaka wait <id> Sam                         # waiting on someone; follow-up defaults to +2d
taskstaka chase <id> pinged again               # poked them again; next follow-up gets shorter
taskstaka reply <id> redirected --to Bo "Bo owns DNS"   # answered | declined | partial | redirected
taskstaka unblock <id>                          # they got back to you — it is ready and in hand
taskstaka waiting                               # who am I waiting on, for what, how long?
taskstaka add "TTL question?" -p <id> -w Bo     # create it already waiting on Bo
taskstaka someday <id>                          # park it (doesn't block, doesn't rank)
taskstaka touch <id> pinged Sam                 # bump "last touched", journal a note
taskstaka edit <id>                             # open the note in $EDITOR
taskstaka ls · show <id> · stack · mv <id> <parent>
taskstaka tui                                   # the always-on panel
```

`next` answers one question — *what do I have to do now?* — the same way in the CLI,
the TUI and the web:

```
▶ in hand  e3xv3s  Email platform-eng to approve the zone   started 1h
           so that › Get DNS address
           ≡ draft: hi team, could you approve…
           done e3xv3s · reopen e3xv3s (put back) · add -p e3xv3s --start "first I need to…" · wait e3xv3s WHO

Get DNS address   3 now · 4 later (2 🤖) · ✉ Priya
qk8y9y  ☐ Tell Claude: hostname only, or reachable from the VPC?                 1h
e3xv3s  ☐ Email platform-eng to approve the zone                   ▶ in hand     1h
96qed1  ☐ Log in: gcloud auth login ‹ Create the zone              ↑ ready       1h
        … 4 later: Ask Priya about wildcard certs (✉ Priya · 1h), Create the zone (🤖 Claude · after Log in: gcloud auth login), +2   (--later)
```

The plate on top is the task in hand. Under it, each story is a stack: the rows you can
do right now (independent streams, in the order you wrote them), then `… N later` —
everything still required and *why not yet*. `--later` expands it; `--explain` gives the
reason under each row.

The panel (`taskstaka tui`) has five pages (`1`–`5`, or `v` to cycle) that match the
web's views — the web adds a **Done** page; in the panel, `h` shows completed tasks
in the outline instead: **Next** shows the plate boxed at the top — `d` done, `p` put back, `i` *first I
need to…*, `w` wait on someone — then each story as a stack (`l` unfolds the later list).
**Projects.** A root story can carry a project (`taskstaka add --project devops "…"`,
`taskstaka edit ID --project devops`, or the Project field in the web pane). Its subtasks
inherit it, several roots can share one, and each project gets its own colour, shown as a
pill beside *now*, *in hand* and on Outline roots in the web and the TUI. The agent skill
reuses a matching project or creates one named for the repo.

**Quick add.** One line becomes a new root task: `taskstaka quick "renew the cert #devops
@Priya +3d"` (`@Name [date]` at the end waits on them; `#project` as the first or last word
sets the project). In the web UI press `n` anywhere. For a hotkey from any app, bind your
launcher to `http://127.0.0.1:7407/quick` (a bare one-box page; `?context=personal` picks
the context) or to the command: on macOS a Raycast/Alfred hotkey or a Shortcuts action
"Open URL"; on Linux a desktop keyboard shortcut running
`xdg-open http://127.0.0.1:7407/quick` or `taskstaka quick "$(zenity --entry --text=Add)"`.
taskstaka itself does not grab global keys.

**Outline** is the tree (`⇧` + arrows rearrange, `A` adds under, `h` shows completed),
**Waiting on** groups by person (`f` chase, `u` unblock when they reply), **Someday** is
the parked outline (`u` revives), **Nudges** is the current set. `?` lists every key.

Ids are 6 characters; any unique prefix works. Add `--json` to anything for scripts.

## The model, in short

The full vocabulary is in [`CONTEXT.md`](CONTEXT.md); the decisions behind it are in
`docs/adr/`. The part you need to hold in your head:

**Nesting means "first this."** A subtask is a prerequisite: something that must finish
before its parent can. *To bake an apple pie from scratch you must first invent the
universe* — so "invent the universe" goes under "bake the pie". Break work down from the
end result backwards; the deepest open leaf is what you do first.

**Siblings mean "and also this."** Everything under one parent is independent and can be
in flight at once; the parent finishes when all of them do. Siblings never depend on
each other — if you want to write "after the one above", nest it instead. A prerequisite
two siblings share becomes a sibling placed first.

**You store what you said; taskstaka computes the rest.**

| Stored status | means |
|---|---|
| `todo` / `doing` | not started / started (`doing` is weak — many tasks can be doing) |
| `waiting` | on a person, always with a follow-up date |
| `someday` | parked: doesn't block its parent and doesn't rank, but the parent can't be closed until it is done or dropped |
| `done` / `dropped` | closed |

| Derived fact | shown as | means |
|---|---|---|
| blocked | `▽` | has open prerequisites below it — look down |
| waiting on a person | `✉ Priya · 3d · 2×` | the leaf (or, for a blocked parent, the leaf it depends on) is waiting on someone |
| actionable | `☐` | open, unblocked, not parked, and either not waiting or waiting with its follow-up due — yours to do |
| resurfaced | `↑ ready` | its last prerequisite just closed and you haven't touched it since — *the moment every flat list loses* |
| follow-up | `✉ follow up` | a wait whose date has arrived; the action is to chase |
| in hand | `▶ in hand` | the most recently started task that is open and unblocked — the one thing to go do |
| agent step | `🤖` | Claude's to perform; still listed for you, because you prompt it and check it |
| orphaned | `⚠` | reopened or added under a parent that was already finished; still yours to do |

**The hand.** `start` takes a task in hand; it becomes the top plate with its notes.
`put-back` empties the hand (the next most-recently-started task moves up, or nothing).
`done` on the task in hand starts the next one in the same story: the parent if that was
its last prerequisite, else the next sibling in outline order (ADR 0010). Any prerequisite landing under a `doing` task — added, moved there, or reopened
— **pauses** it back to `todo` with a note, because a task cannot be both doing and
blocked. *First I need to…* (`add -p <in-hand> --start`) is the **push**: the new task
goes in hand and the one above pauses. The hand stays inside the story: it never jumps to
another story on its own. When there is nothing next in the story, the hand is empty and
the parent shows `↑ ready`.

**Waiting is a person, not a status.** A wait always has a follow-up date; `chase` records
that you poked them and shortens the next one; `reply` records what came back
(`redirected --to Bo` hands the wait to Bo); `unblock` means they answered and it is in
hand. A story stuck on a person says `✉ Bo · 4d`, not "blocked by 1".

**Ranking is explainable.** Within what is actionable: overdue → `↑ ready` → doing →
follow-up due → due soon → high → normal → low, then age. Context (`work` /
`personal`) is a filter, never a rank: `next -c work`, or `TASKSTAKA_CONTEXT=work`.

## The daemon

The first command auto-starts `taskstaka serve` in the background; it owns the files
and every client talks to it on `127.0.0.1:7407`. `taskstaka status` / `taskstaka stop`.
For always-on, install a service:

**systemd (Linux)** — `~/.config/systemd/user/taskstaka.service`
```ini
[Unit]
Description=taskstaka daemon
[Service]
ExecStart=%h/.cargo/bin/taskstaka serve
Environment=TASKSTAKA_DIR=%h/todo
Restart=on-failure
[Install]
WantedBy=default.target
```
`systemctl --user enable --now taskstaka`

**launchd (macOS)** — `~/Library/LaunchAgents/dev.taskstaka.plist`
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>dev.taskstaka</string>
  <key>ProgramArguments</key><array><string>/Users/you/.cargo/bin/taskstaka</string><string>serve</string></array>
  <key>EnvironmentVariables</key><dict><key>TASKSTAKA_DIR</key><string>/Users/you/todo</string></dict>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>
```
`launchctl load ~/Library/LaunchAgents/dev.taskstaka.plist`

Nudges (desktop notifications) fire for: stuck in *doing* > 2 days, nothing actionable,
tasks rotting untouched > 14 days, a parent resurfacing, follow-ups due, too much WIP.
`taskstaka serve --quiet 22-8` silences them overnight; `--no-notify` logs only (as an
environment variable it must be `TASKSTAKA_NO_NOTIFY=true`).

To turn the popups off or on without restarting: `taskstaka notifications off` (or `on`; with
no argument it shows the setting), or the switch on the web **Settings** page. Nudges still
appear in the Nudges view. A parent that finishing a task puts straight in your hand (ADR 0010)
does not send a "ready" popup.

**Settings and the notes folder.** Machine-level settings live in
`~/.config/taskstaka/settings.json` (`$XDG_CONFIG_HOME` is honoured; `$TASKSTAKA_CONFIG_DIR`
overrides both). They hold the notifications switch and the notes folder. The folder is
chosen in this order: `--dir`, then `$TASKSTAKA_DIR`, then the saved setting, then `~/todo`.
Change it from the Settings page: the daemon restarts on the new folder, copying your notes
into it if it is empty or new, and leaves the old folder untouched. Choosing a folder that
already holds taskstaka notes uses it as it is. From a terminal the same move is
`taskstaka restart --to <folder>`. See ADR 0011.

## Other clients

- **Web:** `http://127.0.0.1:7407/` — a Linear-style shell: sidebar (All · Work ·
  Personal; Outline, Next, Waiting on, Someday, Done), a Notion-style nested checklist
  in the middle, and a side pane for any task's details. Type a line, `Enter` for the
  next, `Tab` to make it a subtask, `Shift+Tab` to outdent, tick the box to finish (a
  parent's box finishes its subtree), end a line with `@Name` to wait on someone.
  Hover a row for `▶ start` and `open`; `Ctrl+.` starts/stops the focused row; `open` (or `Ctrl+Enter`) slides in the pane: properties, subtasks, notes,
  activity, and the wait / chase / reply actions. `Esc` closes it; `\\` or the chevron on the sidebar's edge collapses the sidebar.
  **Next** answers "what do I have to do now?": the task in hand on top (start one and it
  lands there; tick it and the next one slides up), then each story with the steps you can
  do right now and, folded behind `… 3 later`, everything still required to finish it and
  why not yet — including Claude's steps. Capture a whole stack in one line —
  `Bake an apple pie > Make the filling > Buy apples` — the last segment is what you do
  first. `↳ first` on any row adds a prerequisite under it.
- **Claude Code:** `taskstaka setup claude` installs two things and is safe to re-run after
  every upgrade: the **MCP server** (registered as `taskstaka`, user scope, pointing at the
  installed binary) and the **taskstaka skill** (`~/.claude/skills/taskstaka/SKILL.md`),
  which teaches Claude to turn any multi-step plan into tasks — your manual steps with the
  exact command or draft email in the notes, its own steps tagged `agent` and closed as it
  goes, and anything that depends on a person already *waiting on* them. `--project` installs
  into the current repo instead; `--dry-run` shows the commands. Then ask "what should I work
  on?" or just start a piece of work.
- **Obsidian:** open `~/todo` as a vault. `index.md` is the tree; each note is a task.
  Read-only in v1 — the daemon overwrites `index.md`, and notes it writes win.
- **Hotkey:** bind a key to a one-line prompt that runs `taskstaka add`.
  - macOS, Raycast script command:
    ```bash
    #!/bin/bash
    # @raycast.schemaVersion 1
    # @raycast.title Add todo
    # @raycast.mode silent
    # @raycast.argument1 { "type": "text", "placeholder": "task" }
    TASKSTAKA_DIR="$HOME/todo" "$HOME/.cargo/bin/taskstaka" add "$1"
    ```
  - Linux, sxhkd + rofi: `super + t` →
    `t=$(rofi -dmenu -p 'todo') && [ -n "$t" ] && taskstaka add "$t"`

## Waiting on people

The scenario this exists for: you ask A, A says talk to B, B leaves you on read, and
you forget B is the reason the story is stuck. So:

- a wait **always** has a follow-up date (default two days out);
- `chase` records that you poked them and shortens the next follow-up;
- `waiting` (and the strip at the top of the panel, TUI header, and web pages) lists
  everyone you're waiting on, grouped by person, with how long and how many chases;
- a blocked story says `✉ B · 4d`, not "blocked by 1";
- at 9:00 you get one digest: "Waiting on 2 people: B — DNS address (4d, 2×) · …".

In the web outliner, ending a line with `@Name` creates it waiting on that person
(`ask about TTL @B +3d`).

## Agents

The MCP server gives an agent everything to keep the list honest for you: `waiting`
(with `due=true` for follow-ups that are due), `chase` (idempotent per `dedupe`, tagged
`source="agent"`), `record_reply` (answered / declined / partial / redirected, with a
summary), and `waiting_ref` links back to the conversation. Nothing sends messages;
agents draft, you send.

A daily routine with the taskstaka MCP and a messages MCP (Teams, Slack, mail):

> For each item in `taskstaka.waiting(due=true)`: open `waiting_ref` if set, else search
> messages with the person for the task title since `since`. If they replied, call
> `record_reply` with a one-line summary (`redirected` if they named someone). If silent,
> call `chase` with `dedupe=today` and draft a short follow-up for me to send. End with
> the drafts.

Answered items resurface in `next`; chases move the counts; you get a notification
"B replied on DNS address: …".

## Repo layout

- `crates/core` — `taskstaka-core`: model, frontmatter, tree, store, the needs-action
  computation, nudge rules. Sync, no framework deps, unit-tested with an injected clock.
- `crates/taskstaka` — the binary: CLI, daemon, TUI, MCP bridge, web UI.
- `docs/PLAN.md` — design and phase status. `docs/adr/` — why the non-obvious decisions.
  `CONTEXT.md` — the vocabulary.

```
cargo fmt --all && cargo clippy --all-targets && cargo test
```

## License

[MIT](LICENSE).
