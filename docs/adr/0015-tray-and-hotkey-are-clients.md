---
status: accepted
date: 2026-10-10
amends: 0003
---

# 0015: The tray and the hotkey are clients of the daemon, in their own process

The user's words: "the idea was that we wanted the tray and hotkey for different OS
combinations to live outside of the daemon, so that we kept the specific operating system
code separate from the core daemon … the preference is that we keep OS-specific code
separate and have it make web calls to the daemon."

## Context

The plan said the hotkey was "out of the binary": a launcher (Raycast, Alfred, sxhkd) binds
the key and runs `taskstaka add`. That keeps the daemon free of GUI code but asks the user
to install and configure a second tool, and gives them nothing in the menu bar. A menu bar
item needs an event loop and platform frameworks (AppKit, AppIndicator) that have no
business in a daemon that must start in milliseconds from any CLI command and run headless
under launchd or systemd.

## Decision

- **`taskstaka-tray` is a separate binary in its own crate (`crates/tray`).** It shows the
  plate's title in the menu bar, a menu of the *now* rows, and registers the global hotkey.
  All OS-specific code lives there; the daemon, core and the CLI gain none.
- **It is a client like the TUI**: it reads `GET /api/v1/plate` and `GET /api/v1/plan`,
  follows `GET /api/v1/changes`, and every click is `POST /api/v1/commands` or a URL
  (`/quick`, `/t/{id}`, `/next`) opened in the default browser. It computes nothing;
  ADR 0003's single writer and ADR 0014's plate rule hold unchanged.
- **The daemon gains one read endpoint, `/api/v1/plate`**, because the plate is product
  logic every surface must agree on, and the web was already recomputing it in JavaScript.
- **The HTTP client moved to `crates/client` (`taskstaka-client`)** so the tray depends on
  the client and core only, never on the daemon, the TUI or the MCP bridge.
- **The hotkey shows the existing `/quick` page in a Spotlight-style panel**: a borderless,
  always-on-top `tao` window with a `wry` webview (WKWebView on macOS, WebKitGTK on Linux),
  so quick add never leaves the app you are in. The page's `?panel=1` mode talks to the host
  over wry's IPC (Esc and a finished add hide the panel). First built as a browser tab; the
  user's words: "I want something like how Spotlight opens with cmd+space as the inspiration."
- **Supervisors are respected.** On macOS `stop`, `restart` and `status` detect a launchd
  agent that owns the daemon (`dev.taskstaka` from `taskstaka setup macos`, or
  `sh.brew.taskstaka` from `brew services`) and go through `launchctl kickstart`
  instead of spawning a second daemon. Agents are written with
  `KeepAlive {SuccessfulExit: false}` so a clean `stop` stays stopped and a crash restarts,
  and without `TASKSTAKA_DIR`, so the folder in Settings applies (ADR 0011).

## Consequences

One tray crate serves every OS behind `cfg`: macOS is built and verified first; Linux needs
an icon (AppIndicator hides title-only items) and has no hotkey under Wayland yet (resolved
by ADR 0016: a desktop shortcut runs `taskstaka tray quick`, which reaches the item over a
socket; `setup linux` writes systemd user units). Installing
means two binaries; `taskstaka setup macos` and the Homebrew formula install both. The tray
auto-starts the daemon on launch like any client, but after an explicit `taskstaka stop` it
only waits, until a click or the hotkey asks for the daemon again. systemd awareness for
`restart` is still to do.
