GraphRelay — graph-aware semantic code search (MCP + REST)

To install:

  1. Double-click "Install.command".

     This build isn't code-signed or notarized (no Apple Developer ID is
     configured for this project), so macOS will refuse to open it the
     first time with an "unidentified developer" warning. Right-click (or
     Control-click) "Install.command" and choose Open, then confirm in the
     dialog — you only need to do this once.

  2. Follow the instructions printed in the Terminal window that opens.

Get started after installing:

  gr init      # register the current project
  gr start     # start the daemon (downloads the embedding model on first run)
  gr setup     # wire up Claude Code, Cursor, or other coding tools

More info: https://github.com/openwt-vn/graphrelay
