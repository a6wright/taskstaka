# docs

Document stores (Drive, Docs, Notion, Confluence: tools like `search_files`,
`list_recent_files`, `read_file_content`, `get_document`, `list_comments`).

**Window:** since `last run`, else yesterday 00:00.
1. documents shared with the user or edited by others in the window whose title or first
   lines name a meeting from the calendar pull, "notes", "minutes", "action items",
   "retro", "1:1" → open, read only the sections headed *Action items*, *Next steps*,
   *TODO*, *Decisions*, and lines that name the user or say "will", "to do", "owner";
2. comments that mention the user and are unresolved → `new` ask;
3. a document a task links to that changed → `update <id>` as a touch note.

An action item names an owner: the user's are `new` candidates (or `update <id>` when a
task already says it); someone else's that the user needs are `new` candidates
`waiting_on` that person. Items with no owner are offered as the user's and said so.

**Cite:** `docs · <author or meeting> · <date> · <title> · <link>`.
**Cap:** eight candidates; at most four documents opened, read by section.
