# Plan 13 — Developer Experience: Auto-setup, Multi-tool Support, Benchmark Fixes

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

## Context

Three problems surfaced from beta feedback (2026-06-30):

1. **Setup friction** — `codegraph init` creates `config.toml` and stops. Steps 4 and 5
   of the setup guide (MCP wiring + CLAUDE.md hook) are manual and easy to miss.
   Pham Hieu's feedback: "even when tool is enabled it only fires when explicitly mentioned
   in the prompt." Root cause: he skipped the instruction-injection step.

2. **Single-tool story** — docs and auto-setup only cover Claude Code + Cursor.
   OpenCode users (Pham Hieu) have a different config format, a different instruction
   file, and no documented path.

3. **Benchmark gaps** — Luu Nguyen's 30-task internal benchmark shows two failure
   patterns vs baseline: R02/R03 code_review fails (codegraph 60% vs baseline 80%),
   and N04 uses 25 tool calls vs graphify's 11.

---

## Success criteria

- `codegraph setup` run in a project root → MCP entry present in the target tool's
  config AND instruction hook present in the correct file — without the user doing
  either manually.
- OpenCode users reach the same accuracy as Claude Code users with the same setup steps.
- R02/R03 code_review accuracy: codegraph reaches parity with baseline (80%).
- N04 call count: ≤ 12 calls (≤ graphify).
- `cargo clippy --workspace -- -D warnings` remains clean.
- `cargo test --workspace` passes.

---

## Stream A — `codegraph setup` subcommand

### What it does

Detects which coding tools are installed on the machine and, for each one found:
1. Injects the MCP server entry into the tool's config file.
2. Injects the "use CodeGraph first" instruction into the tool's instruction file
   (project-scoped, not global, unless `--global` flag is passed).

Both operations are idempotent — running twice does not duplicate entries.

### A1 — Detection

Detect installed tools by probing known binary names and config paths:

| Tool | Binary | Config dir | Instruction file |
|------|--------|------------|-----------------|
| Claude Code | `claude` | `~/.claude/` | `CLAUDE.md` |
| Cursor | *(no CLI)* | `~/.cursor/` | `.cursorrules` |
| OpenCode | `opencode` | `~/.config/opencode/` | `AGENTS.md` |
| Amp | `amp` | `~/.config/amp/` | `AGENTS.md` |
| Codex (OpenAI) | `codex` | `~/.codex/` | `AGENTS.md` |

Detection: `which <binary>` plus existence check of config dir. Any tool where either
passes counts as installed. Print a summary line per tool:

```
  ✓ Claude Code found — will configure
  ✓ OpenCode found    — will configure
  ✗ Amp not found     — skipping
```

### A2 — MCP config injection

Each tool has a different MCP config format. Inject the minimum required entry:

**Claude Code** — `~/.claude/claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "<project-slug>": {
      "type": "http",
      "url": "http://localhost:3030/<project-slug>/mcp"
    }
  }
}
```

**Cursor** — `~/.cursor/mcp.json` (same format as Claude Desktop):
```json
{
  "mcpServers": {
    "<project-slug>": {
      "type": "http",
      "url": "http://localhost:3030/<project-slug>/mcp"
    }
  }
}
```

**OpenCode** — reads `~/.claude/claude_desktop_config.json` directly (same format as
Claude Code). No separate file needed — inject into Claude Desktop config, OpenCode
picks it up automatically.

**Amp** — `~/.config/amp/settings.json` (TBD — probe Amp docs before implementing;
placeholder: write a note file if Amp config format is not yet stable).

**Codex** — `~/.codex/config.toml`:
```toml
[[mcp_servers]]
name = "<project-slug>"
url  = "http://localhost:3030/<project-slug>/mcp"
```

Parsing strategy: read existing JSON/TOML, merge the new entry under the correct key,
write back. Use `serde_json` (already a dep) for JSON files; `toml` (already a dep)
for TOML files. Back up the original file before writing (`.bak` suffix).

### A3 — Instruction-hook injection

Each tool reads a different instruction file. Content is the same block across all of them
(parametrized by tool name only for the preamble).

**Files to write into:**

| Tool | File | Mode |
|------|------|------|
| Claude Code | `<project-root>/CLAUDE.md` | append block if not present |
| Cursor | `<project-root>/.cursorrules` | append block if not present |
| OpenCode | `<project-root>/AGENTS.md` | append block if not present |
| Amp | `<project-root>/AGENTS.md` | same file as OpenCode |
| Codex | `<project-root>/AGENTS.md` | same file |

Detection of "already present": grep for the string `find_relevant_code` in the target
file. If found, skip.

Content to inject (tool-agnostic):

```markdown
## Code search — use CodeGraph first

Before reading files, grepping, or exploring the codebase, always query CodeGraph MCP
tools. CodeGraph returns ranked symbols with call graphs in 1 tool call instead of
many file reads.

- `find_relevant_code` — intent-based search: ranked symbols + snippets + call graphs
- `explain_symbol`     — understand a function with its callers and callees
- `trace_impact`       — blast-radius before changing anything
- `get_file_context`   — symbol map near a line (not raw file text)
- `watch_status`       — check index freshness

Only fall back to direct file reads when CodeGraph cannot answer (config files,
non-indexed content, binary files).
```

### A4 — CLI wiring

Add `setup` subcommand to the existing arg parser in `src/main.rs`:

```
codegraph setup [--global] [--dry-run] [--tool <name>]

  --global      Write instruction hook to ~/.claude/CLAUDE.md (Claude Code) and
                equivalent global files for other tools, not project CLAUDE.md
  --dry-run     Print what would be changed; write nothing
  --tool <name> Only configure the named tool (claude|cursor|opencode|amp|codex)
```

`codegraph init` should call `setup` automatically after creating `config.toml`, with
an interstitial prompt:

```
Configure your coding tools? (Claude Code, OpenCode detected)
  [Y] Yes — inject MCP config and instruction hooks
  [n] No  — I'll do it manually (see docs/setup-guide.md)
```

---

## Stream B — Tool-specific docs

### B1 — OpenCode setup page

Create `docs/opencode-hook.md` following the same structure as `docs/claude-code-hook.md`:

- How OpenCode reads MCP config (via Claude Desktop JSON)
- Where to place the instruction block (`AGENTS.md`)
- Why auto-invocation requires the `AGENTS.md` block (without it, tool only fires when
  explicitly mentioned — matches Pham Hieu's symptom exactly)
- Verification: "Ask OpenCode: how does X work? — you should see find_relevant_code
  called before any file reads."

### B2 — AGENTS.md reference

`AGENTS.md` is the common instruction file for OpenCode, Amp, Codex, and Harness.
Document it once in `docs/agents-md-hook.md` and link from each tool page.

Note: if both `CLAUDE.md` and `AGENTS.md` exist in a project, Claude Code reads
`CLAUDE.md`; OpenCode/Amp/Codex read `AGENTS.md`. Both can coexist with the same
CodeGraph block — `codegraph setup` writes to both.

### B3 — Harness

Harness AI coding assistant (AIDA) config format is not yet confirmed. Placeholder:
add a `docs/harness-hook.md` stub with "format TBD — contact @Hieu when testing"
rather than shipping broken instructions.

---

## Stream C — Benchmark improvements

Source: `https://github.com/OWTVN/owtvn-employee-agent/tree/benchmark/codegraph-vs-graphify`

### C1 — Diagnose R02 (safeFetch) and R03 (PII_PATTERNS)

Both are `code_review` category failures — codegraph and graphify both fail while
baseline passes. This points to a retrieval problem, not a graph-expansion problem
(graphify also fails).

Hypotheses to test:
- **H1 (constant indexing)**: `PII_PATTERNS` is a top-level constant (`const` / module-level
  variable), not a function. The retrieval pipeline may rank it low because in-degree
  weight (β) favors frequently-called symbols; constants are never "called".
  Fix: ensure constants/module-vars are indexed with `SymbolKind::Constant` and
  receive a small base score boost (or set β contribution to 0 for them).

- **H2 (review query mismatch)**: Code review tasks ask "what is wrong with this code"
  or "find the security check" — the intent phrasing doesn't match function names.
  `find_relevant_code("safe external HTTP fetch")` should surface `safeFetch` but
  may not if the docstring/snippet doesn't contain those terms.
  Fix: verify docstring extraction is capturing JSDoc/comments above the function.

- **H3 (snippet truncation)**: The MCP result is capped at 2000 chars; for a long
  function the relevant part may be cut off.
  Fix: ensure snippet starts at function signature, not arbitrary offset.

Steps:
1. Pull the employee-agent benchmark branch and run `codegraph-vs-graphify` tasks
   R02 + R03 manually, inspecting raw `find_relevant_code` results.
2. Confirm which hypothesis is correct before writing code.
3. Implement targeted fix (likely H1 + H2).

### C2 — Diagnose N04 (createResumeRouter, 25 calls)

Navigation task. Codegraph uses 25 tool calls — more than both baseline (37 — baseline
also high) and graphify (11). The agent is not finding `createResumeRouter` on the
first `find_relevant_code` query, so it iterates.

Hypotheses:
- **H1 (routing/resume semantic gap)**: "resume" in routing context (HTTP resume
  session) vs "resume" in document context (CV) — embedding may place these far apart.
  BGE-small may rank wrong results for "resume router".
  
- **H2 (function not in top-K)**: `createResumeRouter` may be ranked below K=5 default.
  The agent gets 5 results, none match, tries a rephrased query — repeating 5× = 25 calls.

Fix strategy:
1. Run `find_relevant_code("create resume router", top_n=10)` against the indexed
   employee-agent repo and check rank of `createResumeRouter`.
2. If ranked > 5: increase default `top_n` from 5 to 8 in the MCP tool schema, or add
   a `--top-n` hint to the instruction block in CLAUDE.md/AGENTS.md.
3. If ranked > 10: the embedding is genuinely far off — consider adding BM25 boost
   for function-name token matches (e.g. exact token match in qualified name gets +0.3
   score bonus).

### C3 — Add failing tasks to local benchmark suite

Copy R02, R03, N04 task definitions from the employee-agent benchmark into
`benchmarks/` so they run as part of `cargo run -p benchmarks`. This prevents
regression once fixed.

### C4 — Re-run after fixes and update RESULTS.md

After C1 + C2 fixes:
1. Re-run the 30-task benchmark on the employee-agent repo.
2. Target: code_review ≥ 80%, N04 calls ≤ 12.
3. Update `RESULTS.md` and report back to Luu Nguyen.

---

## Key invariants (do not break)

- `codegraph setup` must be idempotent — double-run produces identical files.
- Never overwrite a user's entire CLAUDE.md/AGENTS.md — always append the block.
- MCP config backup (.bak) written before any JSON/TOML mutation.
- `find_relevant_code` default top_n change (if made in C2) must not break existing
  MCP tests (`tools_list_returns_exactly_5_tools` etc.).

---

## Step order

1. **C1 diagnosis first** — read the failing tasks before writing any code.
   Retrieval bugs need root cause confirmed, not guessed.
2. **A1 + A2 + A3** — implement `codegraph setup` (no external deps, self-contained).
3. **B1 + B2** — docs for OpenCode (unblocks Pham Hieu immediately).
4. **C2 + C3** — N04 fix + local benchmark registration.
5. **C4** — re-run + report.
6. **B3** — Harness stub (low priority, placeholder only).

---

## Files to create / modify

| File | Action |
|------|--------|
| `src/main.rs` | Add `setup` subcommand; call it from `init` |
| `src/cmd_setup.rs` | New: tool detection, MCP injection, hook injection |
| `docs/opencode-hook.md` | New: OpenCode setup guide |
| `docs/agents-md-hook.md` | New: AGENTS.md reference |
| `docs/harness-hook.md` | New: stub |
| `crates/codegraph-core/src/retrieval/pipeline.rs` | C1 fix (if H1 confirmed) |
| `crates/codegraph-mcp/src/handlers.rs` | C2 fix (if top_n change needed) |
| `benchmarks/src/main.rs` | Add R02, R03, N04 tasks (C3) |
| `RESULTS.md` | Update after C4 re-run |
