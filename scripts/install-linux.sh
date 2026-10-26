#!/usr/bin/env bash
# Halny Vibe Deck - Linux user installation (no root required).
# Installs into ~/.local/share/halny-vibe-deck/app and exposes the
# 'halny' command through ~/.local/bin. Re-running the script upgrades
# an existing installation in place.
set -euo pipefail

REPO="Dragonk/halny-vibe-deck"
ASSET="halny-vibe-deck-linux-x64.zip"
INSTALL_DIR="${HALNY_INSTALL_DIR:-$HOME/.local/share/halny-vibe-deck/app}"
BIN_DIR="${HALNY_BIN_DIR:-$HOME/.local/bin}"

command -v curl >/dev/null 2>&1 || { echo "error: curl is required"; exit 1; }
command -v unzip >/dev/null 2>&1 || { echo "error: unzip is required"; exit 1; }

echo "Fetching the latest release from GitHub..."
JSON="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")"
URL="$(printf '%s' "$JSON" | grep -o -m1 "https://[^\"/]*$ASSET")"
if [ -z "$URL" ]; then
  echo "error: no $ASSET asset found in the latest release"
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "Downloading $URL ..."
curl -fL --progress-bar -o "$TMP/halny.zip" "$URL"

mkdir -p "$INSTALL_DIR" "$BIN_DIR"
echo "Installing into $INSTALL_DIR ..."
unzip -o "$TMP/halny.zip" -d "$INSTALL_DIR" >/dev/null

# Locate the Electron executable; legacy names are kept so the script
# also upgrades pre-rename layouts.
EXE="$(find "$INSTALL_DIR" -maxdepth 1 -type f \( -name 'halny-vibe-deck' -o -name 'vibe-nexus' -o -name 'mistral-vibe-nexus' \) | head -n1)"
if [ -z "$EXE" ]; then
  echo "error: could not find the app executable in $INSTALL_DIR"
  exit 1
fi
chmod +x "$EXE"

printf '#!/usr/bin/env bash\nexec "%s" "$@"\n' "$EXE" > "$BIN_DIR/halny"
chmod +x "$BIN_DIR/halny"
rm -f "$BIN_DIR/vibe-nexus"

mkdir -p "$HOME/.local/share/applications"
printf '[Desktop Entry]\nType=Application\nName=Halny Vibe Deck\nExec=%s\nIcon=%s\nTerminal=false\nCategories=Development;\n' \
  "$EXE" "$INSTALL_DIR/resources/icon.png" > "$HOME/.local/share/applications/halny-vibe-deck.desktop"

echo ""
echo "Halny Vibe Deck installed."
echo "  App:     $INSTALL_DIR"
echo "  Command: halny"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    echo "  Hint: add this line to your ~/.bashrc to get the command:"
    echo "        export PATH=\"$BIN_DIR:$PATH\""
    ;;
esac

if [ -d "$HOME/.local/share/vibe-nexus/app" ]; then
  echo ""
  echo "Note: found a legacy vibe-nexus installation at ~/.local/share/vibe-nexus/app — remove it manually if you no longer need it."
fi
