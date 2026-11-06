# Harness AI — GraphRelay integration (help wanted)

> **Status: unconfirmed.** We haven't verified Harness AIDA's MCP config format ourselves yet. If you use Harness and get this working, a PR against this file — or even just an issue with what you found — would genuinely help the next person.

---

## What we know so far

- Harness AIDA has supported MCP since mid-2025.
- The instruction file is likely `AGENTS.md`, same as OpenCode, Amp, and Codex — but we haven't confirmed Harness actually reads it.
- The MCP server registration format is unconfirmed.

---

## What to try if you're using Harness

1. **Check whether Harness AIDA reads `AGENTS.md` from the project root.** If it does, the instruction block from [docs/agents-md-hook.md](agents-md-hook.md) should work as-is.

2. **Check whether Harness AIDA supports the Claude Desktop MCP config format** (`~/.claude/claude_desktop_config.json`). If it does, `gr setup` may already register the server correctly — run it and see whether Harness picks up the connection.

3. **Tell us what you find.** Open an issue at [github.com/openwt-vn/graphrelay/issues](https://github.com/openwt-vn/graphrelay/issues) with what worked, or a PR updating this file directly — either closes the loop for the next person trying this.

We'd rather ship this page honestly incomplete than pretend it's tested when it isn't.
