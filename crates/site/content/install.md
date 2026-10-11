# Install

## Homebrew

macOS and Linux. It builds from source, so the first install takes a few minutes.

```
brew install a6wright/taskstaka/taskstaka
taskstaka init                 # creates ~/todo (or $TASKSTAKA_DIR)
```

`taskstaka add "Renew the cert"` then `taskstaka next`; the daemon starts itself on the first command.

## From source

Rust 1.89 or newer.

```
git clone https://github.com/a6wright/taskstaka
cd taskstaka
cargo install --path crates/taskstaka
taskstaka init
```

## macOS menu bar and ⌥Space

The menu bar item shows the task in hand, its notes, and what you can do now. <kbd>⌥Space</kbd> drops a Spotlight-style quick-add panel over whatever you are doing: type, press Enter, and the panel closes.

```
cargo install --path crates/tray      # taskstaka-tray (Homebrew installed it for you)
taskstaka setup macos                 # launchd agents for the daemon and the tray
```

`taskstaka setup macos --hotkey ctrl+alt+space` picks another key. `--no-tray` installs the daemon agent only. `--uninstall` stops and removes both. `taskstaka tray start`, `stop` and `status` control the item afterwards, through launchd. Logs go to `~/Library/Logs/taskstaka.log` and `taskstaka-tray.log`.

## Claude Code

```
taskstaka setup claude
```

Installs the taskstaka skill under `~/.claude/skills/`, adds a managed block to `~/.claude/CLAUDE.md`, and registers the MCP server. Restart Claude Code or run `/mcp`, then ask "what should I work on?". Run it again after an upgrade; it rewrites the same files. `--project` installs into the current repo instead; `--dry-run` shows the commands.

## Always on

The first command starts the daemon in the background and later commands reuse it. To bring it back at login:

```
brew services start taskstaka                 # Homebrew: the daemon only
taskstaka setup macos                         # macOS: the daemon and the menu bar item
systemctl --user enable --now taskstaka       # Linux, with a user unit running `taskstaka serve`
```

`taskstaka status`, `stop` and `restart` know when launchd or `brew services` owns the daemon and go through it, so a restart keeps the daemon supervised.

## Updating

```
brew upgrade taskstaka
taskstaka restart              # the old daemon keeps serving old code until you do this
```

From a checkout: `cargo install --path crates/taskstaka` (and `--path crates/tray`), then `taskstaka restart`. On macOS, run `taskstaka setup macos` again so the menu bar item reloads too. Each command warns while the daemon is older than the binary.
