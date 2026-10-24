#!/bin/sh
# Installs daw for the current user: the app in ~/.local/bin, plus an icon
# and a menu entry so app launchers (wofi, rofi, fuzzel, GNOME, KDE...) find
# it. Run from the unpacked download folder. No sudo needed.
#
#   ./install.sh               install or update
#   ./install.sh --uninstall   remove everything this put in place
set -eu

here=$(cd "$(dirname "$0")" && pwd)
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin/daw"
icon="$data/icons/hicolor/512x512/apps/daw.png"
entry="$data/applications/daw.desktop"

refresh() {
    # Not every desktop has these; skipping them is harmless.
    command -v update-desktop-database > /dev/null 2>&1 &&
        update-desktop-database -q "$data/applications" || true
    command -v gtk-update-icon-cache > /dev/null 2>&1 &&
        gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin" "$icon" "$entry"
    refresh
    echo "daw removed."
    exit 0
fi

install -Dm755 "$here/daw" "$bin"
install -Dm644 "$here/daw.png" "$icon"
# Point the menu entry at the full path, since launchers don't always have
# ~/.local/bin on their PATH.
mkdir -p "$(dirname "$entry")"
sed "s|^Exec=.*|Exec=$bin|" "$here/daw.desktop" > "$entry"
refresh

echo "daw installed: $bin"
echo "Open it from your app launcher, or run: daw"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo "Note: ~/.local/bin isn't on your PATH, so 'daw' in a terminal won't work yet." ;;
esac
