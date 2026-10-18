#!/bin/sh
# taskstaka installer — https://taskstaka.com/install.sh
#
#   curl -fsSL https://taskstaka.com/install.sh | sh
#
# macOS: Homebrew tap, then `taskstaka setup macos` (menu bar item, ⌥Space).
# Linux: the tray's build packages (apt, dnf or pacman), rustup if there is no
# cargo, `cargo install` of both binaries from the latest release, then
# `taskstaka setup linux` (systemd user units; on GNOME, Alt+Space).
#
# Knobs (environment variables):
#   TASKSTAKA_REF=main      build that branch or tag instead of the latest release
#   TASKSTAKA_NO_TRAY=1     skip the menu bar item (and its build packages)
#   TASKSTAKA_NO_SETUP=1    install the binaries only; no agents, units or init
set -eu

REPO=https://github.com/a6wright/taskstaka

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

os=$(uname -s)
case "$os" in
  Darwin | Linux) ;;
  *) die "unsupported system: $os" ;;
esac

install_macos() {
  have brew || die "Homebrew is needed on macOS: https://brew.sh"
  say "Installing taskstaka with Homebrew (builds from source; a few minutes)"
  brew install a6wright/taskstaka/taskstaka
}

# The newest tagged release, or main when GitHub cannot be asked.
latest_ref() {
  if [ -n "${TASKSTAKA_REF:-}" ]; then
    printf '%s' "$TASKSTAKA_REF"
    return
  fi
  tag=$(curl -fsSL https://api.github.com/repos/a6wright/taskstaka/releases/latest 2>/dev/null \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
  printf '%s' "${tag:-main}"
}

tray_packages() {
  if pkg-config --exists gtk+-3.0 webkit2gtk-4.1 2>/dev/null; then
    return
  fi
  sudo=sudo
  [ "$(id -u)" = 0 ] && sudo=""
  if have apt-get; then
    cmd="$sudo apt-get install -y --no-install-recommends build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev"
  elif have dnf; then
    cmd="$sudo dnf install -y gcc pkgconf-pkg-config gtk3-devel webkit2gtk4.1-devel"
  elif have pacman; then
    cmd="$sudo pacman -S --needed --noconfirm base-devel gtk3 webkit2gtk-4.1"
  else
    die "the menu bar item needs GTK 3 and WebKitGTK 4.1 development packages; install them, or set TASKSTAKA_NO_TRAY=1"
  fi
  say "Installing the menu bar item's build packages (asks for your password)"
  printf '    %s\n' "$cmd"
  $cmd </dev/tty || die "could not install the packages; install them by hand, or set TASKSTAKA_NO_TRAY=1"
}

install_linux() {
  if ! have cargo; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
      PATH="$HOME/.cargo/bin:$PATH"
    else
      say "Installing Rust with rustup (taskstaka builds from source)"
      curl --proto '=https' --tlsv1.2 -fsSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
      PATH="$HOME/.cargo/bin:$PATH"
    fi
  fi
  [ -n "${TASKSTAKA_NO_TRAY:-}" ] || tray_packages
  ref=$(latest_ref)
  case "$ref" in
    v*) refarg="--tag $ref" ;;
    *) refarg="--branch $ref" ;;
  esac
  say "Building taskstaka ($ref); the first build takes a few minutes"
  cargo install --locked --git "$REPO" $refarg taskstaka
  if [ -z "${TASKSTAKA_NO_TRAY:-}" ]; then
    say "Building the menu bar item"
    cargo install --locked --git "$REPO" $refarg taskstaka-tray
  fi
}

case "$os" in
  Darwin) install_macos ;;
  Linux) install_linux ;;
esac

PATH="$HOME/.cargo/bin:$PATH"
have taskstaka || die "taskstaka is not on PATH; add ~/.cargo/bin (Linux) or Homebrew's bin (macOS) and run: taskstaka init"

if [ -n "${TASKSTAKA_NO_SETUP:-}" ]; then
  say "Installed. Next: taskstaka init"
  exit 0
fi

say "Creating the notes folder (~/todo unless TASKSTAKA_DIR says otherwise)"
taskstaka init

notray=""
[ -n "${TASKSTAKA_NO_TRAY:-}" ] && notray="--no-tray"
case "$os" in
  Darwin)
    say "Keeping the daemon and the menu bar item running across logins (launchd)"
    taskstaka setup macos $notray
    ;;
  Linux)
    say "Keeping the daemon and the menu bar item running across logins (systemd --user)"
    taskstaka setup linux $notray
    ;;
esac

say "Done. Try: taskstaka add \"Renew the cert\"  ·  taskstaka next  ·  the quick-add key from any app"
printf '    Claude Code: taskstaka setup claude   ·   docs: https://taskstaka.com/help\n'
