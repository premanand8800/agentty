#!/usr/bin/env sh
# One-line installer for Linux and macOS:
#   curl -fsSL https://raw.githubusercontent.com/premanand8800/agentty/main/scripts/get.sh | sh
set -eu
REPO="premanand8800/agentty"
OS=$(uname -s); ARCH=$(uname -m)
case "$OS-$ARCH" in
  Linux-x86_64) ASSET="agentty-linux-x86_64.tar.gz" ;;
  Linux-aarch64|Linux-arm64) ASSET="agentty-linux-aarch64.tar.gz" ;;
  Darwin-*) ASSET="agentty-macos-universal.zip" ;;
  *) echo "No prebuilt agentty for $OS $ARCH. Build from source: https://github.com/$REPO"; exit 1 ;;
esac
URL="https://github.com/$REPO/releases/latest/download/$ASSET"
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
echo "Downloading $ASSET ..."
curl -fsSL "$URL" -o "$TMP/$ASSET"
if [ "$OS" = "Darwin" ]; then
  (cd "$TMP" && unzip -q "$ASSET")
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/agentty.app"
  mv "$TMP/agentty.app" "$HOME/Applications/"
  # Not notarized yet: clear the download quarantine so Gatekeeper lets it open.
  xattr -dr com.apple.quarantine "$HOME/Applications/agentty.app" 2>/dev/null || true
  mkdir -p "$HOME/.local/bin"
  ln -sf "$HOME/Applications/agentty.app/Contents/MacOS/agentty" "$HOME/.local/bin/agentty"
  echo "Installed ~/Applications/agentty.app (open it from Launchpad or Spotlight) and the agentty command."
else
  tar -xzf "$TMP/$ASSET" -C "$TMP"
  sh "$TMP/agentty/install.sh"
fi
