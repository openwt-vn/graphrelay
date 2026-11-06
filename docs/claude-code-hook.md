# Making Claude Code Always Use GraphRelay First

Registering the MCP server isn't enough on its own — by default, Claude Code doesn't know to prefer GraphRelay over its built-in file-reading and grep tools. It'll happily read raw files even with GraphRelay sitting right there. This guide shows how to fix that so Claude reaches for GraphRelay automatically, on every task, without you having to ask each time.

If you haven't set this up yet, `gr setup` does it for you in one step — this guide is for when you want to do it by hand, understand what it's writing, or customize the instruction block.

---

## The short version

Add this block to your project's `CLAUDE.md` (create it at the repo root if it doesn't exist):

```markdown
## Code search — use GraphRelay first

Before reading files, grepping, or exploring the codebase, always query GraphRelay MCP tools:

- `search_code` — intent-based search (ranked symbols with snippets and call graphs) or
  named-type lookup (all implementations of an interface/trait/base class)
- `inspect` — a known symbol's definition + callers/callees, or a file's/module's contents
  (symbol map near a line, or a high-level directory summary, depending on what you pass)
- `trace_impact` — before changing anything, check blast radius
- `watch_status` — check index freshness before trusting results

Only fall back to direct file reads when GraphRelay cannot answer the question
(config files, build scripts, non-indexed content).
```

That's all. Claude Code reads `CLAUDE.md` at the start of every session and treats it as binding instructions.

---

## Why this works

Claude Code loads `CLAUDE.md` files from the project root (and parent directories) into every session as system-level instructions. When those instructions say "use GraphRelay first," the model follows them consistently — the same way a new engineer picks up a team's coding standards from a style guide instead of guessing.

There's no programmatic hook in Claude Code that forces tool-call ordering — the CLAUDE.md instruction really is the mechanism here, not a workaround for a missing one. And it works well in practice: our own 40-task retrieval benchmark (`search_code`, both semantic and no-semantic mode) hits 100% when GraphRelay is used, against 0% for the no-GraphRelay baseline (see the main README's Benchmarks table). One honest gap: we haven't run a controlled A/B that isolates the instruction itself — GraphRelay available, but the agent *not* told to prefer it — so we can't yet say precisely how much of that gap is the tool versus the instruction telling Claude to use it. That's a real experiment we just haven't run.

---

## Placing the file

**Project-level (recommended):** `<repo-root>/CLAUDE.md`

Applies to all Claude Code sessions opened in this project. Commit it to the repo so your whole team benefits automatically.

**Global (personal preference):** `~/.claude/CLAUDE.md`

Applies to every project you open. Useful if you want GraphRelay-first behavior everywhere you've indexed.

---

## Full example CLAUDE.md

Here is a complete `CLAUDE.md` that combines the GraphRelay hook with typical project conventions:

```markdown
# Project conventions

## Code search — use GraphRelay first

Before reading files, grepping, or exploring the codebase, always query GraphRelay MCP tools:

- `search_code` — intent-based search (ranked symbols with snippets and call graphs) or
  named-type lookup (all implementations of an interface or pattern)
- `inspect` — a known symbol's definition + callers/callees, or a file's/module's contents
  (symbol map near a line, or a high-level directory summary, depending on what you pass)
- `trace_impact` — before changing anything, check blast radius
- `watch_status` — check index freshness before trusting results

Only fall back to direct file reads when GraphRelay cannot answer (config files, non-indexed content).

## Style

- TypeScript strict mode — no `any`
- Tests live next to source files (`foo.test.ts`)
- `pnpm` for package management

## Commands

- `pnpm test` — run tests
- `pnpm build` — production build
- `pnpm lint` — lint + type check
```

---

## Verifying it works

After adding the CLAUDE.md block, open Claude Code in the project and ask a question that requires understanding the codebase:

```
How does user authentication work in this project?
```

In the tool call log you should see `search_code` invoked **before** any `Read` or `Bash` calls. If Claude skips straight to reading files, double-check the CLAUDE.md is in the project root and the GraphRelay MCP server is connected (`claude mcp list`).

---

## Tool reference

| Tool | When to use |
|---|---|
| `search_code(intent?, implements?, top_n?)` | `intent` for anything concept/intent-based; `implements` when looking for all implementations of a named interface/trait/base class — exactly one required |
| `inspect(symbol?, path?, focus_line?, include_source?, max_symbols?)` | `symbol` when you already know the function/class name (definition + callers + callees); `path` when you know the file or directory (symbol-level context, or a module summary if it's a directory) — exactly one required |
| `trace_impact(symbol, depth?)` | Before modifying a symbol — see what depends on it |
| `watch_status()` | When results seem off — check if the index is stale |

**tip:** `search_code` with `intent` is the workhorse. Use it for anything intent-based. Reserve `inspect` with `symbol` and `trace_impact` for when you already know the symbol name.
