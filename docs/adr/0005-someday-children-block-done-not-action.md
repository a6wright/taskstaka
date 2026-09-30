---
status: accepted
date: 2026-09-29
---

# A someday subtask does not block its parent's actionability, but does block closing it

`someday` means "deliberately parked", so a parked subtask must not hide its parent from the panel — otherwise parking anything under a story makes the story vanish. But closing a story while a parked subtask still hangs off it would orphan that subtask under a done parent, where nobody will ever see it again.

We chose the asymmetric rule: someday children are excluded from `blocked`, and included in the "open descendants" check that refuses `done`/`drop` without `--cascade`. The alternative (someday children ignored for both) leads to orphans; the other alternative (someday children block both) makes `someday` useless inside a tree.

## Consequences

- Marking a story done can fail because of a subtask you forgot you parked. That is the feature: the forgotten thing surfaces at the moment you are deciding what the story's outcome was.
- The related rule "block at the leaf": a *waiting* or *someday* ancestor defers its whole subtree. A story that is waiting on a client should have a `waiting` subtask, not be `waiting` itself; the CLI warns when you `wait` a task with open subtasks.
