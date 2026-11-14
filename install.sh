#!/usr/bin/env sh
# Build in release mode and install for the current user.
#   ./install.sh             install (or update)
#   ./install.sh uninstall   remove everything installed
# Set PREFIX to install elsewhere (default: ~/.local).
set -eu
cd "$(dirname "$0")"

APP_ID=io.github.NiriSettings
PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/niri-settings"
DESKTOP="$PREFIX/share/applications/$APP_ID.desktop"
ICON="$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg"

refresh_caches() {
    command -v update-desktop-database >/dev/null 2>&1 &&
        update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 &&
        gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

if [ "${1:-}" = "uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    refresh_caches
    echo "Removed niri-settings from $PREFIX."
    exit 0
fi

cargo build --release
install -Dm755 target/release/niri-settings "$BIN"
# Use an absolute Exec path: launchers spawned by the compositor often don't
# have $PREFIX/bin (e.g. ~/.local/bin) on their PATH.
mkdir -p "$(dirname "$DESKTOP")"
sed "s|^Exec=niri-settings|Exec=$BIN|" "data/$APP_ID.desktop" > "$DESKTOP"
chmod 644 "$DESKTOP"
install -Dm644 "data/icons/$APP_ID.svg" "$ICON"
refresh_caches

echo "Installed to $BIN"
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) echo "Note: $PREFIX/bin is not on your PATH." ;;
esac
