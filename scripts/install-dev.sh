#!/usr/bin/env bash
# Install Previewer's icon + .desktop file into the user's XDG data dirs so
# GNOME Shell, the dock, and Activities pick up the app icon when running
# the dev build. Safe to re-run.
#
# This is a development convenience. M7 packaging (PKGBUILD / .deb) installs
# system-wide to /usr/share/.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID="org.moma.Previewer"

ICON_SRC="$REPO_ROOT/data/icons/hicolor/scalable/apps/${APP_ID}.svg"
ICON_DEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
ICON_DEST="$ICON_DEST_DIR/${APP_ID}.svg"

DESKTOP_DEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
DESKTOP_DEST="$DESKTOP_DEST_DIR/${APP_ID}.desktop"

BIN_PATH="$REPO_ROOT/target/debug/previewer"

mkdir -p "$ICON_DEST_DIR" "$DESKTOP_DEST_DIR"

# Icon: symlink so updates to the SVG in-repo flow through immediately.
ln -sf "$ICON_SRC" "$ICON_DEST"
echo "✓ icon → $ICON_DEST"

# Desktop file: write a dev variant that points Exec at the built binary so
# GNOME Shell can launch it directly from Activities and match it back to the
# running window.
cat > "$DESKTOP_DEST" <<EOF
[Desktop Entry]
Type=Application
Name=Previewer
GenericName=Image and PDF Viewer
Comment=View, annotate, and sign images and PDFs
Categories=Graphics;Office;Viewer;
Icon=${APP_ID}
StartupNotify=true
StartupWMClass=${APP_ID}
Exec=${BIN_PATH} %F
Terminal=false
MimeType=image/png;image/jpeg;image/webp;image/heic;image/heif;application/pdf;
EOF
echo "✓ desktop file → $DESKTOP_DEST"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$DESKTOP_DEST_DIR" || true
fi

echo
echo "Done. If the dock doesn't pick the icon up immediately, log out + back in"
echo "(or restart gnome-shell with Alt+F2 → 'r' on Xorg sessions)."
