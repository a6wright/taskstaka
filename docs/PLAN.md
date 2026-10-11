# taskstaka — a "what needs action next" server backed by markdown

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
  .taskstaka/                 # daemon.json {pid,port}, lock, nudges.json (Obsidian ignores dot-dirs)
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
(`▽ blocked by 2`, `▶ doing 2d`, `✉ waiting on Bob until Oct 2`, `due Oct 1`); then
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
  (closes them with the same status — ADR 0006). A forgotten someday child surfaces instead of orphaning.
- I4 reopening a child of a closed parent does **not** reopen the parent: the child is
  *orphaned* (derived, shown with ⚠) and stays actionable — ADR 0006. A reopened or revived
  prerequisite under a `doing` parent pauses it (ADR 0009).
- I5 `updated` bumps only on user-visible edits and `touch` — never on derived changes or
  reparent — so "untouched for 14d" stays honest.

## Architecture

**Single writer.** The daemon owns the files. Every client (CLI, TUI, MCP, web) talks
HTTP+JSON to `127.0.0.1:7407`. Clients auto-start the daemon if `/health` fails (read
`.taskstaka/daemon.json` → spawn `taskstaka serve --detach` → poll ≤2s; `fd-lock` on
`.taskstaka/lock` guarantees one daemon wins a race). Recommend a systemd user unit /
launchd plist for always-on.

**Cargo workspace, two packages** (`resolver = "3"`, edition 2024):
```
Cargo.toml                       # [workspace] members = ["crates/*"], [workspace.dependencies]
crates/core/     taskstaka-core     # lib: sync, pure, NO tokio/axum/ratatui — compiler-enforced
  src/model.rs        Task, TaskId, Status, Context, Priority, TaskView/NextItem/Nudge (wire DTOs)
  src/frontmatter.rs  split/parse/render (serde_norway); round-trip tested
  src/tree.rs         children map, ancestors(), cycle check
  src/command.rs      Command enum + Outcome{primary, affected, events}
  src/store.rs        Store::open(dir), Store::apply(cmd, now), atomic writes, lock, edit guard
  src/action.rs       needs_action(tree, now, ctx, cfg) -> Vec<NextItem>
  src/index_md.rs     render index.md
  src/nudge.rs        nudges(tree, now, cfg, last_fired) -> Vec<Nudge>
crates/taskstaka/   taskstaka          # one binary, clap subcommands
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
per-(rule,task) cooldown persisted in `.taskstaka/nudges.json`; quiet hours in config):

| Rule | Trigger | Cooldown |
|---|---|---|
| R1 stuck_doing | `doing` > 2d — "On X for 3d. Story Y is waiting on it." | 24h/task |
| R2 all_blocked | nothing actionable but open tasks exist — "Go unblock something." | 4h |
| R3 stale_in_queue | actionable, untouched > 14d — "do, drop, or someday it" | 24h |
| R4 resurfaced | event: last blocking child closed — "'P' is back." | immediate |
| R5 follow_up_due | `followup(t)` | 24h/task |
| R6 wip_limit | `count(doing) > 3` — "Pick one." | 24h |

**MCP:** `taskstaka mcp` is a stdio process that auto-starts the daemon and maps tools 1:1
to HTTP: `next`, `add_task`, `get_task`, `set_status`, `update_task`, `move_task`,
`touch`, `tree`; resources `taskstaka://index`, `taskstaka://task/{id}`.

**Tray and hotkey (ADR 0015):** out of the *daemon*, in `taskstaka-tray` (`crates/tray`), a
separate OS-facing process that only speaks HTTP: plate title in the menu bar, a menu of *now*
rows, and a global hotkey (`alt+space`) that opens `/quick` in the browser. Launcher recipes
(Raycast/Alfred/sxhkd → `taskstaka add`) remain the no-tray alternative.

## Build phases

Status legend: ✅ done · 🔨 in progress · ⬜ not started. **Update this table when a phase lands.**

| | Phase | Deliverable | Verified by |
|---|---|---|---|
| ✅ | **P1 core + CLI** | `taskstaka-core` complete; CLI runs `Store` in-process. `init add start done wait someday drop reopen next ls show mv edit touch rm reindex path`. **Usable alone.** | 30 core unit tests (every tier, resurfaced, someday inheritance, followup); frontmatter round-trip incl. Obsidian-rewritten YAML; `insta` snapshots of `index.md` and canonical frontmatter; 3 `assert_cmd` e2e tests in `crates/taskstaka/tests/cli.rs` |
| ✅ | **P2 daemon** | axum on `127.0.0.1:7407`, all endpoints incl. generic `POST /api/v1/commands`, daemon.json, lock, auto-start from any CLI command, `/changes` long-poll, `serve/stop/status`; CLI → ureq via a `Backend` trait (direct mode kept as `--direct` / `TASKSTAKA_DIRECT=1`) | `crates/taskstaka/tests/daemon.rs` drives the real router in-process on port 0 (resurface flow, error envelope, long-poll wake, second-daemon refusal); `tests/cli.rs` auto-starts a real detached daemon on `TASKSTAKA_PORT=0`, reuses it, and stops it |
| ✅ | **P3 TUI** (superseded by P14) | `taskstaka tui [-c ctx]`: ranked rows with tier badge, `‹ root` crumb, queued/touched ages; detail pane; `b` toggles a "waiting & blocked" section that says *why* each is stuck; keys `j/k d s w x z o a A t e b c r ? q`; prompts for add/wait/note; `e` suspends to `$EDITOR` and reloads; refreshes on `/changes` long-poll + 30s age tick; header shows DISCONNECTED if the daemon goes away | `tui/app.rs` unit tests (ranking rows, selection survives rebuild, keys → Commands); `tui/ui.rs` `TestBackend` snapshots (`panel_snapshot`, `panel_stuck`, `panel_help`) + empty/all-blocked states |
| ✅ | **P4 nudges** | R1–R6 as pure rules in `core::nudge` (`evaluate`, event-driven `resurfaced`); `NudgeLog` cooldowns persisted to `.taskstaka/nudges.json`; daemon scheduler every 5 min + 300 ms after each mutation; `Notifier` trait with `DesktopNotifier` (shell-out) and `LogNotifier`; `serve --no-notify`, `--quiet 22-8`; `GET /nudges` returns the current set; TUI header shows the first nudge | `core::nudge` tests: every rule's message, all-blocked, cooldown suppression + persistence across reload, prune, quiet hours wrapping midnight; `tests/daemon.rs` asserts `/nudges` republishes after a mutation |
| ✅ | **P5 MCP** | `taskstaka mcp`: rmcp 3.5 stdio server proxying to the daemon (auto-starts it). Tools `next get_task list_tasks tree add_task set_status update_task move_task touch nudges`; resources `taskstaka://index`, `taskstaka://next`; server instructions explain the model (block at the leaf, resurfaced). Logs to stderr. | `crates/taskstaka/tests/mcp.rs` speaks raw newline-delimited JSON-RPC to the spawned binary: initialize, tools/list, add → next (blocked) → set_status (resurfaced event) → resources/read, and error mapping. To register: `claude mcp add taskstaka -- taskstaka mcp` |
| ✅ | **P6 web** | Served by the daemon at `http://127.0.0.1:7407/`: panel page (summary, nudges, quick-add, ranked rows with status buttons, toggleable waiting & blocked section, context pills) and task page (ancestry, facts, subtasks with buttons, wait/touch/add-subtask forms, full edit form incl. body and reparent). Plain HTML + one CSS file, dark-mode aware, phone-width layout. Form posts gated on `Sec-Fetch-Site`/`Origin`. | `crates/taskstaka/tests/web.rs` against the in-process router: pages render, escaping, add → subtask → blocked, refused close surfaces as flash, edit form round-trip, cross-site/cross-origin posts → 403, 404 for unknown ids |
| ✅ | **P7 outliner** (post-v1, from first use) | Web `/` is now an outliner: nested checklist over the tree, `Enter` new line, `Tab`/`Shift+Tab` indent/outdent, checkbox completes the subtree, `⌫` on an empty line deletes, live via long-poll. The ranked panel moved to `/next`. Core: sibling `order` + `after` on Add/Move, cascade-close with same status, orphans instead of auto-reopen (ADR 0006). | core tests for ordering, cascade-done and orphans; web test asserts the outline shell + `/next`; JS syntax-checked with node. **Interaction not yet exercised in a real browser** — first manual pass pending. |
| ✅ | **P8 waiting on people** (from the DNS scenario, ADR 0007) | Waiting always has a follow-up (default +2d); `chase` escalates (2d→1d), idempotent per `dedupe`, `source`-tagged; `reply answered/declined/partial/redirected` with a required summary; handoff via `edit --waiting-on`; `waiting_ref` conversation link; blocked parents report `waiting_on_summary` ("✉ B · 4d"); `waiting` view grouped by person in CLI/TUI header/web strip; R5 mentions chase count, R7 daily digest; outline `@Name [+3d]` quick-capture; MCP tools `waiting chase record_reply`, agent-shaped output and instructions; `taskstaka restart` + stale-daemon warning | core tests (default until, chase escalation + dedupe, handoff, replies, grouping, parent summary, digest once/day); `tests/cli.rs` DNS scenario end to end; `tests/mcp.rs` agent loop (waiting → chase ×2 dedupes → redirected → answered); web strip/forms in `tests/web.rs` |
| ✅ | **P9 web shell** (Linear/Notion-inspired; the reference screenshots are not kept in the repo) | One single-page shell served for every page route: sidebar with contexts + views (Outline, Next, Waiting on, Someday, Done, nudges), Notion-quiet rows (checkbox · title · one state chip; age/priority/`open` on hover), Linear-style side pane replacing `/t/{id}` (title, properties, actions, wait/chase/reply, subtasks, notes, activity from the journal). Dark-first, Inter with system fallback. Assets live in `crates/taskstaka/assets/` and are `include_str!`-ed; the form endpoints are gone — the page uses the JSON API only. | `tests/web.rs`: every page route serves the shell, assets have correct types, the API endpoints the first paint needs answer, form bodies are refused (415), old `/w/*` is 404; `node --check` on the script; **`e2e/run.js` drives headless Chromium (Playwright) against a real daemon**: type/Tab/Enter, `@B`, blocked-parent chip, hover start, pane open/done/Esc, sidebar chevron + `\\`, Waiting/Next views, parent checkbox cascade — screenshots in `e2e/shots/`. |
| ✅ | **P10 Claude integration** | `taskstaka setup claude`: installs the bundled skill (`assets/skill/SKILL.md` → `~/.claude/skills/taskstaka/`, or `.claude/skills/` with `--project`) and (re)registers the MCP server via `claude mcp add --scope user --transport stdio taskstaka -- <exe> mcp` (remove-then-add so an upgraded binary path is picked up). The skill teaches the plan habit: story → one subtask per step, `agent`-tagged steps closed as Claude goes, manual steps with exact instructions in the body, person-dependent steps created `waiting_on`. MCP instructions carry the same convention; the web shows `🤖 agent` and `#tag` chips. | `setup.rs` tests: frontmatter + tool names, writes under `$HOME`, idempotent; manual: `taskstaka setup claude --dry-run` |
| ✅ | **P11 the stack** (ADR 0008) | Nesting = prerequisite, siblings = independent, taught in the skill/CLAUDE.md/glossary. **Stack view** (web `/stack`, CLI `taskstaka stack`): the task in hand on top with its notes and only the actions that move it; plates beneath are the chain (then… so that…), with 'also needs' when a parent has other open children; ticking shifts up, hands off at an agent plate. Next rows show their chain. Capture: `A > B > C` builds a stack; `↳ first` on a row and 'first I need to…' in the pane/stack add a prerequisite. | e2e: chain capture depths, `↳ first`, start → plates → tick shifts up; `render::stack` CLI |
| ✅ | **P12 Next = the stack** | Stack and Next merged: `core::action::plan` groups Next by story — `now` (ranked leaves) and `later` (everything else still open, nearest-to-doable first, each with *why not yet*: after X / ✉ person / 🤖 Claude), stuck stories listed with their reason. `GET /api/v1/plan`, MCP `plan` (the skill starts there), CLI `next` prints stories with `… N later` (`--later` expands), web Next shows the in-hand plate on top and folds each story's later list. Agent steps are counted and listed (a person still has to prompt/check them). | core `plan_tests` (DNS + passport fixture: now/later order and reasons, stuck); e2e: start → in hand, unlocks line, fold/expand, tick shifts up |
| ✅ | **P13 in hand** (ADR 0009) | In hand = most recently started, open, unblocked, else nothing. Put back / wait empty the hand; push starts the new prerequisite in hand and pauses the task above (store-enforced, journaled). Plate carries editable notes. Now rows are unranked parallel streams in outline order. Resurfaced reads "↑ ready". Nudge set changes wake long-polls. Definitions live in `CONTEXT.md` (The hand). | core `in_hand_tests`; e2e: push from plate, pause, done/put back move the hand, ready badge |
| ✅ | **P14 TUI = the same model** | `taskstaka tui` mirrors the web: `1` Next (the in-hand plate boxed on top with its notes and `d done · p put back · i first I need to… · w wait`; per story the *now* rows, a folded `… N later` line (`l`/Enter unfold, each with *why not yet*), the story as the base `▔▔ so that`), `2` Outline (☐ ▶ ▽ ✉ 💤 ✓ marks, `↑ ready`, `🤖 Claude`, ⇧arrows move, `h` completed), `3` Waiting on grouped by person (`f` chase, `u` unblock), `4` Someday as an outline of parked stories + parked leaves under live stories (`u` revive, `a` parks), `5` Nudges. `i` from the plate creates the prerequisite already `doing` so the store takes it in hand and pauses the task above. Widths are display cells, so emoji align. | `tui/app.rs` tests (plate + stacks + fold, push from plate → `Add{status: doing}`, views and their rows, prompts); `tui/ui.rs` `TestBackend` snapshots `panel_next`, `panel_next_expanded`, `panel_outline`, `panel_waiting`, `panel_someday`, `panel_help`; empty/stuck states |
| ✅ | **P15 the hand advances** (ADR 0010, amends 0009) | Closing the task in hand starts the next in the same story: the parent if it just resurfaced, else the next open sibling in outline order (descending to its first ready prerequisite), else the hand stays empty. Never crosses stories. Store-enforced in `advance_hand`, journaled "started: next after X". | core `store` tests (sibling then parent, only when it was in hand, descends to prerequisite); `tests/cli.rs` story ends up in hand |
| ✅ | **P16 notifications setting** | `.taskstaka/settings.json` (`core::settings`), read at delivery time so it applies without a restart; `GET/PUT /api/v1/settings`, `taskstaka notifications [on\|off]`, 🔔 switch in the web sidebar. No resurfaced popup when auto-advance already put the parent in hand. Test daemons run with `TASKSTAKA_NO_NOTIFY=true` so `cargo test` never pops real notifications. | core `settings` tests (default on, round trip, broken file); `tests/cli.rs` setting persists |
| ✅ | **P17 settings page** (ADR 0011) | Web `/settings`: the notifications switch (moved from the sidebar) and the notes folder. Machine settings in `~/.config/taskstaka/settings.json` (`core::settings`, `$TASKSTAKA_CONFIG_DIR` override); folder order `--dir`, `$TASKSTAKA_DIR`, setting, `~/todo`. `taskstaka restart --to <folder>`: validate, stop, copy into an empty or new folder (or use an existing taskstaka folder), save, start; old folder untouched. `PUT /api/v1/settings {dir}` hands off to a detached helper and the page waits for the new daemon. Open tabs now resync after any restart (rev compared with `!==`). | core `settings` tests; `notes_dir` tests (copy, existing, refusals, unchanged); `tests/cli.rs` restart-to moves and refuses; browser run against a sandbox daemon |
| ✅ | **P18 calmer TUI** | The Next view's box becomes the TUI's grammar: a rounded box per story on Next (the story line is set into the bottom border; a one-task story drops the duplicate), per root story on Outline, per person on Waiting on, per section on Someday; 2-column margins, a spacer under the header, a heavier border for the task in hand, a boxed detail pane, a footer that drops low-priority keys to fit the width and always keeps `? help · q quit`, the details box hidden until Space opens it, and a list that scrolls by whole boxes with `▲ N more above` / `▼ N more below` markers (never a half box, never the footer squeezed out). Quieter rows: no `blocked by N` chip (the ▽ says it), age only from 3 days, a count instead of a peek on `▸ N later`, adjacent one-task stories share one `single tasks` box, a header that omits zero counts. Rows, selection and keys are unchanged: boxes are drawn around row ranges (`groups`, `row_line` in `tui/ui.rs`). `dump_real` (ignored test) renders a copy of real notes for a look. | `tui/ui.rs` snapshots re-recorded after reading the frames; `box_edges_line_up` checks every border is the same width in all four views at 60 and 100 columns, with emoji and a selected row |
| ✅ | **P19 projects** | A free-text, coloured `project` on root stories, inherited by everything under them and shareable across roots (ADR 0012). Shown as a pill beside *now*, *in hand* and on Outline roots in the web, and in the Next box border, the in-hand plate and Outline roots in the TUI. Colours are unique per project and stored on the root. CLI `--project`/`--no-project`, MCP `project` and a `projects` tool, and the taskstaka skill reuses or creates one per repo. | `project.rs` and store tests (inherit, share, case, subtask refused, move, reopen); TUI colour test; checked in headless Chrome |
| ✅ | **P20 quick add** | One line becomes a new root task (`quick.rs`: `@Name [date]` waits on them, `#project` first/last word). `taskstaka quick`, `POST /api/v1/quick`, a bare `/quick` page for any launcher or small window, and `n` in the web UI. The global key moved to the tray in P22 (launcher recipes stay as the alternative). | parser unit tests; CLI test; web test of `/quick` and the endpoint |
| ✅ | **P21 macOS pass** (2026-10-10) | Runs on macOS 27: `cargo install`, `init`, daemon auto-start, web, osascript notifications. `GET /api/v1/plate` serves the plate (ADR 0014) for HTTP-only clients. The HTTP client moved to `crates/client` (`taskstaka-client`); `daemon_exe()` lets other binaries start the daemon; `process_alive` uses `kill -0` off Linux. `service.rs`: `stop`/`restart`/`status` detect a launchd agent that owns the daemon (`dev.taskstaka`, Homebrew's `sh.brew.taskstaka` / `homebrew.mxcl.taskstaka`, matched by pid or by the default folder) and `kickstart` it instead of spawning a second one. | `tests/daemon.rs` plate test (empty → up next → in hand → put back prefers never started); `service.rs` unit tests with a fake launchctl; `status --json` carries `supervisor`; manual: stop stays down, restart kickstarts under launchd |
| ✅ | **P22 tray + hotkey** (ADR 0015) | `crates/tray` → `taskstaka-tray`: tao event loop (Accessory policy, no Dock icon), tray-icon status item with the plate title (`▶` in hand, `○` up next, `taskstaka · offline`), muda menu from a pure `MenuSpec` (Done / Put back / Start, up to 10 *now* rows → `/t/{id}`, Quick add, Open, Quit), global-hotkey `alt+space` → `/quick` in the browser; follows `/api/v1/changes` like the TUI, auto-starts the daemon on launch, waits after an explicit stop. Linux gets a placeholder icon (AppIndicator), not built here. | `menu.rs`, `actions.rs`, `hotkey.rs` unit tests (14); manual on macOS 27: bare binary shows the item, title follows start/add/put back/done/stop within a second |
| ✅ | **P23 setup macos** | `taskstaka setup macos [--no-tray] [--hotkey K] [--uninstall] [--dry-run]`: writes `dev.taskstaka` and `dev.taskstaka.tray` LaunchAgents (absolute binary paths, `RunAtLoad`, `KeepAlive {SuccessfulExit:false}`, `PATH`, logs in `~/Library/Logs`, no `TASKSTAKA_DIR`), `bootout` + `bootstrap gui/<uid>`; rerun after `cargo install`. README launchd section rewritten around it. | `setup_macos.rs` unit tests (plist content, escaping, missing tray → note, dry run, uninstall) with a fake launchctl; `tests/cli.rs` dry run; manual: both agents running, logs written |
| ✅ | **P24 release + Homebrew tap** | Repo public; tags `v0.1.0` and `v0.1.1` (the latter adds Homebrew 7 service detection) with GitHub releases; tap `a6wright/homebrew-taskstaka` with `Formula/taskstaka.rb`: source tarball + sha256, `depends_on "rust" => :build`, installs `taskstaka` and (macOS) `taskstaka-tray` from the workspace, `service` block running `taskstaka serve` with `keep_alive crashed: true` (so `taskstaka stop` sticks; label `sh.brew.taskstaka` is recognised by `service.rs`), caveats pointing at `brew services start` / `taskstaka setup macos` / `setup claude`. | `brew install --build-from-source a6wright/taskstaka/taskstaka`, `brew test`, `brew audit --strict`; `brew services start` → `taskstaka status` names the Homebrew supervisor |
| | post-v1 | see the candidate list under **Resume here** | |

### Deviations from the original plan (recorded as they happen)

- **P1:** timestamps are rounded to whole seconds before writing (readable frontmatter, stable diffs); consequently `resurfaced` uses `max(child.completed) ≥ t.updated` rather than `>`.
- **P1:** no `fd-lock` crate; `std::fs::File::try_lock` (stable since Rust 1.89) holds the daemon lock.
- **P1:** `edit <id>` with no flags opens the note in `$EDITOR` and reloads — simpler than a body-only PATCH round-trip for the CLI. The HTTP `PATCH` still carries `body`.
- **P2:** the CLI computes `next`/`ls`/`show` locally from a full `GET /tasks?include_closed=true` snapshot rebuilt into a `Tree`, rather than calling `/next`. One rendering path for direct and daemon modes, and CLI/TUI can never disagree with each other. `/next` exists for MCP, web and curl.
- **P2:** added `GET /api/v1/resolve/{prefix}` and a generic `POST /api/v1/commands` (takes a `Command`); the resource-style endpoints build the same `Command`s internally.
- **P2:** the binary crate is also a library (`crates/taskstaka/src/lib.rs`) so integration tests can bind the real router in-process.
- **P3:** the TUI always talks to the daemon (no `--direct`): holding the exclusive lock for the panel's lifetime would block every CLI command. Like the CLI it rebuilds a local `Tree` from `GET /tasks` and ranks with core, so the two can never disagree.
- **P3:** added a `b` "waiting & blocked" section not in the original key list — seeing *what* is blocking is the natural follow-up to "nothing actionable".
- **P4:** no `notify-rust`. Desktop notifications shell out to `notify-send` (Linux) and `osascript` (macOS): zero extra dependencies, and it works from an unbundled binary on macOS where the native API needs a signed .app. Failures are logged at debug and ignored — the panel is the reliable surface.
- **P4:** the resurfaced nudge key embeds the firing second (`resurfaced:<id>@<ts>`) so "once per resurfacing" holds even though the rule has no cooldown; `prune` keeps the log bounded.
- **P5:** tools return pretty-printed JSON as text content rather than rmcp's structured `Json<T>` output, because that wrapper requires `JsonSchema` on every core type and the text is what an LLM reads anyway. Input structs do derive `JsonSchema` (via `rmcp::schemars`) so tool schemas are accurate.
- **P5:** failures surface as JSON-RPC errors (`-32602` for not-found/validation/conflict, `-32603` otherwise), not as `isError` tool results. rmcp's `Result<_, ErrorData>` return maps that way by default; clients see the message either way.
- **P6:** no askama. Two pages and a handful of forms are hand-built over an `esc()` helper in `server/web.rs`; a template engine was more machinery than it saved and would have added a build-time dependency. Revisit if the UI grows past a few pages.
- **P6:** HTML forms are the one browser-reachable mutation path, which the "JSON-only mutations" defence in ADR 0003 did not cover. Form handlers refuse requests whose `Sec-Fetch-Site` is not `same-origin`/`none` or whose `Origin` does not match `Host`. Flash messages travel in `?msg=`; `back` targets are restricted to local paths.
- **P9:** no form endpoints; the page is a single shell over the JSON API. Every re-render preserves the focused row/caret (the first Playwright run caught the live-update re-render stealing focus from the new-task box). Sidebar collapses via a chevron on its boundary or `\\`; `▶ start` lives on row hover and `Ctrl+.` toggles doing on a focused row.
- **P13:** agent-tagged steps rank for the human after all (reversing the P8-era rule): a person still triggers the agent and checks the result, so hiding them hid work. The 🤖 chip is the signal.
- **P13:** the sidebar no longer collapses (the toggle broke the layout and nobody wanted it); *nudges* is a page.
- **P13 (2026-10-08, ADR 0014):** the plate is never empty while something is ready. With nothing in hand it shows *up next*: the first ready row in Next order never started, else the first ready row (`action::plate`). Computed, not stored; the plate offers *start* instead of *put back*. Web, TUI and CLI share the rule.
- **P12/P13 (2026-10-07, ADR 0013):** stories on Next are in outline order, not ranked by their best row. Ranking them made the boxes swap places when the row carrying the rank was ticked. Tiers still badge rows and still order the flat `next` list.
- **P6:** the daemon still binds `127.0.0.1` only, so "phone on the LAN" needs a `--bind` option — deferred to post-v1 alongside an auth token.

## Resume here

**P1–P24 complete** (2026-10-10). `cargo fmt`, `clippy --all-targets`, `cargo test` (163 tests)
are clean at head. taskstaka runs on macOS 27: the daemon and the menu bar item
(`taskstaka-tray`, ADR 0015) are launchd agents installed by `taskstaka setup macos`, `⌥Space`
opens quick add, and `stop`/`restart`/`status` go through launchd when it owns the daemon.
The HTTP client is its own crate (`crates/client`); `GET /api/v1/plate` serves the plate.
Homebrew: `brew install a6wright/taskstaka/taskstaka` from the tap `a6wright/homebrew-taskstaka`
(source build; formula at `v0.1.1`).

**Next, roughly in order of pain:**

1. **Linux tray** — build `crates/tray` on Ubuntu (`libgtk-3-dev libappindicator3-dev`), a real
   icon instead of the placeholder square, systemd user units from a `setup linux`, and
   systemd awareness in `service.rs`. Wayland has no global hotkey yet (global-hotkey is X11
   only); fall back to the launcher recipes there.
2. **Deferred from the 2026-10-10 review** (all small): `setup macos --no-daemon` for brew
   users (today a `dev.taskstaka` agent is installed beside `sh.brew.taskstaka` and one
   exits at login); check the notes folder is initialised before loading the daemon agent;
   `restart` under launchd ignores `--port` silently; `open_page` runs discovery on the UI
   thread with no visible failure; `Effect::Open` should carry a path, not a URL;
   `default_dir()` is duplicated in the tray and belongs in `taskstaka-client`; unused
   `anyhow`/`serde` in the tray manifest; uid from `HOME`'s owner; the
   `status_json_reports_the_supervisor` test shells out to the real `launchctl`; ADR 0015's
   frontmatter says `amends: 0003` while the body says 0003 holds.
3. **Prebuilt bottles** (`cargo-dist` → tap) once someone other than the author installs it.
4. **Native quick-add popover** in the tray (`wry`) if the browser tab for `⌥Space` grates.

Candidate features after that (the one list; the phase table's *post-v1* row points
here), roughly by how much they change daily use:

- **Hot-key add → agent breakdown**: a one-line capture that hands the story to Claude to
  shape as a stack (skill first, then UI).
- **Snooze** (`snoozed_until`), **Obsidian two-way** (file watching), **`--bind` + auth
  token** for the phone, **recurring tasks**, streamable-HTTP `/mcp`.
- **Business-day follow-ups**; **Teams/Slack read integration** (detecting "left on
  read" — the agent recipe in the README is the manual version).
- **R6 WIP limit tuning** (still hard-coded to 3 in `core::nudge`); a **Ctrl+K palette** in
  the web shell; TUI **search** and **multi-select**.

When picking one up: update the phase table above (add a row), record deviations as
they happen, and add an ADR if the decision is hard to reverse and would surprise a
future reader.

## Verification (end to end, smoke test)

```
taskstaka init && taskstaka add "Ship auth story" -c work
taskstaka add "Fix JWT expiry" -p <story> && taskstaka add "Write migration" -p <story>
taskstaka next                # one story, "2 now · 1 later"; both children as ☐ rows; the story is the later one
taskstaka start <jwt>         # → "todo → doing"; next shows the ▶ in hand plate for it
taskstaka add "Read the RFC" -p <jwt> --start   # push: the new task is in hand, jwt "paused: first Read the RFC"
taskstaka done <rfc>          # "↑ Fix JWT expiry is ready: all its prerequisites are done"
taskstaka done <jwt>; taskstaka done <migration>
taskstaka next                # the story is a now row with ↑ ready; nothing later
taskstaka tui                 # 1 Next shows the same; 2 Outline shows ✓ with h
cat todo/index.md             # tree with ▽/▶/✉ annotations; open in Obsidian, links resolve
```

## Known risks (accepted)

- Someday children block `done` but not actionability — will annoy once; orphans are worse.
- Frozen filename slugs go stale after renames — `title:` is truth, links keep working.
- Multi-file mutations (cascade close, pause) aren't atomic across files; each file stays
  individually valid, and every status change is journaled into the task's own note, so a
  half-applied cascade is visible rather than silent.
- Tier order will feel wrong sometimes — `--explain` from day one so tuning is informed.
