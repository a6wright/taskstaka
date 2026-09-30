# tododo

A single person's task tree, and the computation that says which of those tasks needs action right now. Exists because work is a tree — a story spawns subtasks that block it — and every list-shaped tool loses the parent once you descend into a child.

## Language

### Tasks

**Task**:
One thing to do. Has a title, a context, a stored status, and optionally a parent. Every task is a markdown note.
_Avoid_: todo, item, ticket, card

**Subtask**:
A task with a parent. Nothing else distinguishes it; a subtask can have subtasks.
_Avoid_: child (fine in code, not in UI), step

**Story**:
Informal name for a root task with subtasks. Not a type; just how people talk about the top of a tree.
_Avoid_: epic, project

**Context**:
Which life a task belongs to: `work` or `personal`. A filter, never a rank. Determined by the root of the tree.
_Avoid_: area, category, workspace

### Status (stored — what the user said)

**Todo**:
Not started.

**Doing**:
The user has started it. Records `started`. Deliberately weak: it does not gate actionability and many tasks can be doing.
_Avoid_: in progress, active, WIP

**Waiting**:
Blocked on something outside the system, named in `waiting_on`, optionally with an `until` date to chase it.
_Avoid_: blocked (that word is reserved for the derived fact), on hold, pending

**Someday**:
Deliberately parked. Excluded from ranking. Does not block its parent, but does prevent the parent from being closed.
_Avoid_: backlog, icebox, snoozed

**Done** / **Dropped**:
Closed, either finished or consciously abandoned. Both record `completed`.
_Avoid_: complete, cancelled, archived

### Facts (derived — never stored)

**Blocked**:
A task with at least one direct subtask in a blocking status (todo, doing or waiting). A blocked task is not actionable and should not nag.
_Avoid_: waiting on children, parent status

**Deferred**:
A task inside a subtree whose root-ward ancestor is someday or waiting. The whole subtree is hidden from the panel.

**Actionable**:
Open, not deferred, not blocked, and either todo/doing or a waiting task whose follow-up date has arrived. The set of actionable tasks is the product.
_Avoid_: available, ready, next (that is the command)

**Resurfaced**:
An actionable task whose last subtask has just closed and which the user has not touched since. The moment the old tools lost.
_Avoid_: unblocked, reopened (that is a status transition)

**Orphaned**:
An open task with a finished ancestor: its parent was ticked off and it was reopened or added afterwards. Still actionable; shown with a hint rather than hidden.
_Avoid_: dangling, stranded, zombie

**Follow-up**:
A waiting task whose `until` date is today or past. Actionable; the action is to chase.

**Tier**:
The lexicographic rank bucket of an actionable task: overdue, resurfaced, doing, follow-up, due soon, high, normal, low. Explainable by design.
_Avoid_: score, priority (that is a stored field)

**Needs action**:
The ranked list of actionable tasks for a context. What `next` prints and the panel shows.

### Time

**Queued**:
How long since a task was created.

**Touched**:
How long since `updated` last moved. Only user-visible edits and an explicit touch move it.
_Avoid_: modified, last activity

**Nudge**:
A rule-generated reminder ("story X is waiting on this", "nothing is actionable"). Best-effort as a desktop notification; reliable in the panel.
_Avoid_: alert, alarm, reminder (fine informally)

### Storage

**Note**:
The markdown file for one task: YAML frontmatter plus a free-form body. The source of truth.

**Index**:
The generated `index.md` tree. Derived, overwritten on every change; never edited by hand.

**Vault**:
The todo directory as Obsidian sees it. Read-only in v1.
