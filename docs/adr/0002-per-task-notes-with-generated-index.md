---
status: accepted
date: 2026-09-29
---

# One markdown note per task is the source of truth; index.md is generated

Tasks live as `notes/<id>-<slug>.md` files with YAML frontmatter; `index.md` is a nested checklist regenerated on every mutation. We chose this over a single nested-checklist file (the Obsidian Tasks style) because per-task metadata — timestamps, parent, waiting reason — does not fit on a checklist line, and because free-form notes per task need somewhere to live. The index exists so the vault still reads as a tree in Obsidian and elsewhere.

In v1 the vault is **read-only from Obsidian**: no file watching, no merge. All mutations go through the store. We kept a cheap guard anyway — the store records each file's `(mtime, len)` at load and re-reads a note before writing it if that changed — because the user will edit a body in Obsidian within a week regardless, and clobbering it silently is the worst outcome.

## Consequences

- The filename slug is frozen at creation so Obsidian `[[wikilinks]]` keep resolving after a rename; `title:` in frontmatter is the truth.
- Only `*.md` files that do not start with `.` are loaded; atomic writes leave `.tmp-*` files behind after a crash.
- Load fails loudly on any unparsable note. Single user; fix it by hand rather than silently skipping data.
- Two-way Obsidian editing is a post-v1 feature, not a bug.
