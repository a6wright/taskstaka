---
status: accepted
date: 2026-09-30
supersedes-in-part: 0005
---

# Ticking a parent completes its subtree; reopening a child does not reopen the parent

Trying v1, the user found the "add subtask to a task" form model confusing: they think in
nested checklists — type a line, `Tab` to make it a subtask, tick a box to finish. The
web UI's default view is now an outliner over the same tree. That changes two rules:

1. **Cascade closes with the same status.** Ticking a parent with open subtasks marks
   them `done` too (v1 marked them `dropped`). In an outliner, "this whole thing is
   finished" is what a parent checkbox means. `drop --cascade` still drops.
2. **No auto-reopen.** Un-ticking a subtask under a finished parent leaves the parent
   finished; the subtask becomes **orphaned** — a derived fact, not a status — and stays
   actionable with the hint "its parent is already finished". v1 reopened the whole
   ancestor chain, which surprised the user: reopening a leftover should not un-finish
   the story.

ADR 0005's rule that someday children do not block but do block *closing* stands; the
outliner's checkbox passes `cascade`, so in practice a parked subtask gets completed
along with its parent rather than blocking the tick.

## Consequences

- Sibling order is now a stored field (`order`, fractional) because an outliner needs
  "insert here", and `Add`/`Move` take `after`. Tasks without it sort by creation time.
- `Event::AutoReopened` is gone; `CascadeClosed { status }` replaces `CascadeDropped`.
- `index.md` annotates orphans with "⚠ parent finished" so the vault view stays honest.
