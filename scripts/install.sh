#!/usr/bin/env sh
# Install agentty for the current user (no root). Uses the prebuilt binary next to this script
# (release tarball), or builds from source in a git checkout.
set -eu
cd "$(dirname "$0")"
[ -f ./agentty ] || { cd ..; cargo build --release; cp target/release/agentty ./agentty.bin; }
BIN=$( [ -f ./agentty ] && echo ./agentty || echo ./agentty.bin )
DESKTOP=$( [ -f ./agentty.desktop ] && echo ./agentty.desktop || echo packaging/agentty.desktop )
ICON=$( [ -f ./agentty.png ] && echo ./agentty.png || echo packaging/agentty.png )
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/256x256/apps"
install -m 755 "$BIN" "$HOME/.local/bin/agentty"
rm -f ./agentty.bin
sed "s|^Exec=agentty|Exec=$HOME/.local/bin/agentty|" "$DESKTOP" > "$HOME/.local/share/applications/agentty.desktop"
install -m 644 "$ICON" "$HOME/.local/share/icons/hicolor/256x256/apps/agentty.png"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$HOME/.local/share/applications" || true
echo "Installed $HOME/.local/bin/agentty and an app-menu entry. Run: agentty"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Note: add $HOME/.local/bin to your PATH." ;; esac
