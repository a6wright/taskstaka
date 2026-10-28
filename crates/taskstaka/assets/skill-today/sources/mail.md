# mail

Servers whose tools search or read threads and messages (`search`, `list_threads`,
`get_message`, `get_thread`, `drafts`).

**Window:** since `last run`, else yesterday 00:00. Two searches, newest first:
1. received: unread or unanswered threads where the user is addressed directly (a group
   alias or a review-requested-from-team where anyone could answer is not a candidate);
2. sent: the user's own messages in the window — a sent message to a person a task waits
   on is a `chase <id>` candidate; one that asks a new question of someone is a `new`
   candidate already `waiting_on` them.

Open a thread only to settle a candidate: did the user already reply in it (then it is not
an ask of them), did the waited-on person answer (then `reply <id>` with their outcome in
one line: answered, declined, partial, redirected to whom).

**Candidates:** `reply <id>` and `chase <id>` first (they are applied silently in
reconcile), then `new` asks of the user, then `update <id>` for a thread about an existing
task with no `waiting_ref` yet (the thread link becomes the ref).

**Cite:** `mail · <sender> · <date> · <subject> · <thread link>`.
**Cap:** eight candidates; read snippets, open at most eight threads.
