# tracker

Issue and project trackers (Linear, Jira, GitHub issues, Asana: tools like `list_issues`,
`search_issues`, `get_issue`, `list_comments`, `get_notifications`).

**Window:** since `last run`, else yesterday 00:00.
1. issues assigned to the user created or updated in the window → `new` when no task names
   them, `update <id>` when one does (the issue URL becomes the task's `waiting_ref` when
   the task waits on someone in that thread, else a touch note);
2. comments or notifications that ask the user something → `new` ask;
3. issues the user had a task for that were closed by someone else → `done? <id>`, never
   closed silently: the user confirms in round 1;
4. for each person in `waiting`: their comments in the window on anything the user owns
   → `reply <id>`.

A tracker usually maps to one project: the team or project name it uses is the
`project:` line for this source; ask once in the setup round when it is not obvious.

**Cite:** `tracker · <who> · <date> · <key or title> · <url>`.
**Cap:** eight candidates.
