---
status: accepted
date: 2026-10-08
amends: 0009
---

# 0014: The plate is never empty while something is ready

The user's words: "there should never be a time where next is empty, the big box at the
top of next... we should just pick the first available thing if nothing is started... so
putting things back just grabs another thing that is ready to go."

## Context

ADR 0009 defined *in hand* as the most recently started open, unblocked task, "or nothing".
ADR 0010 filled the hand after `done` within the same story. Two gaps remained: *put back*
left the plate empty, and so did finishing the last task of a story. An empty plate hands
the user a decision the tool could have made: Next already lists what is ready, in the
order they wrote it.

## Decision

- **In hand is unchanged** (ADR 0009): the most recently started `doing` task that is open
  and unblocked. `start`, `put back`, `push` and the store's hand advancing (ADR 0010) keep
  their meaning; nothing is auto-started.
- **The plate** (`core::action::plate`) is what the top of Next shows: the task in hand,
  else **up next** — the first ready row in Next order (stories and rows in outline order,
  ADR 0013), preferring rows that were **never started** (`started` unset) over rows that
  were started once and put back. Putting a task back therefore brings up a different one;
  once every ready row has been put back once, the round starts again from the top.
- Up next is computed, not stored. Its plate reads "up next", not "in hand", and offers
  *start* in place of *put back*. Done, *first I need to…* and *wait* work as on the in-hand
  plate; a push from an up-next plate starts the prerequisite in hand, as from the in-hand
  plate.
- Web, TUI and CLI (`next`, `stack` with no id) all use the same rule; the row badge reads
  `▶ up next`.
- The plate is empty only when nothing is ready at all, and Next then says so.

## Consequences

`started` gains a second reading: set on a `todo` task, it means "picked up once and put
back", which demotes it in the up-next order. Reopening a finished task keeps `started` and
so counts as put back. Only rows Next shows can be up next: a `doing` task under a waiting
or parked ancestor is neither in hand nor up next.
