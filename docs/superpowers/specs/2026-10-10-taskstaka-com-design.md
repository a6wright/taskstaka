# taskstaka.com — design

**Status:** approved in chat on 2026-10-10 (pages, look, tech), with one addition from the
user: "fun diagrams that you create in SVGs for how stacks work, with animations".

## Purpose

A help site at https://taskstaka.com: what taskstaka is, how to install it (as plainly as
brew.sh), and how to use it. It is also the seed of the paid sync service: the same
Cloud Run service grows sync endpoints later, so the site is a Rust binary from day one,
not a static bucket.

Audience: a person who has heard of it and wants it running in two minutes, then comes
back for "how do I…". Voice words: plain, quick, orange.

## Pages

- **Home `/`** — the logo, one line ("A 'what needs action next' task server backed by
  markdown"), the install command in a copyable block front and centre, a screenshot of
  the Next view, three short reasons (tree not list; the plate; waiting on people), the
  animated stack diagram.
- **Install `/install`** — Homebrew (`brew install a6wright/taskstaka/taskstaka`,
  `taskstaka init`), from source (`cargo install --path crates/taskstaka`), macOS menu bar
  and `⌥Space` (`cargo install --path crates/tray`, `taskstaka setup macos`), Claude Code
  (`taskstaka setup claude`), always-on daemon (`brew services` / launchd / systemd),
  updating. Each as a heading with one block; no prose walls.
- **Help `/help`** — the model first: a task is one thing; nesting means *first this*;
  siblings mean *and also this*; a task with open subtasks is blocked and comes back
  **resurfaced**; **Next**, the **plate** (in hand / up next), **waiting on** with
  follow-ups and chases, someday. Then one section per surface: CLI (the dozen commands
  with one line each), web (outline, Next, pane, quick add `n`), TUI (views 1–5, keys),
  menu bar (title, menu, `⌥Space`), agents (MCP tools and the skill). Anchors for every
  heading so the app can deep-link later.
- **Changelog `/changelog`** — one entry per GitHub release, from `site/content/changelog.md`
  (kept by hand at release time; a later action can regenerate it).
- `/healthz` → `ok`. 404 page in the same shell.

Content lives in `site/content/{index,install,help,changelog}.md`, written from the README
and CONTEXT.md, and is the single source for the pages. Screenshots live in `site/assets/`,
captured from a scratch daemon with demo data by `site/shots.js` (the e2e harness), at
1280 wide, dark theme, so they match the palette.

## Diagrams (SVG, inline, animated)

Three diagrams, each an inline `<svg>` with its own `<style>` so it animates without
JavaScript, and a `prefers-reduced-motion` rule that shows a readable still (the stack shows its initial frame, which explains the stack better than the emptied one):

1. **A story is a tree.** A story box spawns subtasks below it; the story dims (blocked)
   while they are open. Loops.
2. **The stack.** Plates stacked like the app's Next view: the bottom plate ("first I need
   to…") is ticked, slides out, and the one above it moves down into hand. Loops through
   three plates, then the story itself lights up.
3. **Resurfaced.** The last subtask closes and the parent jumps back to the top of Next
   with the ↑ ready badge.

Style: the app's row and plate styling (rounded 10px plates, `--line` borders, the warn
yellow for *doing*, the orange accent for the moment of resurfacing), Inter labels,
500–700 ms moves with an ease-out curve, pauses between beats so it reads at a glance.

## Look

Identity-preserving: the app's `app.css` tokens (dark-first, `--bg #0f1012`, `--line`,
`--muted`, Inter) with one change: the accent is the logo's orange (sampled from
`assets/brand/mark.png`, roughly `#f07a1f`), used for the install block's border, links,
the resurfaced moment, and nothing else. Light scheme follows `prefers-color-scheme` like
the app. Type: Inter 400/500/600; headings on a 1.25 scale with `clamp()` for the home
line only; body 16–17px at 68ch. Layout: one column, 760px content, a sticky top bar
(mark + name, Install, Help, Changelog, GitHub). Phone-first; the same `@media` structure
as the app. No cards, no eyebrows, no stock photos: the logo, the screenshots and the
diagrams are the imagery.

## Tech

- `crates/site` → binary `taskstaka-site`: axum (workspace dep), `pulldown-cmark` (new
  workspace dep) renders the Markdown at startup into pages wrapped in a hand-built shell
  (same approach as the daemon's `web.rs`), served from memory with `cache-control`
  (`public, max-age=300` for pages, a year for hashed assets is not needed yet — assets
  get `max-age=86400`). Listens on `$PORT` (Cloud Run) or 8080. Logs to stdout via
  `tracing`.
- Assets embedded with `include_bytes!`: `site.css`, `logo.png`, `mark.png`,
  `favicon.ico`, `apple-touch-icon.png`, screenshots.
- `crates/site/Dockerfile`: multi-stage, `rust:1-bookworm` builder (`cargo build --release
  -p taskstaka-site --locked`), `debian:bookworm-slim` runtime, non-root, `PORT=8080`.
  Built from the repo root (the workspace), `.dockerignore` keeps `target/`, `e2e/node_modules`
  and the notes out.
- Deploy: `gcloud run deploy taskstaka-site --source . --region us-central1 --project
  taskstaka --allow-unauthenticated --min-instances 0 --cpu 1 --memory 256Mi`. Custom
  domain: Cloud Run domain mapping for `taskstaka.com` and `www.taskstaka.com`; the DNS
  records it returns are the user's step. A GitHub Action deploying on push to `main` is
  a follow-up once Workload Identity is set up; the first deploys are from this machine.

## Non-goals (now)

Sync, accounts, analytics, a blog, search, i18n, prebuilt installers.

## Testing

- `crates/site` unit tests: Markdown → page HTML (headings get ids, code blocks keep
  their text, the shell wraps every page, 404 for unknown paths, `/healthz`).
- Integration test driving the axum router in-process (same pattern as `tests/web.rs`).
- Headless screenshots at 1280 / 720 / 390 of every page, read by eye, before deploy.
- `docker build` locally; `curl` the container; then the Cloud Run URL; then the domain.
