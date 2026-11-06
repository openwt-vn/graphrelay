# Making OpenCode Always Use GraphRelay First

OpenCode happens to read the same MCP config format Claude Code does, so getting it working is two steps:

1. Register the MCP server (the same JSON file Claude Code uses)
2. Add the instruction block to `AGENTS.md`

`gr setup` does both for you in one command — read on if you want to do it by hand or understand what it's doing.

---

## Automated setup (recommended)

```sh
cd /path/to/your-project
gr setup
```

`gr setup` detects that OpenCode is installed, injects the MCP server entry
into `opencode.json` in your project root, and appends the
instruction block to `AGENTS.md` in your project root.

---

## Why AGENTS.md is required

Here's the symptom you'll hit without it:

> "I installed the MCP server but the tool only calls GraphRelay when I explicitly
> mention it in the prompt. It doesn't use it automatically."

OpenCode — like most non-Claude tools — won't proactively reach for an MCP tool unless something tells it to. The `AGENTS.md` block is that something: it tells the model to try `search_code` before falling back to raw file reads.

Skip it, and the MCP server sits there connected but effectively dormant — available, but never used.

---

## Manual setup

### Step 1 — Register the MCP server

Add the server to `opencode.json` in your project root (or `~/.config/opencode/opencode.json`):

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "graphrelay": {
      "type": "remote",
      "url": "http://localhost:3030/your-project/mcp",
      "enabled": true
    }
  }
}
```

Replace `your-project` with the slug shown in `gr status`.

### Step 2 — Add AGENTS.md

Create or append to `AGENTS.md` at the project root:

```markdown
## Code search — use GraphRelay first

Before reading files, grepping, or exploring the codebase, always query GraphRelay MCP
tools. GraphRelay returns ranked symbols with call graphs in one tool call instead of
many file reads.

- `search_code`  — intent-based search (ranked symbols + snippets + call graphs) or
  named-type lookup (find every implementation of an interface/trait/base class)
- `inspect`      — a known symbol's definition + callers/callees, or a file's/module's
  contents (pass `include_source: true` for a file's full text, in place of Read)
- `trace_impact` — blast-radius before changing anything
- `watch_status` — check index freshness

Only fall back to direct file reads when GraphRelay cannot answer (config files,
non-indexed content, binary files).
```

---

## Verifying it works

Start a session and ask a question that requires exploring the codebase:

```
How does user authentication work in this project?
```

In the tool call log you should see `search_code` called **before** any file
reads. If OpenCode skips straight to reading files, double-check:

1. `AGENTS.md` exists at the project root and contains `search_code`
2. The daemon is running: `gr status`
3. The MCP entry is in `opencode.json` (project root) under `mcp`
4. You restarted OpenCode after adding the config

---

## Switching between models

OpenCode defaults to the smallest model in its model list. The instruction block
works with any model, but smaller models follow instructions less reliably. If
GraphRelay is still not invoked automatically after setup, try a larger model.

GraphRelay itself is model-agnostic — the same MCP tools work regardless of which
LLM OpenCode is using.
