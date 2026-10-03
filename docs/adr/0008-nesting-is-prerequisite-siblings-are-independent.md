---
status: accepted
date: 2026-10-02
---

# Nesting means "first this"; siblings mean "and also this"

While the agent built the first real plan it wrote "blocked on gcloud login" into a
task's notes, because the tree had no way to say that one step must precede a sibling.
We considered adding an explicit `depends_on` link between siblings (a prototype was
built and reverted) and making sibling order mean sequence. Both were rejected in favour
of reading the tree we already have the way it was always meant:

- **A subtask is a prerequisite.** It goes under the task it unblocks. "To bake an apple
  pie from scratch you must first invent the universe": the universe nests under the pie.
  Depth is a stack of causes; you knock it out from the bottom, and finishing a leaf
  hands you back the thing you were trying to do (resurfacing, ADR 0001).
- **Siblings are independent.** Everything under one parent can run at the same time;
  the parent finishes when all of them do. Siblings never depend on each other — if you
  want to say "after the one above", nest instead.
- **A shared prerequisite becomes a sibling placed first.** Order carries "this one
  first"; we accept losing strict blocking there rather than duplicating the task or
  adding cross-links.

Why not `depends_on`: it would be the first relationship not visible in the outline's
shape, cutting against "structure is the outline", and it was only needed because the
skill had taught a flat "one subtask per step, in order" shape. The fix was the skill.

## Consequences

- No engine change. `blocked`, `actionable` and `resurfaced` already implement this.
- The skill, the `CLAUDE.md` block and the glossary now teach *stack, not list*.
- Next can show the stack each row sits in — what finishing it unlocks — because that
  chain is the reason the row is there.
- Capture can lean on it: typing a prerequisite under a task (Tab) is "first this".
