# Changelog

Releases are on [GitHub](https://github.com/a6wright/taskstaka/releases).

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
