---
status: accepted
date: 2026-10-06
---

# Machine settings live outside the notes, and the notes folder is changeable

The user's words: "we should have a settings page added, add the notifications switch
there, and we should configure where the notes directory is stored."

**Where settings live.** `~/.config/taskstaka/settings.json` (`$XDG_CONFIG_HOME`, or
`$TASKSTAKA_CONFIG_DIR` for tests). Not inside the notes folder: the folder's own location
cannot be stored inside it, and a popup preference belongs to this machine, not to notes
that may be synced to another one. This replaces the short-lived
`<notes>/.taskstaka/settings.json` from the notifications switch (it never held a value
anyone had set).

**Order for the folder:** `--dir`, `$TASKSTAKA_DIR`, the saved setting, `~/todo`. The flag
and the environment variable still win, so the launchd/systemd units in the README keep
working; Settings shows the saved folder and warns when it differs from the running one.

**Changing the folder is a restart, not a hot swap.** The daemon holds one folder under a
single-writer lock (ADR 0003). So the web page (or `taskstaka restart --to <folder>`):

1. validates the target before stopping anything: absolute (or `~`), not a file, not inside
   or containing the current folder, and either empty or new, or already a taskstaka folder;
2. hands off to a detached `restart --to` helper that stops the daemon, so the copy is
   final and no edit can land mid-copy;
3. copies the notes (and the reminder cooldown log) into an empty or new folder, or uses an
   existing taskstaka folder as it is;
4. saves the setting and starts a daemon on the new folder; if the copy fails it starts the
   old one again.

**The old folder is never modified or deleted.** Moving is a copy; cleaning up is the
user's. **Rejected:** moving (deleting the source) — an unrecoverable step to hang off a
text field; hot-swapping the store inside a running daemon — the lock, the long-polls and
the nudge log would all have to be rebuilt in place.
