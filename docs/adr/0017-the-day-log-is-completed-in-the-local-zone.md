---
status: accepted
date: 2026-10-26
---

# 0017: The day log is `completed` seen in the local zone, drawn as the outline it came from

The user's words: "we need a mode that shows like a calendar of each task done on each
day … so we can say 'what did I do yesterday' and 'what did I do today'", and, on the first
draft: "how are we maintaining which part of the tree an individual task corresponds to?
… the done flat list makes it hard to know what was what."

## Decision

1. **The day a task was finished is its `completed` timestamp seen in the local zone of
   the surface showing it.** Core takes the zone from the injected `&Zoned` (the daemon's
   system zone for the CLI and MCP); the web buckets in the browser. On one machine they
   agree; there is no timezone setting.
2. **There is no event log.** `completed` is cleared on reopen and set again on the next
   close, so a task closed twice counts once, on its latest close. Closing writes no
   journal line. This is accepted: the day log answers "what did I finish", not "what
   happened to this task".
3. **A day is never a flat list.** Every surface groups the day by story and keeps outline
   order: a parent closed by a cascade precedes its subtasks (`depth` nests them), and a
   leaf finished alone carries the ancestors between it and the story as a dimmed chain
   (`chain`). The month grid counts per story, not per task. The data shape
   (`DayLog → StoryLog → DayLogItem{depth, chain}`) is the same in core, the API, the CLI
   text, the MCP JSON and the browser's own bucketing.
4. **The web's week is the seven days ending on the anchor** (today by default), not a
   Monday-aligned week, so yesterday is always in view — on a Monday too. The month grid is
   calendar-aligned.

## Consequences

- `Done` in the web is the day log; the flat "last two weeks" list is gone. The sidebar
  count stays "closed in the last 14 days".
- `taskstaka log` and the MCP `log` tool share `action::day_log`; the browser has a
  twin (`dayLogFor`) because it already holds every task and the search and filters must
  apply. Keep the two in step.
- A closed note edited by hand without `completed` has no day and is not shown anywhere
  in the log.
- If a history of closes is ever wanted (reopened and finished again on a later day), a
  journal line on close is the path; it was left out to keep I5 and the notes quiet.
