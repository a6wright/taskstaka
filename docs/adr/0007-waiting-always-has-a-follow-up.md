---
status: accepted
date: 2026-09-30
---

# A person you are waiting on is a first-class thing, and a wait always has a follow-up

Real scenario: DNS task → asked engineer A → A said talk to B → B left the user on read.
With 45 unread threads the user *will* forget that B is why DNS is stuck. In v1 the
`waiting` status carried an optional `until`; a wait without one was a black hole that no
rule would ever mention. And there was no way to answer "who am I waiting on, for what,
how long?" without opening each blocked story.

Decisions:

1. **Entering `waiting` always sets a follow-up date** (`today + follow_up_days`, default 2)
   unless given. The invariant is enforced in the store, so no client can create a
   forgettable wait.
2. **`chase` is a command, not a note.** It records a timestamp, bumps the count, and sets
   the next follow-up *shorter* (2d → 1d → 1d). Its idempotency key (`dedupe`) and `source`
   tag exist so an agent can run it daily without double-counting and without hiding that
   it was the agent.
3. **`record_reply` closes the loop with a required summary.** Answered/declined hand the
   task back (`todo`); redirected is a handoff to a new person with a fresh clock;
   partial keeps waiting. Every outcome is journaled so the body says *why*.
4. **The wait is derived upward.** A blocked parent reports `waiting_on_summary` (the
   oldest waiting descendant) so every surface can say "⌛ B · 4d" instead of "blocked by 1".
5. **`waiting_on()` is a view of its own**, grouped by person, and R7 sends it as a daily
   digest. That list, not the task tree, is what the user glances at before opening Teams.

Alternatives rejected: a "waiting" *status on the parent* (breaks block-at-the-leaf and
loses which subtask is stuck); a free-text journal only (nothing to rank or digest);
auto-restarting the daemon after `cargo install` (surprising mid-TUI; we warn instead —
see `taskstaka restart`).

## Consequences

- `Task` gains `waiting_since`, `waiting_ref`, `chases`; all cleared on leaving `waiting`.
- Handoff via `edit --waiting-on` or a redirected reply resets `chases` and the clock: the
  new person has not kept you waiting yet.
- `chase`/`record_reply` refuse non-waiting tasks rather than guessing.
- No tool sends messages. Agents draft; the user sends.
