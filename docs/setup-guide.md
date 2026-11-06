# Setup Guide

This walks you from zero to Claude Code using GraphRelay on your project — realistically under 5 minutes, most of which is waiting for the first index to finish.

---

## 1. Install

```sh
curl -fsSL https://raw.githubusercontent.com/openwt-vn/graphrelay/main/scripts/install.sh | sh
```

This grabs the pre-built binary for your OS/architecture and drops it at `~/.local/bin/gr`.

**Supported platforms:**
- macOS Apple Silicon (`aarch64-apple-darwin`)
- Linux x86_64 (`x86_64-unknown-linux-gnu`)
- Linux ARM64 (`aarch64-unknown-linux-gnu`)

Verify it worked:
```sh
gr --help
```

> **PATH note:** If `gr` isn't found, add `~/.local/bin` to your shell profile:
> ```sh
> echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
> ```

---

## 2. Register your project

```sh
cd /path/to/your-project
gr init
```

There is no embedding model to choose or download: retrieval is SQLite FTS5 (BM25) plus call-graph expansion and reranking, always (see [how it works](how-it-works.md)).

`gr init` creates `.graphrelay/config.toml` (your project config) and adds `.graphrelay/` to `.gitignore`, so none of this touches your repo history.

---

## 3. Start the daemon

```sh
gr start
```

On first run, this indexes your project — parse with tree-sitter, write the symbol/call graph to SQLite, build the lexical index — and then starts watching for file changes. Nothing is downloaded.

Check progress with:
```sh
gr status
```

Wait until the state shows `fresh`:

```
Daemon: running (PID 12345, port 3030)

Project       State    Files    Symbols  URL
----------------------------------------------------------------
your-project  fresh     1234       8901  http://localhost:3030/your-project/mcp
```

Large projects (100k+ lines) can take a minute or two on the first pass. After that, only changed files get re-indexed, so subsequent starts are fast.

---

## 4. Connect your coding agent

The fastest path is one command:

```sh
gr setup
```

This detects Claude Code, Cursor, OpenCode, Amp, or Codex, registers the MCP server with whichever ones it finds, and writes the "use GraphRelay first" instruction block into the right file for each (`CLAUDE.md`, `.cursorrules`, or `AGENTS.md`). Run it interactively and it'll show you a checkbox picker; run `gr setup --all` to skip the picker and configure everything it detects, or `gr setup --dry-run` to see what it would do without writing anything.

If you'd rather wire up Claude Code by hand:

```sh
claude mcp add graphrelay --transport http http://localhost:3030/your-project/mcp
```

Replace `your-project` with the slug shown in `gr status`. Verify it connected:
```sh
claude mcp list
# your-project: connected (4 tools)
```

---

## 5. Make sure your agent actually uses it

Registering the MCP server gets you the tools; it doesn't guarantee your agent reaches for them instead of grepping. `gr setup` already handles this for you by writing the instruction hook — but if you set things up manually, or want to see what it wrote, add this to your project's `CLAUDE.md` (create it at the repo root if it doesn't exist):

```markdown
## Code search — use GraphRelay first

Before reading files, grepping, or exploring the codebase, always query GraphRelay MCP tools:

- `search_code` — intent-based search, returns ranked symbols with snippets and call graphs
  (or a named-type lookup via `implements`)
- `inspect` — understand a specific function/class with its callers and callees (`symbol`),
  or get the symbol map near a line instead of reading a raw file (`path`, plus
  `include_source: true` if you want the full file text)
- `trace_impact` — before changing anything, check blast radius

Only fall back to direct file reads when GraphRelay cannot answer (config files, non-indexed content).
```

This is the same block this project's own `CLAUDE.md` uses — it works. See [docs/claude-code-hook.md](claude-code-hook.md) for the full version with more tool coverage, and [docs/agents-md-hook.md](agents-md-hook.md) if you're wiring up a non-Claude tool by hand.

---

## 6. Check it end-to-end

Query directly with curl, no agent involved:

```sh
curl -s -X POST http://localhost:3030/your-project/mcp \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "tools/call",
    "params": {
      "name": "search_code",
      "arguments": {"intent": "how is authentication handled", "top_n": 3}
    }
  }' | python3 -m json.tool
```

Or just ask your agent directly:

```
How is authentication handled in this project?
```

Watch the tool-call log. You should see `search_code` invoked before any raw file reads, and the answer should come back with real file paths and line numbers — not a guess from training data.

---

## Multiple projects

Register as many projects as you like; one daemon serves all of them on the same port:

```sh
cd /path/to/project-a && gr init
cd /path/to/project-b && gr init

gr start   # one daemon, every registered project

claude mcp add project-a --transport http http://localhost:3030/project-a/mcp
claude mcp add project-b --transport http http://localhost:3030/project-b/mcp
```

---

## Updating

```sh
gr update
```

This downloads the latest release and replaces the running binary in place. (You can also re-run the install script from step 1 — it does the same thing.)

---

## Uninstalling

```sh
gr stop
rm ~/.local/bin/gr
rm -rf ~/.graphrelay          # removes every index
```

To remove just one project without touching the binary or your other indexes: `gr unregister` (run it from inside the project directory).

---

## Common issues

**`graphrelay: command not found`** — `~/.local/bin` isn't on your PATH. See step 1.

**Daemon shows "not running" immediately after start** — check the logs:
```sh
cat ~/.graphrelay/daemon.log
```
Usually a port conflict or an unreadable project directory.

**Port 3030 already in use:**
```sh
gr stop
# or use another port:
gr start --port 3031
```

**Index stuck on "indexing" for a while** — normal for very large repos on the first run. Watch it progress:
```sh
gr status   # total_files should be climbing
```

**State shows "stale" after editing files** — the watcher should pick up changes within 50–100ms. If it doesn't, force a fresh index:
```sh
gr stop && gr index && gr start
```

---

## Other MCP clients

Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "myproject": { "type": "http", "url": "http://localhost:3030/myproject/mcp" }
  }
}
```

Claude Code, manually: `claude mcp add graphrelay --transport http http://localhost:3030/myproject/mcp`.

`gr setup` writes the `AGENTS.md` instruction hook for OpenCode, Amp and Codex and registers the MCP server for OpenCode; for Amp and Codex it prints where to add the server URL by hand. Any other MCP client can `POST http://localhost:3030/{project}/mcp` (JSON-RPC 2.0 over HTTP).
