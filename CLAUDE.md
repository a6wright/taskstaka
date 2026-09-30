# tododo

A "what needs action next" task server backed by markdown. Read these before changing anything:

- `docs/PLAN.md` — the approved design and the phase checklist (**status lives there**; update it when a phase lands)
- `CONTEXT.md` — glossary; use these words in code, docs and UI
- `docs/adr/` — why the non-obvious decisions were made; add one when you make another

## Layout

- `crates/core` (`tododo-core`) — sync, framework-free: model, frontmatter, tree, store, `action.rs` (the product), nudges. **No tokio/axum/ratatui here**; that is enforced by it being a separate package.
- `crates/tododo` — the one binary: CLI, daemon, TUI, MCP bridge, web.

## Conventions

- Task ids are 6-char Crockford base32 (`0-9 a-z` minus `i l o u`). Test fixture ids must respect that or `parse()` fails.
- Inject the clock: every core function that needs "now" takes a `&Zoned`; tests use `"2026-09-29T15:00:00[UTC]"`.
- `updated` bumps only on user-visible edits and `touch` (invariant I5). Derived changes never bump it.
- Mutations go through `Command` → `Store::apply` → `Outcome`. Do not write task files any other way.
- `cargo fmt --all && cargo clippy --all-targets && cargo test` must be clean before a commit. Snapshots: `INSTA_UPDATE=always cargo test -p tododo-core` after checking the diff is intended.
- After `cargo install --path crates/tododo`, run `tododo restart` — the running daemon keeps serving the old binary otherwise (README "Updating and running").
- A `waiting` task always has `until`; `chase`/`record_reply` are the only ways to move a wait forward. See ADR 0007.
