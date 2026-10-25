#!/usr/bin/env bash
# Mistral Vibe Nexus - Linux user installation (no root required).
# Installs the app into ~/.local/share/vibe-nexus/app and exposes the
# 'vibe-nexus' command through ~/.local/bin. Re-running the script upgrades
# an existing installation in place.
set -euo pipefail

REPO="Dragonk/mistral-vibe-nexus"
ASSET="vibe-nexus-linux-x64.zip"
INSTALL_DIR="${VIBE_NEXUS_INSTALL_DIR:-$HOME/.local/share/vibe-nexus/app}"
BIN_DIR="${VIBE_NEXUS_BIN_DIR:-$HOME/.local/bin}"

command -v curl >/dev/null 2>&1 || { echo "error: curl is required"; exit 1; }
command -v unzip >/dev/null 2>&1 || { echo "error: unzip is required"; exit 1; }

echo "Fetching the latest release from GitHub..."
JSON="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest")"
URL="$(printf '%s' "$JSON" | grep -o -m1 'https://[^"]*'"$ASSET")"
if [ -z "$URL" ]; then
  echo "error: no $ASSET asset found in the latest release"
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "Downloading $URL ..."
curl -fL --progress-bar -o "$TMP/nexus.zip" "$URL"

mkdir -p "$INSTALL_DIR" "$BIN_DIR"
echo "Installing into $INSTALL_DIR ..."
unzip -o "$TMP/nexus.zip" -d "$INSTALL_DIR" >/dev/null

# Locate the Electron executable in the unpacked directory.
EXE="$(find "$INSTALL_DIR" -maxdepth 1 -type f \( -name 'mistral-vibe-nexus' -o -name 'vibe-nexus' \) | head -n1)"
if [ -z "$EXE" ]; then
  echo "error: could not find the app executable in $INSTALL_DIR"
  exit 1
fi
chmod +x "$EXE"

# Command wrapper: vibe-nexus
printf '#!/usr/bin/env bash\nexec "%s" "$@"\n' "$EXE" > "$BIN_DIR/vibe-nexus"
chmod +x "$BIN_DIR/vibe-nexus"

# Desktop entry (application menu)
mkdir -p "$HOME/.local/share/applications"
printf '[Desktop Entry]\nType=Application\nName=Mistral Vibe Nexus\nExec=%s\nIcon=%s\nTerminal=false\nCategories=Development;\n' \
  "$EXE" "$INSTALL_DIR/resources/icon.png" > "$HOME/.local/share/applications/vibe-nexus.desktop"

echo ""
echo "Mistral Vibe Nexus installed."
echo "  App:     $INSTALL_DIR"
echo "  Command: vibe-nexus"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    echo "  Hint: add this line to your ~/.bashrc to get the command:"
    echo "        export PATH="$BIN_DIR:$PATH""
    ;;
esac

