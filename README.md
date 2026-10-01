# tododo

A "what needs action next" task server, backed by markdown.

Work is a tree: a story spawns subtasks, the subtasks block the story, and every
list-shaped tool loses the parent once you descend into a child. tododo stores what
you *said* about each task and computes what *needs action*: a task with open
subtasks is blocked and stays out of your way; when its last subtask closes, the
parent comes back to the top of the list, flagged **resurfaced**. That moment is
the whole product.

```
$ tododo done z9v416

↩  Ship auth story  (12atqg) is back: all its subtasks are done. It needs action.
z9v416  Write migration: todo → done
```

Every task is a markdown note with YAML frontmatter under `~/todo/notes/`, plus a
generated `~/todo/index.md` tree you can open in Obsidian. Sync the directory with
git, Drive, whatever — the files are yours.

## Install

```
cargo install --path crates/tododo
tododo init                 # creates ~/todo (or $TODODO_DIR)
```

## Updating and running

Day to day the daemon starts itself; you rarely think about it.

Rebuild and install after pulling changes:

```
cargo install --path crates/tododo
tododo restart              # the old daemon keeps running old code until you do this
```

Any command will also warn `the daemon started before this binary was built` until you
restart (silence with `TODODO_NO_STALE_WARNING=1`).

Run without installing (development):

```
cargo run -p tododo -- next
cargo run -p tododo -- serve   # foreground daemon; logs to stderr, RUST_LOG=debug for more
cargo test                     # cargo clippy --all-targets before committing
cd e2e && npm install && npx playwright install chromium && npm test   # browser scenario, screenshots in e2e/shots/
```

Where things are:

```
~/todo/notes/*.md      your tasks (TODODO_DIR moves it)
~/todo/index.md        generated tree — open in Obsidian
~/todo/.tododo/        daemon.json (pid, port), lock, nudge log — safe to delete when stopped
```

Daemon controls:

```
tododo status | stop | restart
tododo serve --port 7407 --quiet 22-8 --no-notify
systemctl --user restart tododo                    # if you installed the unit below
launchctl kickstart -k gui/$(id -u)/dev.tododo     # macOS
```

If a command says the directory is locked, a daemon is running: drop `--direct`, or stop it.

## Daily use

```
tododo add "Ship auth story" -c work
tododo add "Fix JWT expiry" -p <id>          # subtask; the story is now blocked
tododo next --explain                        # ranked, with a reason per row
tododo start <id> · done <id> · drop <id>
tododo wait <id> Sam                         # waiting on someone; follow-up defaults to +2d
tododo chase <id> pinged again               # poked them again; next follow-up gets shorter
tododo reply <id> redirected --to Bo "Bo owns DNS"   # answered | declined | partial | redirected
tododo waiting                               # who am I waiting on, for what, how long?
tododo add "TTL question?" -p <id> -w Bo     # create it already waiting on Bo
tododo someday <id>                          # park it (doesn't block, doesn't rank)
tododo touch <id> pinged Sam                 # bump "last touched", journal a note
tododo edit <id>                             # open the note in $EDITOR
tododo ls · show <id> · mv <id> <parent>
tododo tui                                   # the always-on panel
```

Ids are 6 characters; any unique prefix works. Add `--json` to anything for scripts.

### Ranking

Actionable tasks are ordered by tier, then age: **overdue → resurfaced → doing →
follow-up due → due soon → high → normal → low**. Context (`work` / `personal`) is
a filter, not a rank: `next -c work`, or `TODODO_CONTEXT=work`.

## The daemon

The first command auto-starts `tododo serve` in the background; it owns the files
and every client talks to it on `127.0.0.1:7407`. `tododo status` / `tododo stop`.
For always-on, install a service:

**systemd (Linux)** — `~/.config/systemd/user/tododo.service`
```ini
[Unit]
Description=tododo daemon
[Service]
ExecStart=%h/.cargo/bin/tododo serve
Environment=TODODO_DIR=%h/todo
Restart=on-failure
[Install]
WantedBy=default.target
```
`systemctl --user enable --now tododo`

**launchd (macOS)** — `~/Library/LaunchAgents/dev.tododo.plist`
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>dev.tododo</string>
  <key>ProgramArguments</key><array><string>/Users/you/.cargo/bin/tododo</string><string>serve</string></array>
  <key>EnvironmentVariables</key><dict><key>TODODO_DIR</key><string>/Users/you/todo</string></dict>
  <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>
```
`launchctl load ~/Library/LaunchAgents/dev.tododo.plist`

Nudges (desktop notifications) fire for: stuck in *doing* > 2 days, nothing actionable,
tasks rotting untouched > 14 days, a parent resurfacing, follow-ups due, too much WIP.
`tododo serve --quiet 22-8` silences them overnight; `--no-notify` logs only.

## Other clients

- **Web:** `http://127.0.0.1:7407/` — a Linear-style shell: sidebar (All · Work ·
  Personal; Outline, Next, Waiting on, Someday, Done), a Notion-style nested checklist
  in the middle, and a side pane for any task's details. Type a line, `Enter` for the
  next, `Tab` to make it a subtask, `Shift+Tab` to outdent, tick the box to finish (a
  parent's box finishes its subtree), end a line with `@Name` to wait on someone.
  Hover a row for `▶ start` and `open`; `Ctrl+.` starts/stops the focused row; `open` (or `Ctrl+Enter`) slides in the pane: properties, subtasks, notes,
  activity, and the wait / chase / reply actions. `Esc` closes it; `\\` or the chevron on the sidebar's edge collapses the sidebar.
- **MCP** (Claude Code, Claude Desktop): `claude mcp add tododo -- tododo mcp`. Then ask
  "what should I work on?" — the server's instructions teach the model the tree rules.
- **Obsidian:** open `~/todo` as a vault. `index.md` is the tree; each note is a task.
  Read-only in v1 — the daemon overwrites `index.md`, and notes it writes win.
- **Hotkey:** bind a key to a one-line prompt that runs `tododo add`.
  - macOS, Raycast script command:
    ```bash
    #!/bin/bash
    # @raycast.schemaVersion 1
    # @raycast.title Add todo
    # @raycast.mode silent
    # @raycast.argument1 { "type": "text", "placeholder": "task" }
    TODODO_DIR="$HOME/todo" "$HOME/.cargo/bin/tododo" add "$1"
    ```
  - Linux, sxhkd + rofi: `super + t` →
    `t=$(rofi -dmenu -p 'todo') && [ -n "$t" ] && tododo add "$t"`

## Waiting on people

The scenario this exists for: you ask A, A says talk to B, B leaves you on read, and
you forget B is the reason the story is stuck. So:

- a wait **always** has a follow-up date (default two days out);
- `chase` records that you poked them and shortens the next follow-up;
- `waiting` (and the strip at the top of the panel, TUI header, and web pages) lists
  everyone you're waiting on, grouped by person, with how long and how many chases;
- a blocked story says `⌛ B · 4d`, not "blocked by 1";
- at 9:00 you get one digest: "Waiting on 2 people: B — DNS address (4d, 2×) · …".

In the web outliner, ending a line with `@Name` creates it waiting on that person
(`ask about TTL @B +3d`).

## Agents

The MCP server gives an agent everything to keep the list honest for you: `waiting`
(with `due=true` for follow-ups that are due), `chase` (idempotent per `dedupe`, tagged
`source="agent"`), `record_reply` (answered / declined / partial / redirected, with a
summary), and `waiting_ref` links back to the conversation. Nothing sends messages;
agents draft, you send.

A daily routine with the tododo MCP and a messages MCP (Teams, Slack, mail):

> For each item in `tododo.waiting(due=true)`: open `waiting_ref` if set, else search
> messages with the person for the task title since `since`. If they replied, call
> `record_reply` with a one-line summary (`redirected` if they named someone). If silent,
> call `chase` with `dedupe=today` and draft a short follow-up for me to send. End with
> the drafts.

Answered items resurface in `next`; chases move the counts; you get a notification
"B replied on DNS address: …".

## Repo layout

- `crates/core` — `tododo-core`: model, frontmatter, tree, store, the needs-action
  computation, nudge rules. Sync, no framework deps, unit-tested with an injected clock.
- `crates/tododo` — the binary: CLI, daemon, TUI, MCP bridge, web UI.
- `docs/PLAN.md` — design and phase status. `docs/adr/` — why the non-obvious decisions.
  `CONTEXT.md` — the vocabulary.

```
cargo fmt --all && cargo clippy --all-targets && cargo test
```
