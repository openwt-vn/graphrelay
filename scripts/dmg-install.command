#!/bin/bash
# Double-clicked from the mounted GraphRelay DMG. Copies the bundled `gr`
# binary into ~/.local/bin (matching scripts/install.sh's default) and clears
# the quarantine flag Finder/DMG downloads attach, so the first `gr`
# invocation doesn't hit a Gatekeeper prompt on top of this one.
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="$HOME/.local/bin"

mkdir -p "$INSTALL_DIR"
cp "$DIR/gr" "$INSTALL_DIR/gr"
chmod +x "$INSTALL_DIR/gr"
xattr -d com.apple.quarantine "$INSTALL_DIR/gr" 2>/dev/null || true

echo ""
echo "✓ Installed gr to $INSTALL_DIR/graphrelay"
echo ""

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo "Add this to your shell profile (~/.zshrc or ~/.bash_profile), then restart your terminal:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    echo ""
    ;;
esac

echo "Get started:"
echo "  gr init      # register the current project"
echo "  gr start     # start the daemon"
echo ""
read -r -p "Press Enter to close this window..." _
