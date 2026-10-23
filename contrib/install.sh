#!/usr/bin/env bash
# Build and install Refract RGB for the current user.
#
#   contrib/install.sh --plugins /path/to/SignalRGB/Plugins
#   contrib/install.sh --dry-run          # show what would run
#   contrib/install.sh --uninstall
#
# Needs sudo for three things only: the udev rule, the plugdev group, and loading i2c-dev.
set -euo pipefail

PLUGINS="${REFRACT_PLUGINS:-}"
DRY=0; UNINSTALL=0; SERVICE=1
while [ $# -gt 0 ]; do
  case "$1" in
    --plugins)    PLUGINS="${2:?--plugins needs a directory}"; shift ;;
    --dry-run)    DRY=1 ;;
    --uninstall)  UNINSTALL=1 ;;
    --no-service) SERVICE=0 ;;
    -h|--help)    sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$HOME/.local/bin/refract"
RULE=/etc/udev/rules.d/70-refract-rgb.rules
MODS=/etc/modules-load.d/refract-rgb.conf
APPS="$HOME/.local/share/applications"
ICON="$HOME/.local/share/icons/hicolor/scalable/apps/refract-rgb.svg"

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
run() { printf '    $ %s\n' "$*"; [ "$DRY" = 1 ] || "$@"; }

if [ "$UNINSTALL" = 1 ]; then
  say "Removing Refract RGB"
  run systemctl --user disable --now refract-rgb.service || true
  run rm -f "$HOME/.config/systemd/user/refract-rgb.service" "$BIN" "$APPS/refract-rgb.desktop" "$ICON"
  run sudo rm -f "$RULE" "$MODS"
  run sudo udevadm control --reload
  echo; echo "Settings in ~/.config/refract-rgb were kept."
  exit 0
fi

say "Checking requirements"
command -v cargo >/dev/null || { echo "Rust is required: https://rustup.rs" >&2; exit 1; }
command -v systemctl >/dev/null || { echo "systemd is required for the background service." >&2; exit 1; }
if ! command -v google-chrome >/dev/null && ! command -v chromium >/dev/null && ! command -v chromium-browser >/dev/null; then
  echo "    note: no Chrome/Chromium found. Built-in effects work; SignalRGB HTML effects will not."
fi
if [ -z "$PLUGINS" ]; then
  echo "    note: no --plugins directory given. Only memory (SMBus) devices will work until you set one;"
  echo "          see the README section 'Getting SignalRGB plugins'."
fi

say "Building"
run cargo build --release --manifest-path "$ROOT/Cargo.toml"
run install -Dm755 "$ROOT/target/release/refract" "$BIN"

say "Device access (needs sudo)"
getent group plugdev >/dev/null || run sudo groupadd plugdev
run sudo install -Dm644 "$ROOT/contrib/70-refract-rgb.rules" "$RULE"
run sudo udevadm control --reload
run sudo udevadm trigger --subsystem-match=hidraw --action=add
run sudo udevadm trigger --subsystem-match=i2c-dev --action=add
echo i2c-dev | { [ "$DRY" = 1 ] && cat >/dev/null || sudo tee "$MODS" >/dev/null; }
run sudo modprobe i2c-dev
if ! id -nG "$USER" | tr ' ' '\n' | grep -qx plugdev; then
  run sudo usermod -aG plugdev "$USER"
  echo "    You were added to 'plugdev': log out and back in once for it to apply."
fi

if [ "$SERVICE" = 1 ]; then
  say "Starting at login"
  if [ -n "$PLUGINS" ]; then export REFRACT_PLUGINS="$PLUGINS"; fi
  run "$BIN" install-service
fi

say "Adding the app launcher"
run install -Dm644 "$ROOT/contrib/refract-rgb.desktop" "$APPS/refract-rgb.desktop"
run install -Dm644 "$ROOT/assets/logo.svg" "$ICON"
run update-desktop-database "$APPS" || true

say "Done"
echo "    Controls: http://127.0.0.1:7878/   (or the tray icon)"
echo "    Status:   systemctl --user status refract-rgb"
