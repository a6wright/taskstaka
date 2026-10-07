# taskstaka

A single person's task tree, and the computation that says which of those tasks needs action right now. Exists because work is a tree — a story spawns subtasks that block it — and every list-shaped tool loses the parent once you descend into a child.

## Language

### Tasks

**Task**:
One thing to do. Has a title, a context, a stored status, and optionally a parent. Every task is a markdown note.
_Avoid_: todo, item, ticket, card

**Subtask**:
A task with a parent — a prerequisite: something that must finish before its parent can. "To bake an apple pie from scratch you must first invent the universe": the universe nests under the pie. A subtask can have subtasks.
_Avoid_: child (fine in code, not in UI), step, dependency

**Siblings**:
Tasks under the same parent. Independent by definition: they can all be in flight at once, and the parent finishes when all of them do. Siblings never depend on each other; order among them is priority, not sequence. A prerequisite two siblings share becomes a sibling placed first.

**Stack**:
The chain of open prerequisites from a leaf up to its story — what finishing the leaf unlocks. Next shows the deepest open leaves; the stack is why each one is there.

**Story**:
Informal name for a root task with subtasks. Not a type; just how people talk about the top of a tree.
_Avoid_: epic

**Project**:
A free-text, coloured label on a root story ("taskstaka", "devops"), inherited by everything under it; several roots may share one. Shown as a pill next to *now*, *in hand* and on Outline roots, so you can tell which piece of work a row belongs to. Not a filter and not a rank, and not a task type: just a label. The colour is chosen when the name first appears and stored on the root.
_Avoid_: theme, epic, workspace

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

**Resurfaced** (shown as **↑ ready**):
An actionable task whose last prerequisite has just closed and which the user has not touched since. The moment the old tools lost. The UI says "ready — prerequisites done" because "back" assumed you had been there before.
_Avoid_: unblocked (that is for people), reopened (that is a status transition)

**Orphaned**:
An open task with a finished ancestor: its parent was ticked off and it was reopened or added afterwards. Still actionable; shown with a hint rather than hidden.
_Avoid_: dangling, stranded, zombie

**Follow-up**:
A waiting task whose `until` date is today or past. Actionable; the action is to chase. Every wait has one; the store sets a default.

**Chase**:
Recording that you poked the person again. Bumps the chase count and shortens the next follow-up. An agent may record a chase on your behalf (tagged), but nobody sends messages through taskstaka.
_Avoid_: ping, nag, remind (that is a nudge)

**Handoff**:
Changing who a waiting task waits on (A said "talk to B"). Resets the wait's clock and chase count; journaled.
_Avoid_: reassign, escalate

**Unblock**:
The person you were waiting on got back to you (or it is otherwise ready): the task leaves `waiting` and goes straight to `doing`, so it is at the top of Next.
_Avoid_: resolve, release

**Reply**:
What came back from the person: answered, declined, partial, or redirected to someone else. Recorded with a one-line summary; answered/declined hand the task back to you.

**Digest**:
The once-a-day nudge listing everyone you are waiting on.

**Conversation link** (`waiting_ref`):
An opaque pointer to where the wait lives (thread URL, message id) so you or an agent can jump back to it.

**Tier**:
The lexicographic rank bucket of an actionable task: overdue, resurfaced, doing, follow-up, due soon, high, normal, low. Explainable by design.
_Avoid_: score, priority (that is a stored field)

**Needs action**:
The ranked list of actionable tasks for a context. What `next` prints and the panel shows.

### The hand

**In hand**:
The task you most recently started (`doing`) that is still open and unblocked. Nothing else is ever in hand; when there is no such task, the hand is empty. Finishing the in-hand task starts the next one in the same story (the parent if it just became ready, else the next sibling); it never moves to another story. The in-hand task is the top plate on Next, with its notes: the one thing to go do.
_Avoid_: current task, focus, active

**Put back**:
Taking the in-hand task out of your hand without finishing it (`doing` → `todo`). The hand moves to the next most-recently-started task, or empties.
_Avoid_: stop, pause (see below), cancel

**Push**:
Adding a prerequisite under the in-hand task because you learned you need it first. The new task starts in hand; the task above it pauses.

**Pause**:
What happens to a `doing` task when a prerequisite lands under it: it goes back to `todo` with a journal line ("paused: first X"). A task cannot be both doing and blocked.

**Plate**:
One task on Next drawn as a card. The in-hand plate on top; per story, the *now* plate (every parallel stream you could work on, in outline order, unranked), the *later* plates (everything still required, nearest-to-doable first, each with why not yet), and the story at the base.

### Time

**Queued**:
How long since a task was created.

**Touched**:
How long since `updated` last moved. Only user-visible edits and an explicit touch move it.
_Avoid_: modified, last activity

**Nudge**:
A rule-generated reminder ("story X is waiting on this", "nothing is actionable"). Best-effort as a desktop notification; reliable in the panel.
_Avoid_: alert, alarm, reminder (fine informally)

### Plans with an agent

**Story**:
(see above) — when Claude helps, the story is the request and each step is a subtask in the order it will happen.

**Agent step**:
A subtask Claude performs itself, tagged `agent`, marked doing/done as it goes. Visible so the human sees what was done for them.

**Manual step**:
A subtask only the user can do (console, command on their machine, approval, sending a message). Untagged; its notes carry exactly what to do so it stands alone tomorrow.
_Avoid_: human task, todo for me

### Storage

**Note**:
The markdown file for one task: YAML frontmatter plus a free-form body. The source of truth.

**Index**:
The generated `index.md` tree. Derived, overwritten on every change; never edited by hand.

**Vault**:
The todo directory as Obsidian sees it. Read-only in v1.
