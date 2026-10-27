# Changelog

Releases are on [GitHub](https://github.com/a6wright/taskstaka/releases).

## 0.1.7 · 2026-10-26

- **Done is a day log** — "what did I do yesterday / today?" for standup. The seven days ending today as columns, each day grouped by story with the finished tasks nested as they were in the outline; a task finished on its own shows the `› parent` chain back to its story. <kbd>‹</kbd> <kbd>›</kbd> slide a week, *month* shows a grid with one line per story per day, <kbd>⎘</kbd> copies a day as Markdown, *✗ dropped* includes dropped tasks. The day's people traffic is there too: waits started, chases and replies under the task they happened on (✉ marks a task you only touched).
- `taskstaka log` (`--on yesterday`, `--since -6d`, `--dropped`, `--json`) prints the same outline in the terminal; the MCP server gets a `log` tool for agents writing standup notes. Dates accept `yesterday` and `-Nd`.
- Web: a change arriving while the page was refreshing could leave the list behind the sidebar until the next change; the refresh now notices and reloads.

## 0.1.6 · 2026-10-26

- Notes render as Markdown everywhere they show: the web pane, the plate and under rows (lists, links, `code`, task lists, tables); click the notes to edit the source, click away or <kbd>Esc</kbd> to save. The TUI details pane styles them too. Raw HTML in a note is shown as text.

## 0.1.5 · 2026-10-26

- Web: a search box (<kbd>/</kbd>) that matches titles, notes, who a task waits on and ids, keeping the ancestors of a match for context.
- Web: filter every view to one project or tag from the sidebar's Projects and Tags lists, a project pill or a `#tag` chip. The filters live in the URL.
- Web: fold a row's subtasks with the chevron left of it (`▸ fold` in the head folds every story); folds are remembered.
- Web: rearrange by clicking. A row's grip picks it up, then click the line where it should go, or `↳ nest under` a row. <kbd>Alt+↑</kbd> / <kbd>Alt+↓</kbd> swap siblings. Drag still works.

## 0.1.4 · 2026-10-18

- Linux: the menu bar item runs on GNOME, KDE and any StatusNotifier host, with the same menu and the same quick-add panel. `taskstaka setup linux` writes systemd user units for the daemon and the item, and on GNOME a keyboard shortcut (<kbd>Alt+Space</kbd>) that runs `taskstaka tray quick`, which toggles the panel of the running item. `stop`, `restart`, `status` and `tray start|stop|status` go through `systemctl --user` when a unit owns the process.
- `curl -fsSL https://taskstaka.com/install.sh | sh` installs everything on either system.

## 0.1.3 · 2026-10-11

- `taskstaka tray start`, `stop` and `status` control the menu bar item. With the launchd agent from `setup macos`, `start` brings it back after Quit and `stop` unloads it until login; without the agent they run and end a plain `taskstaka-tray`.
- taskstaka.com deploys from GitHub Actions on each push to main, after fmt, clippy and the tests.

## 0.1.2 · 2026-10-10

- <kbd>⌥Space</kbd> opens a Spotlight-style quick-add panel instead of the browser.
- The menu bar menu shows the task's notes, laid out like Markdown, before its actions.
- The web shell works at half width and on phones.
- `restart` hands an unsupervised daemon back to launchd.

## 0.1.1 · 2026-10-10

- `stop`, `restart` and `status` recognise Homebrew's `brew services` label (`sh.brew.taskstaka`).
- `setup macos` retries `launchctl bootstrap` and keeps symlinked binary paths, so `brew upgrade` leaves the agents working.
- The tray waits for a daemon launchd is starting instead of racing it.

## 0.1.0 · 2026-10-10

- First tagged release: the CLI, the daemon, the TUI, the MCP server, the web outliner, the macOS menu bar item with quick add, `taskstaka setup macos`, and the Homebrew tap.
