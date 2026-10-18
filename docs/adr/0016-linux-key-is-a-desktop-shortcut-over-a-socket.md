---
status: accepted
date: 2026-10-18
amends: 0015
---

# 0016: On Linux the quick-add key is a desktop shortcut that reaches the item over a socket

The user's words: "our goal today is to mirror the osx widget functionality on a
ubuntu/gnome or just general linux setup. can it be done generically? if not, lets do it
for gnome."

## Context

ADR 0015 put the menu bar item and the global hotkey in `taskstaka-tray`. On macOS the
item registers `⌥Space` itself. On Linux the item builds and runs unchanged (GTK event
loop, the mark as a StatusNotifierItem through AppIndicator, the panel as a WebKitGTK
webview), but the key does not: Wayland lets no ordinary process grab a key, and the
`global-hotkey` crate is X11 only. Three ways in were weighed:

- **The XDG GlobalShortcuts portal** (GNOME 48+, KDE, Hyprland) is the generic Wayland
  answer, but it needs an async D-Bus client in the item, shows a confirmation dialog, and
  identifies a non-sandboxed app by its desktop entry and systemd scope, which the item does
  not have yet.
- **A signal** (`SIGUSR1`) to the running item is one line, but WebKit's JavaScript engine
  installs its own handler for it (garbage collection) and overrode ours.
- **A desktop shortcut running a command** is what every Linux desktop offers, and the
  command only needs a way to reach the running item.

## Decision

- **The item listens on a control socket**, `$XDG_RUNTIME_DIR/taskstaka-tray.sock` (else a
  per-user file in the temp dir), one word per request. `taskstaka tray quick` sends
  `toggle`; the item shows the panel or hides it, like the hotkey does. The path lives in
  `taskstaka-client::tray` so both binaries agree, and it works on every OS.
- **`taskstaka tray quick` is what the desktop key runs.** No item running: it starts one
  (through the unit when `setup linux` installed it) and asks again; no item to start: it
  opens the quick page in the browser.
- **`taskstaka setup linux` writes the shortcut on GNOME** through gsettings (a custom
  keybinding named "taskstaka: quick add"), in the same spelling the item shows in its menu
  (`--hotkey alt+space` → `<Alt>space`). GNOME binds Alt+Space to the window menu by
  default; setup frees it and says how to put it back; `--uninstall` restores it. Other
  desktops get the command to bind in the report.
- **`setup linux` mirrors `setup macos`**: systemd user units `taskstaka.service`
  (`WantedBy=default.target`) and `taskstaka-tray.service` (`PartOf` and `WantedBy`
  `graphical-session.target`), `Restart=on-failure` so a crash restarts but `taskstaka stop`
  and Quit stick, no `TASKSTAKA_DIR` (ADR 0011). `stop`, `restart`, `status` and
  `tray start|stop|status` go through `systemctl --user` when a unit owns the process, as they
  go through `launchctl` on macOS.
- **The X11 hotkey stays registered** where it works (X11 sessions); under Wayland it
  simply never fires and the shortcut does the job.

## Consequences

The panel's position is the compositor's choice on Wayland (GNOME centres it), not a fifth
of the way down like Spotlight. GNOME needs a StatusNotifier host (Ubuntu ships the
AppIndicator extension). GTK sizes a non-resizable window to its content, so the panel stays
resizable with pinned size constraints on Linux. The portal remains the way to a key the item
binds itself; it needs a desktop entry first.
