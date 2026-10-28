# chat

Servers for direct messages and channels (Slack, Teams, Discord, Google Chat: tools like
`search_messages`, `list_channels`, `get_thread`, `read_dm`).

**Window:** since `last run`, else yesterday 00:00.
1. mentions and DMs to the user ending in a question or a request they have not answered
   or reacted to;
2. the user's own messages to a person a task waits on (`chase <id>`), or asking a new
   question of someone (`new`, already `waiting_on` them, the permalink as `waiting_ref`);
3. for each person in `waiting`: one search of messages from them since the wait began →
   `reply <id>` with the outcome.

Open a thread once to settle it; a reply or an emoji reaction from the user closes the ask.

**Candidates:** `reply` and `chase` first, then asks of the user as `new`, then
`update <id>` with the permalink where a task about the thread exists and has no ref.

**Cite:** `chat · <person> · <channel or DM> · <date time> · <permalink>`.
**Cap:** eight candidates; snippets first, at most eight threads opened.
