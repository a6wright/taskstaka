---
status: accepted
date: 2026-10-05
amends: 0009
---

# Finishing the task in hand starts the next one

The user's words: "when a task is completed, its hard to know 'what next' — that big card
should auto start the next parent story, so we can keep burning things down."

ADR 0009 left the hand empty after `done` and rejected auto-climbing to the parent. In
daily use an empty plate after every tick is a decision the user has to make every time,
and the answer is almost always "the next thing in this story".

**Rule.** When the task *in hand* is closed (done or dropped), the store starts the next
task in the same story, in this order:

1. The parent, if closing this task resurfaced it (it was the last blocking child).
2. Otherwise the next open sibling in outline order that is `todo` and actionable. If that
   sibling has prerequisites of its own, descend to its first ready one (the deepest first
   leaf, as Next would show it).
3. Otherwise nothing: the hand stays empty and Next shows what is ready.

The new task goes `todo` → `doing`, `started` is now, and it is journaled
"started: next after X".

**Scope.**

- Only when the closed task *was in hand*. Closing something you never started does not
  move the hand.
- Never leaves the story. It does not jump to the top of Next across stories; choosing a
  different story is still the user's.
- Enforced in `Store::advance_hand`, so CLI, TUI, web and MCP agree.

**Supersedes** the "rejected: auto-climbing" note in 0009 for the resurfaced-parent case.
The objection there (the parent may have other open children) does not apply: the parent
is only started when its last blocking child just closed. Siblings are still never
ranked; outline order is the user's priority order.
