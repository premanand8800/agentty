#!/usr/bin/env sh
# Build agentty and install it for the current user (no root needed).
set -eu
cd "$(dirname "$0")/.."
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/256x256/apps"
install -m 755 target/release/agentty "$HOME/.local/bin/agentty"
sed "s|^Exec=agentty|Exec=$HOME/.local/bin/agentty|" packaging/agentty.desktop > "$HOME/.local/share/applications/agentty.desktop"
install -m 644 packaging/agentty.png "$HOME/.local/share/icons/hicolor/256x256/apps/agentty.png"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$HOME/.local/share/applications" || true
echo "Installed: $HOME/.local/bin/agentty (and an app-menu entry). Run: agentty"
