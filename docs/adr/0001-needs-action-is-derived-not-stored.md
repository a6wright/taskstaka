---
status: accepted
date: 2026-09-29
---

# Blocked, actionable and resurfaced are derived, never stored

The user's stated pain is that "in progress" as a status is useless ("I can have 4000 tasks in progress"); what matters is which tasks need action now. We store only what the user *said* about a task (`todo`, `doing`, `waiting`, `someday`, `done`, `dropped`) and compute the facts (`blocked`, `deferred`, `actionable`, `resurfaced`, tier) from the tree on every read.

The alternative — a stored `blocked` or `waiting-on-children` status — was rejected because it turns every child mutation into a parent-file write (write amplification and a crash window where two files disagree), and because stored status should express intent while computed facts belong in the view. The cost is that every client must call the core computation rather than reading a field; that is the point of having a core.

## Consequences

- Closing a child never touches the parent's file. The parent "comes back" purely because the computation now says so.
- `resurfaced` self-clears when the user touches the parent (`updated` moves past the last child's `completed`); no acknowledgement state is needed.
- `index.md` still shows these facts as annotations so the vault is readable without running the daemon.
