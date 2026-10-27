# Changelog

Releases are on [GitHub](https://github.com/a6wright/taskstaka/releases).

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
