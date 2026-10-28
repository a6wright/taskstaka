# `today.md` — this machine's sources

Lives at `~/.config/taskstaka/today.md` (`$TASKSTAKA_CONFIG_DIR`, else
`$XDG_CONFIG_HOME/taskstaka`, else `~/.config/taskstaka` — the same folder as taskstaka's
`settings.json`). Written by the skill, readable and editable by the user. It holds sources,
roles, projects and dates: nothing else.

```markdown
# today — sources on this machine
servers seen: claude_ai_Gmail, claude_ai_Google_Calendar, claude_ai_Linear, taskstaka
last run: 2026-10-27

## read · Google Calendar — calendar
tools: mcp__claude_ai_Google_Calendar__*
## read · Linear — tracker
look for: issues assigned to me updated in the window, comments that ask me something
project: taskstaka
## read · ~/notes/meetings — folder
look for: files modified in the window; "Action items", "Next steps", lines naming me
tags: meeting
## unconnected · Gmail — mail (sign in to read asks and replies)
## skip · Google Drive — said no 2026-10-27
```

- `servers seen`: the `<server>` part of every `mcp__<server>__…` tool name in the session,
  sorted, taskstaka included. A different set on a later run triggers the setup round for
  the difference only; the lines already there are kept.
- `last run`: the date of the last completed hand-off; the next pull starts there (three
  days at most).
- One `##` per server or folder: `read | skip | unconnected · <name> — <role>`. The role
  names the playbook (`calendar`, `mail`, `chat`, `tracker`, `docs`, `folder`, `generic`).
- Optional lines under it: `tools:` (a glob over tool names when the server's name is not
  enough), `look for:` (what counts as a candidate here, in the user's words), `project:`
  (the project every root story from this source gets), `tags:` (tags every task from this
  source gets, without the `#`).
- A user-edited line wins over anything the skill would infer; rewrite the file whole only
  in the setup round, otherwise change `last run`, `servers seen` and `project:` lines in
  place.
