# folder

A path on this machine holding meeting transcripts or notes (`~/notes/meetings`, an
Obsidian vault, a transcripts export). Read with Glob, Grep and Read; no server involved.

**Window:** files modified since `last run`, else since yesterday 00:00 (`Glob` the folder
recursively for `*.md`, `*.txt`, `*.vtt`, `*.srt`, `*.json`, newest first, then check
mtimes). A file whose name carries yesterday's date counts even when its mtime does not.

**Read by section**, never whole: `Grep` for headings *Action items*, *Next steps*, *TODO*,
*Decisions*, *Follow-ups*, and for lines naming the user, `@name`, "will", "to do",
"owner", "by Friday". A transcript (`.vtt`, `.srt`, speaker-labelled lines) with no such
headings is skimmed for the user's own commitments ("I'll…", "let me…", "I can…") and for
asks aimed at them. More than five files or any file over 2,000 lines → one Explore agent
with this playbook, returning candidates only.

**Candidates:** the user's commitments as `new` (or `update <id>` when a task already says
it); others' commitments the user depends on as `new` `waiting_on` them; an item that a
task already covers and that the notes call done → `done? <id>` for the user to confirm.

**Cite:** `folder · <file name> · <meeting or date from the file> · <path>`.
**Cap:** eight candidates per folder.
