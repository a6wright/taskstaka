# Install

## One line

```
curl -fsSL https://taskstaka.com/install.sh | sh
```

On macOS it installs the Homebrew tap and runs `taskstaka setup macos`. On Linux it installs the menu bar item's build packages (apt, dnf or pacman; it asks for your password), Rust with rustup if there is no `cargo`, builds both binaries from the latest release, then runs `taskstaka init` and `taskstaka setup linux`. Read it first if you like: it is short. `TASKSTAKA_NO_TRAY=1` skips the menu bar item, `TASKSTAKA_NO_SETUP=1` installs the binaries only.

`taskstaka add "Renew the cert"` then `taskstaka next`; the daemon starts itself on the first command.

## Homebrew

macOS and Linux. It builds from source, so the first install takes a few minutes. On Linux it installs the CLI and the daemon only; the menu bar item comes from source (below).

```
brew install a6wright/taskstaka/taskstaka
taskstaka init                 # creates ~/todo (or $TASKSTAKA_DIR)
```

## From source

Rust 1.89 or newer.

```
git clone https://github.com/a6wright/taskstaka
cd taskstaka
cargo install --path crates/taskstaka
cargo install --path crates/tray      # the menu bar item (Linux: see the packages below)
taskstaka init
```

## macOS menu bar and ⌥Space

The menu bar item shows the task in hand, its notes, and what you can do now. <kbd>⌥Space</kbd> drops a Spotlight-style quick-add panel over whatever you are doing: type, press Enter, and the panel closes.

```
taskstaka setup macos                 # launchd agents for the daemon and the tray
```

`taskstaka setup macos --hotkey ctrl+alt+space` picks another key. `--no-tray` installs the daemon agent only. `--uninstall` stops and removes both. `taskstaka tray start`, `stop` and `status` control the item afterwards, through launchd. Logs go to `~/Library/Logs/taskstaka.log` and `taskstaka-tray.log`.

## Linux top bar and Alt+Space

The same item and panel, as a StatusNotifier icon with the plate's title beside it. It builds against GTK 3 and WebKitGTK 4.1:

```
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev        # Ubuntu, Debian
sudo dnf install gtk3-devel webkit2gtk4.1-devel            # Fedora
cargo install --path crates/tray
taskstaka setup linux                 # systemd user units for the daemon and the tray
```

`setup linux` also binds the key. No app can grab a key under Wayland, so on GNOME it writes a keyboard shortcut named *taskstaka: quick add* that runs `taskstaka tray quick`, which toggles the panel of the running item. GNOME has <kbd>Alt+Space</kbd> on the window menu by default; setup takes it and tells you how to give it back (`--hotkey ctrl+alt+space` picks another key). On KDE, Sway, Hyprland or anything else, bind that command yourself in the desktop's keyboard settings.

GNOME shows tray items only with a StatusNotifier host. Ubuntu ships the AppIndicator extension enabled; on Fedora and others install `gnome-shell-extension-appindicator`, log out and in, and enable it. Setup says so when it is missing.

`--no-tray` installs the daemon unit only. `--uninstall` stops and removes both units and the shortcut. `taskstaka tray start`, `stop`, `status` and `quick` control the item afterwards, through systemd. Logs: `journalctl --user -u taskstaka -u taskstaka-tray`.

## Claude Code

```
taskstaka setup claude
```

Installs the taskstaka skill under `~/.claude/skills/`, adds a managed block to `~/.claude/CLAUDE.md`, and registers the MCP server. Restart Claude Code or run `/mcp`, then ask "what should I work on?". Run it again after an upgrade; it rewrites the same files. `--project` installs into the current repo instead; `--dry-run` shows the commands.

## Always on

The first command starts the daemon in the background and later commands reuse it. To bring it back at login:

```
taskstaka setup macos                         # macOS: the daemon and the menu bar item
taskstaka setup linux                         # Linux: the daemon and the menu bar item
brew services start taskstaka                 # Homebrew: the daemon only
```

`taskstaka status`, `stop` and `restart` know when launchd, systemd or `brew services` owns the daemon and go through it, so a restart keeps the daemon supervised.

## Updating

```
brew upgrade taskstaka
taskstaka restart              # the old daemon keeps serving old code until you do this
```

From a checkout: `cargo install --path crates/taskstaka` (and `--path crates/tray`), then `taskstaka restart`. Run `taskstaka setup macos` or `taskstaka setup linux` again so the menu bar item reloads too. The one-line installer can be run again as well. Each command warns while the daemon is older than the binary.
