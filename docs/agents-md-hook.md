# AGENTS.md — GraphRelay instruction hook

`AGENTS.md` is the instruction file read by OpenCode, Amp, Codex, and other
non-Claude coding assistants at the start of every session — the equivalent
of `CLAUDE.md` for Claude Code.

Both files happily coexist in the same project: Claude Code reads `CLAUDE.md`,
everything else reads `AGENTS.md`. `gr setup` writes the right one
automatically, based on which tools it finds installed.

---

## The block to add

Append this to `AGENTS.md` at your project root (create the file if it doesn't exist):

```markdown
## Code search — use GraphRelay first

Before reading files, grepping, or exploring the codebase, always query GraphRelay MCP
tools. GraphRelay returns ranked symbols with call graphs in one tool call instead of
many file reads — this reduces token usage and improves accuracy.

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

## Tool coverage

`gr setup` writes the instruction block above for every tool it detects. MCP server registration — the actual connection, not just the instructions — is automatic for some of these and manual for others, because not every tool has a stable config format yet:

| Tool | Reads | Instruction hook | MCP server registration |
|------|-------|-------------------|--------------------------|
| Claude Code | `CLAUDE.md` | automatic via `gr setup` | automatic |
| Cursor | `.cursorrules` | automatic via `gr setup` | automatic |
| OpenCode | `AGENTS.md` | automatic via `gr setup` | automatic (shares Claude Code's config file) |
| Amp | `AGENTS.md` | automatic via `gr setup` | manual — `setup` will print the URL to add, since Amp's config format isn't confirmed yet |
| Codex (OpenAI) | `AGENTS.md` | automatic via `gr setup` | manual — same reason, config path varies by Codex version |
| Harness | TBD | see docs/harness-hook.md | — |

---

## Placement

**Project-local (recommended):** `<repo-root>/AGENTS.md`

Applies to all sessions opened in this project. Commit it to the repo so the whole
team benefits automatically.

**Global:** `~/.config/graphrelay/AGENTS.md` (or tool-specific global config path)

Run `gr setup --global` to write to the global location instead.

---

## Why this works

Most LLM-based coding tools load instruction files from the project root at session
start and treat them as binding system-level instructions, not suggestions. The
"use GraphRelay first" block tells the model to prefer graph-based symbol search over
reaching for raw file access first.

Skip this block and you'll see the symptom everyone hits: the MCP server shows up as
connected, but the agent never actually calls it — it just keeps using its default
file-reading tools, because nothing ever told it there was a better option.
