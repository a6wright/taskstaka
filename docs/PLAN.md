# tododo — a "what needs action next" server backed by markdown

## Context

The problem is not "I need a todo list." Work is a tree: a story spawns subtasks, the
subtasks block the story, and every tool tried so far flattens that tree into a list. When
you descend into a subtask the parent falls off the screen and out of your head, and you
never come back to it. Context switching between work and personal compounds it.

The insight from the conversation: **"in progress" is a useless status** — you can have
4000 things in progress and it tells you nothing. What you actually need is a *computed*
**needs-action** set:

- A task with open children is **blocked by them**, not actionable. It should not nag you.
- When its last child closes, the parent becomes actionable again **automatically** and is
  flagged *resurfaced*. You don't have to remember to "pop back"; the system surfaces it
  because nothing blocks it anymore.
- The always-on panel shows the few things that need action *now*, each with its ancestry
  ("this is for story X") and how long it has sat there.

Everything else — TUI, web, MCP, hotkey, Obsidian — is an input/output device for that
one computation.

## Use case (refined)

**Actors:** one person, one machine, two contexts (`work`, `personal`).

**Core loop:**
1. Something lands → capture it in <2s from wherever you are (hotkey→CLI / TUI / MCP / web),
   optionally as a child of whatever you're looking at.
2. Glance at the panel → it shows what needs action next. Not what's "in progress" —
   what is *unblocked and yours to do*, ranked, with a one-line reason.
3. Work on it. Finish it (`done`), or park it (`waiting` on an external thing, with an
   optional follow-up date).
4. If finishing it unblocks a parent, the parent reappears at the top of the panel
   (**resurfaced**) and you get a notification. That's the moment the old tools lost.

**The panel must answer at a glance:**
- What should I do next? (top of the actionable list, and *why* — the tier)
- What is it for? (ancestry: `Ship auth story › Fix JWT expiry`)
- How long has this been sitting? (queued-for, untouched-for)
- What's silently rotting? (old actionable tasks nobody has touched)
- Is everything waiting? (nothing actionable → distinct "go unblock something" state)

**Stored statuses** (user intent): `todo`, `doing`, `waiting` (external; `waiting_on` note,
optional `until` date), `someday` (parked; excluded from ranking), `done`, `dropped`.
**Derived, never stored** (computed facts): `blocked` (has open children), `actionable`,
`resurfaced`.

**Non-goals for v1:** multi-user, sync (files are plain markdown; git/Drive is the user's
business), Obsidian *editing* (read-only view), mobile, recurring tasks, snooze.

## On-disk format

```
todo/
  index.md                 # GENERATED nested checklist tree, rewritten on every mutation
  notes/<id>-<slug>.md     # source of truth, one per task; slug frozen at creation
  .tododo/                 # daemon.json {pid,port}, lock, nudges.json (Obsidian ignores dot-dirs)
```

Per-task file (canonical output; parser tolerates any valid YAML incl. Obsidian rewrites):
```markdown
---
id: k7x2mq                # 6-char Crockford base32, random, collision-checked
title: Fix JWT expiry
parent: a1b2c3            # optional
status: todo
context: work
priority: high            # optional: high | low
created: 2026-09-29T14:02:00Z
updated: 2026-09-29T15:10:00Z
started:                  # omitted when unset (skip_serializing_if)
due: 2026-10-03           # optional civil date
waiting_on: "PR review"   # only with status: waiting
until: 2026-10-01         # optional follow-up date for waiting
tags: [auth]
---
Free-form notes. Obsidian renders this as a normal note.
```

`index.md`: `# Todo` + "generated, do not edit"; `## Work` / `## Personal` nested
checklists with `[[k7x2mq-fix-jwt-expiry|Fix JWT expiry]]` wikilinks and annotations
(`⏳ blocked by 2`, `▶ doing 2d`, `⌛ waiting on Bob until Oct 2`, `due Oct 1`); then
`## Someday`; then `## Done (last 14 days)`.

## Needs-action algorithm (the product)

Definitions, per task `t`, direct children only, `today` in **system tz**:
```
closed(t)     := status ∈ {done, dropped}
blocking(t)   := status ∈ {todo, doing, waiting}      # someday children do NOT block
blocked(t)    := ∃ child c: blocking(c)
deferred(t)   := status = someday ∨ ∃ ancestor with status ∈ {someday, waiting}
followup(t)   := status = waiting ∧ until ≤ today
actionable(t) := ¬closed ∧ ¬deferred ∧ ¬blocked ∧ (status ∈ {todo, doing} ∨ followup)
resurfaced(t) := actionable ∧ has children ∧ all children closed
                 ∧ max(child.completed) > t.updated       # self-clears when user touches t
```
Principle: **block at the leaf.** A story waiting on a client should have a child `waiting`
task, not be `waiting` itself (CLI warns if you `wait` a task with open children).

**Ranking = lexicographic tiers** (explainable; `--explain` shows the tier per row):

| Tier | Predicate | Within |
|---|---|---|
| T0 Overdue | `due < today` | due ↑ |
| T1 Resurfaced | `resurfaced(t)` | longest-ignored first |
| T2 Doing | `status = doing` | started ↑ (finish the oldest) |
| T3 Follow-up | `followup(t)` | until ↑ |
| T4 Due soon | `due ≤ today + 3d` | due ↑ |
| T5 High | `priority = high` | created ↑ |
| T6 Normal | | created ↑ (FIFO — no starvation) |
| T7 Low | `priority = low` | created ↑ |

Tie-break: `updated` ↑ (stalest first). Context is a **filter** (by root's context), not a
rank. `next` defaults to the configured context; `--all` shows both.

**Invariants enforced in `Store::apply`:**
- I1 parent exists and is open on `add`/`mv`; I2 no cycles.
- I3 closing a task with open descendants (incl. `someday`) is refused unless `--cascade`
  (marks them `dropped`). A forgotten someday child surfaces instead of orphaning.
- I4 reopening a child of a closed parent auto-reopens ancestors → `todo`.
- I5 `updated` bumps only on user-visible edits and `touch` — never on derived changes or
  reparent — so "untouched for 14d" stays honest.

## Architecture

**Single writer.** The daemon owns the files. Every client (CLI, TUI, MCP, web) talks
HTTP+JSON to `127.0.0.1:7407`. Clients auto-start the daemon if `/health` fails (read
`.tododo/daemon.json` → spawn `tododo serve --detach` → poll ≤2s; `fd-lock` on
`.tododo/lock` guarantees one daemon wins a race). Recommend a systemd user unit /
launchd plist for always-on.

**Cargo workspace, two packages** (`resolver = "3"`, edition 2024):
```
Cargo.toml                       # [workspace] members = ["crates/*"], [workspace.dependencies]
crates/core/     tododo-core     # lib: sync, pure, NO tokio/axum/ratatui — compiler-enforced
  src/model.rs        Task, TaskId, Status, Context, Priority, TaskView/NextItem/Nudge (wire DTOs)
  src/frontmatter.rs  split/parse/render (serde_norway); round-trip tested
  src/tree.rs         children map, ancestors(), cycle check
  src/command.rs      Command enum + Outcome{primary, affected, events}
  src/store.rs        Store::open(dir), Store::apply(cmd, now), atomic writes, lock, edit guard
  src/action.rs       needs_action(tree, now, ctx, cfg) -> Vec<NextItem>
  src/index_md.rs     render index.md
  src/nudge.rs        nudges(tree, now, cfg, last_fired) -> Vec<Nudge>
crates/tododo/   tododo          # one binary, clap subcommands
  src/main.rs         serve|init|add|start|done|wait|someday|drop|reopen|next|ls|show|mv|edit|touch|tui|mcp|reindex
  src/client.rs       ureq client, discovery, auto-start
  src/server/         axum routes, AppState, notifier, nudge scheduler
  src/tui/            ratatui panel
  src/mcp.rs          rmcp stdio bridge → HTTP
  src/web/ + templates/   askama pages
```
Two packages, not one: a separate lib package makes "core has no async/framework deps"
a compile-time fact. Not five: server/tui/mcp/web all depend on core anyway and would
only add Cargo.toml churn.

**Crates** (versions verified on crates.io 2026-09-29):

| Concern | Crate | Gotchas |
|---|---|---|
| HTTP | `axum 0.8`, `tokio` | path params are `/{id}` in 0.8 |
| MCP | `rmcp 3.5` `["transport-io"]` | `#[tool_router]`/`#[tool_handler]` macros; use re-exported `rmcp::schemars`; logs → **stderr** |
| TUI | `ratatui 0.30` | use `ratatui::crossterm` re-export; `ratatui::init()` installs panic hook |
| YAML | `serde_norway 0.9` | `serde_yaml` unmaintained, `serde_yml` deprecated; write `---` fences yourself; `skip_serializing_if` on optionals |
| Time | `jiff 0.2` `["serde"]` | store `Timestamp` (not `Zoned` — RFC 9557 suffix breaks Obsidian); due = `civil::Date`; "today" via `Zoned::now()` |
| Atomic write | `tempfile 3` | temp in **same dir**, `sync_all`, `persist`, then fsync dir; loader skips dotfiles |
| Notifications | `notify-rust 4` (Linux); `osascript` shell-out (macOS) | behind a `Notifier` trait; call from `spawn_blocking` |
| HTTP client | `ureq 3` (no TLS, `json`) | sync, fast CLI startup |
| CLI / errors / lock / web / tests | `clap 4`, `thiserror`+`anyhow`, `fd-lock 4`, `askama 0.16`, `insta`+`assert_cmd` | |

**Daemon internals:** `AppState { store: Arc<Mutex<Store>>, rev: watch::Sender<u64> }`;
handlers run `store.apply` in `spawn_blocking`. `apply` order: validate in memory →
**external-edit guard** (compare file `(mtime,len)` to what was loaded; on mismatch re-parse
disk and apply on top, warn) → atomic-write affected files → mutate memory → regenerate
`index.md` inline (sub-ms; no debounce race) → bump `rev`. Startup: parse all notes,
**fail loud** on parse errors, regenerate index unconditionally.

**HTTP API** (`/api/v1`; `{id}` accepts unique prefix, 409 on ambiguity):

| | |
|---|---|
| `GET /health` | `{version, rev, todo_dir, pid}` |
| `GET /next?context&limit&explain` | `{rev, items:[NextItem], summary:{actionable, blocked, waiting, doing, someday}}` |
| `GET /tasks?status&context&parent&include_closed` · `GET /tasks/{id}` · `GET /tree?root` | reads |
| `POST /tasks` | `{title, parent?, context?, due?, tags?, priority?, body?}` |
| `PATCH /tasks/{id}` | edit fields/body |
| `POST /tasks/{id}/status` | `{status, waiting_on?, until?, cascade?}` — all transitions; returns `{primary, affected, events}` incl. `resurfaced` |
| `POST /tasks/{id}/move` · `/touch` · `DELETE /tasks/{id}` | |
| `GET /nudges` · `GET /changes?after=<rev>` (long-poll ≤25s) | panel header; TUI refresh |
| `POST /reindex` · `POST /reload` | escape hatches |

Mutations accept JSON bodies only, no CORS headers → browser drive-by can't reach them.

**Nudges** (pure rules in core; daemon evaluates every 5 min + after each mutation;
per-(rule,task) cooldown persisted in `.tododo/nudges.json`; quiet hours in config):

| Rule | Trigger | Cooldown |
|---|---|---|
| R1 stuck_doing | `doing` > 2d — "On X for 3d. Story Y is waiting on it." | 24h/task |
| R2 all_blocked | nothing actionable but open tasks exist — "Go unblock something." | 4h |
| R3 stale_in_queue | actionable, untouched > 14d — "do, drop, or someday it" | 24h |
| R4 resurfaced | event: last blocking child closed — "'P' is back." | immediate |
| R5 follow_up_due | `followup(t)` | 24h/task |
| R6 wip_limit | `count(doing) > 3` — "Pick one." | 24h |

**MCP:** `tododo mcp` is a stdio process that auto-starts the daemon and maps tools 1:1
to HTTP: `next`, `add_task`, `get_task`, `set_status`, `update_task`, `move_task`,
`touch`, `tree`; resources `tododo://index`, `tododo://task/{id}`.

**Hotkey:** out of the binary. macOS: Raycast/Alfred/Hammerspoon script → `tododo add`.
Linux: sxhkd/rofi → `tododo add`. Resolve `TODODO_DIR` at spawn and pass explicitly (hotkey
env is minimal).

## Build phases

Status legend: ✅ done · 🔨 in progress · ⬜ not started. **Update this table when a phase lands.**

| | Phase | Deliverable | Verified by |
|---|---|---|---|
| ✅ | **P1 core + CLI** | `tododo-core` complete; CLI runs `Store` in-process. `init add start done wait someday drop reopen next ls show mv edit touch rm reindex path`. **Usable alone.** | 30 core unit tests (every tier, resurfaced, someday inheritance, followup); frontmatter round-trip incl. Obsidian-rewritten YAML; `insta` snapshots of `index.md` and canonical frontmatter; 3 `assert_cmd` e2e tests in `crates/tododo/tests/cli.rs` |
| ✅ | **P2 daemon** | axum on `127.0.0.1:7407`, all endpoints incl. generic `POST /api/v1/commands`, daemon.json, lock, auto-start from any CLI command, `/changes` long-poll, `serve/stop/status`; CLI → ureq via a `Backend` trait (direct mode kept as `--direct` / `TODODO_DIRECT=1`) | `crates/tododo/tests/daemon.rs` drives the real router in-process on port 0 (resurface flow, error envelope, long-poll wake, second-daemon refusal); `tests/cli.rs` auto-starts a real detached daemon on `TODODO_PORT=0`, reuses it, and stops it |
| ✅ | **P3 TUI** | `tododo tui [-c ctx]`: ranked rows with tier badge, `‹ root` crumb, queued/touched ages; detail pane; `b` toggles a "waiting & blocked" section that says *why* each is stuck; keys `j/k d s w x z o a A t e b c r ? q`; prompts for add/wait/note; `e` suspends to `$EDITOR` and reloads; refreshes on `/changes` long-poll + 30s age tick; header shows DISCONNECTED if the daemon goes away | `tui/app.rs` unit tests (ranking rows, selection survives rebuild, keys → Commands); `tui/ui.rs` `TestBackend` snapshots (`panel_snapshot`, `panel_stuck`, `panel_help`) + empty/all-blocked states |
| ✅ | **P4 nudges** | R1–R6 as pure rules in `core::nudge` (`evaluate`, event-driven `resurfaced`); `NudgeLog` cooldowns persisted to `.tododo/nudges.json`; daemon scheduler every 5 min + 300 ms after each mutation; `Notifier` trait with `DesktopNotifier` (shell-out) and `LogNotifier`; `serve --no-notify`, `--quiet 22-8`; `GET /nudges` returns the current set; TUI header shows the first nudge | `core::nudge` tests: every rule's message, all-blocked, cooldown suppression + persistence across reload, prune, quiet hours wrapping midnight; `tests/daemon.rs` asserts `/nudges` republishes after a mutation |
| ⬜ | **P5 MCP** | `tododo mcp` | `npx @modelcontextprotocol/inspector tododo mcp`; add to Claude Code and ask it "what's next?" |
| ⬜ | **P6 web** | askama pages + forms for the same actions | router request tests |
| | post-v1 | snooze, Obsidian two-way (file watching), streamable-HTTP `/mcp`, recurring | |

### Deviations from the original plan (recorded as they happen)

- **P1:** timestamps are rounded to whole seconds before writing (readable frontmatter, stable diffs); consequently `resurfaced` uses `max(child.completed) ≥ t.updated` rather than `>`.
- **P1:** no `fd-lock` crate; `std::fs::File::try_lock` (stable since Rust 1.89) holds the daemon lock.
- **P1:** `edit <id>` with no flags opens the note in `$EDITOR` and reloads — simpler than a body-only PATCH round-trip for the CLI. The HTTP `PATCH` still carries `body`.
- **P2:** the CLI computes `next`/`ls`/`show` locally from a full `GET /tasks?include_closed=true` snapshot rebuilt into a `Tree`, rather than calling `/next`. One rendering path for direct and daemon modes, and CLI/TUI can never disagree with each other. `/next` exists for MCP, web and curl.
- **P2:** added `GET /api/v1/resolve/{prefix}` and a generic `POST /api/v1/commands` (takes a `Command`); the resource-style endpoints build the same `Command`s internally.
- **P2:** the binary crate is also a library (`crates/tododo/src/lib.rs`) so integration tests can bind the real router in-process.
- **P3:** the TUI always talks to the daemon (no `--direct`): holding the exclusive lock for the panel's lifetime would block every CLI command. Like the CLI it rebuilds a local `Tree` from `GET /tasks` and ranks with core, so the two can never disagree.
- **P3:** added a `b` "waiting & blocked" section not in the original key list — seeing *what* is blocking is the natural follow-up to "nothing actionable".
- **P4:** no `notify-rust`. Desktop notifications shell out to `notify-send` (Linux) and `osascript` (macOS): zero extra dependencies, and it works from an unbundled binary on macOS where the native API needs a signed .app. Failures are logged at debug and ignored — the panel is the reliable surface.
- **P4:** the resurfaced nudge key embeds the firing second (`resurfaced:<id>@<ts>`) so "once per resurfacing" holds even though the rule has no cooldown; `prune` keeps the log bounded.

## Verification (end to end, after P1–P3)

```
tododo init && tododo add "Ship auth story" -c work
tododo add "Fix JWT expiry" --parent <id> && tododo add "Write migration" --parent <id>
tododo next --explain      # story is NOT shown (blocked by 2); both children T6
tododo start <jwt>         # → T2 doing
tododo done <jwt>; tododo done <migration>
tododo next --explain      # story is back at T1 "resurfaced: all subtasks done 0m ago"
tododo tui                 # panel shows the same; R4 notification fired
cat todo/index.md          # tree with ⏳/▶ annotations; open in Obsidian, links resolve
```

## Known risks (accepted)

- Someday children block `done` but not actionability — will annoy once; orphans are worse.
- Frozen filename slugs go stale after renames — `title:` is truth, links keep working.
- Multi-file mutations (cascade, auto-reopen) aren't atomic across files; each file stays
  individually valid. No journal in v1.
- Tier order will feel wrong sometimes — `--explain` from day one so tuning is informed.
