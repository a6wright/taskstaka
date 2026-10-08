---
status: accepted
date: 2026-10-03
---

# In hand: the most recently started, open, unblocked task — or nothing

The user's words: "that is going to be the best thing we have created so far, it's where
to whole-ass instead of thrashing." The plate at the top of Next showing one task with
its notes and three buttons is the product's answer to context switching. It only works
if what is in hand is unambiguous.

**Definition.** In hand = the task you most recently started (`doing`) that is still open
and not blocked. If no task qualifies, the hand is empty. The hand never climbs on its
own: finishing the in-hand task makes its parent *ready* (a now row, badged ↑), but the
parent is not in hand until you start it.

**Consequences enforced in the store, so every client agrees:**

- *Put back* (`doing` → `todo`) and *wait* (on a person) both take a task out of the hand.
- *Push* — adding a prerequisite under the in-hand task — starts the new task in hand and
  **pauses** the task above it (`doing` → `todo`, journaled "paused: first X"). A task can
  not be both doing and blocked; this applies whether the prerequisite arrives from the
  plate, the outline (Tab), the CLI or an agent.
- Siblings in a story's *now* plate are parallel streams in outline order with no ranking
  among them: choosing is the user's, not the tool's. Tiers provide badges (overdue,
  ↑ ready, due soon). *Amended by ADR 0013:* stories keep their outline order too;
  tiers no longer order them.

**Rejected:** auto-climbing the hand to the parent on completion (surprising when the
parent has other open children, and it presumes the user wants to continue the same
story); keeping a blocked task `doing` (incoherent; it hid the new prerequisite).

**Wording:** the resurfaced badge reads "↑ ready — prerequisites done" rather than
"↩ back", because "back" presumed the user had been on that task before, which is false
for a step they have never touched.
