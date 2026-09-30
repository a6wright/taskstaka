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

## Daily use

```
tododo add "Ship auth story" -c work
tododo add "Fix JWT expiry" -p <id>          # subtask; the story is now blocked
tododo next --explain                        # ranked, with a reason per row
tododo start <id> · done <id> · drop <id>
tododo wait <id> --until +3d Sam's review    # waiting on someone; follow-up date
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

- **Web:** `http://127.0.0.1:7407/` — the same panel and actions as plain HTML forms.
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

## Repo layout

- `crates/core` — `tododo-core`: model, frontmatter, tree, store, the needs-action
  computation, nudge rules. Sync, no framework deps, unit-tested with an injected clock.
- `crates/tododo` — the binary: CLI, daemon, TUI, MCP bridge, web UI.
- `docs/PLAN.md` — design and phase status. `docs/adr/` — why the non-obvious decisions.
  `CONTEXT.md` — the vocabulary.

```
cargo fmt --all && cargo clippy --all-targets && cargo test
```
