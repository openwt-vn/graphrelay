# Plan 23 — MCP payload sizing and tool consolidation

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

**Status: both phases done, 2026-09-28.** See `checklist.md`'s "Plan 23"
section for the day-by-day account with real numbers; this document states
the design and invariants that section's work followed.

## Source

A head-to-head LLM-graded benchmark against a competitor tool (aliased
`codegraph-colby` throughout this project's history, never named directly —
see the "Erpnext corpus re-pin" section of `checklist.md`) found that, after
fixing an unrelated corpus-staleness bug, answer quality was near-parity
(67.1% vs 68.3% mean coverage on erpnext Q01-Q05) but codegraph used far
more tool calls (51 vs 32) and cost ($8.17 vs $5.38) for the same 5
questions. A research fork cloned the competitor's real open-source
repository and read its actual indexing/retrieval/MCP source alongside ours.

**Two mechanisms explain the gap, not a smarter retrieval algorithm** — the
competitor's substrate is the same family as ours (SQLite + FTS5 + graph, no
embeddings, no vector search, no LLM-in-the-loop reranker):

1. Their main tool returns near-full source per result, within a
   repo-size-tiered character budget, so the calling agent rarely needs a
   follow-up `Read`. Ours capped every snippet at a flat 600 chars
   regardless of repo size.
2. They ship more tools internally than they list to the calling agent by
   default — their own code comments cite internal A/B testing: more listed
   tools causes more agent mis-picks (a 3-tool surface regressed cost; a
   1-tool surface was catastrophic), settling at roughly 4-5 as the floor.

## Success criteria

- Snippet/response sizes scale with repo size instead of one flat constant
  for every project.
- `get_file_context` can serve as a direct `Read` substitute for a known
  file.
- Retrieval **ranking** is provably unaffected by the payload-sizing change
  — verified via bit-identical `strict-rank-eval.py` MRR/pass@5 on all three
  benchmark corpora (lexical, erpnext, keycloak).
- The listed MCP tool surface shrinks from 7 to 4, reducing agent mis-pick
  surface, without rewriting any of the underlying retrieval/graph logic.
- Every real (non-doc) call site of the old 7 tool names across the whole
  repo — not just the MCP server — is found and updated in the same change,
  including this project's own MRR ship-gate scripts.
- `cargo build/test/clippy --workspace` clean throughout.

## Invariants touched (deliberately, with sign-off)

- **Supersedes "MCP tools stay at 7."** The new invariant is **"MCP tools
  stay at 4"** (`search_code`, `inspect`, `trace_impact`, `watch_status`).
  This is the same class of deliberate, sign-off-gated change as the 200MB
  binary-size gate raise (`CLAUDE.md`), not a silent violation — the user
  explicitly asked to "reduce, merge the tools to less" after being told
  this touches a stated hard invariant.
- Does **not** touch the retrieval pipeline invariants (SQLite-upsert-before-
  chunk-build, rerank weight sum, symbol ID scheme, `chunk_fts` cascade
  delete) — none of Plan 23's changes are in `codegraph-core`'s retrieval
  stages.

## Design

### Phase 1 — payload sizing

Repo-size-tiered budget (`response_budget()`, `crates/codegraph-mcp/src/tools/mod.rs`),
keyed off `total_files` sourced from already-in-memory `watch_status` state
(not a fresh DB query — `GraphDb`'s connection mutex is a documented
contention risk under load):

| `total_files` | `per_item_chars` | `total_chars` | `per_item_floor_chars` |
|---|---|---|---|
| < 500 | 4000 | 24000 | 300 |
| 500–5000 | 2500 | 24000 | 300 |
| > 5000 | 1500 | 24000 | 300 |

`total_chars` held constant across tiers deliberately: erpnext/keycloak
(large) already show lower MRR than lexical (small) — a large repo needs
more candidate *breadth* visible, not more depth per candidate.
`size_snippets_fair_share()` walks results in rank order,
`fair_share = remaining_total / remaining_items` clamped to
`[floor, per_item_chars]`, deducting what was actually kept from the running
total — no ranked result is ever dropped, only shrunk toward the floor.
Truncation always falls on a line boundary
(`truncate_snippet_on_line_boundary()`) — a mid-line cut isn't usable in
place of a real `Read`.

Applied to:
- `find_relevant_code` — replaces the flat `truncate_snippet(&r.snippet, 600)`.
- `explain_symbol` — replaces the flat `truncate_snippet(&snippet, 800)`. Not
  in the originally-approved item list; added because once `find_relevant_code`
  can carry up to 4000 chars, `explain_symbol` (soon `inspect`'s symbol mode)
  returning *less* source than a fuzzy search hit recreates the exact
  extra-follow-up-call problem this plan exists to fix.
- `get_file_context` — new opt-in `include_source: bool` field. When true,
  returns the whole file as `"{1-based line}: {text}"` (matching `Read`'s own
  convention, via a new `SourceLoader::full_text()` thin wrapper around the
  existing private `load()`), centered on `focus_line` when given via a
  balanced outward-expansion window, budget-capped the same way.

### Phase 2 — tool consolidation

Grouped by **what shape of question the agent has**, not by what shape of
answer a tool returns — the latter axis was tried first and produced a
`trace_impact`+`summarize_module` merge sharing nothing conceptually, with
`symbol`/`path` as an ambiguous discriminator duplicated across two
different merged tools. Discarded in favor of:

1. **`search_code`** = `find_relevant_code` (`intent`) + `find_implementations`
   (`implements`) — exactly one required.
2. **`inspect`** = `explain_symbol` (`symbol`) + `get_file_context` +
   `summarize_module` (both via `path`) — exactly one of `symbol`/`path`
   required. `path` auto-classifies via a new shared `resolve_file_lookup`/
   `PathLookup` helper: resolves to exactly one indexed file → file-context
   behavior; doesn't resolve to a file → module-summary behavior; resolves
   to multiple files ambiguously → **hard error, never a silent fallback**
   (the one place presence-inference could otherwise quietly answer a
   different question than the one asked).
3. **`trace_impact`** — left unmerged. Reads as a single unambiguous verb;
   gluing it to anything else was the weak point of the discarded design.
4. **`watch_status`** — unchanged. Structurally the outlier (no args, flat
   health-check shape).

Presence-inference (not an explicit `mode` enum) is deliberate: a `mode`
field just adds a second value that can disagree with the params actually
supplied — a new failure class, not fewer. The router treats "both given"
and "neither given" as hard, named `-32602`/`isError` errors, never a silent
priority order.

Implemented as a **pure routing layer** — `tools/search_code.rs` and
`tools/inspect.rs` call straight into the existing, unmodified
`find_relevant_code::handle`/`find_implementations::handle`/
`explain_symbol::handle`/`get_file_context::handle`/`summarize_module::handle`.
None of the five underlying handler modules' retrieval/graph logic was
rewritten.

### Real breakage found and fixed (validated via a whole-repo grep, not just the MCP server)

- `scripts/strict-rank-eval.py`, `scripts/llm-task-eval.py` — this project's
  own MRR ship-gate scripts. Highest-severity item: a silent break here
  could be misdiagnosed as a retrieval regression rather than a rename.
- `scripts/agent-bench.py`, `scripts/vs-graphify.py` — wire-level `_mcp_call`
  tool-name arguments.
- `src/main.rs` — `codegraph init`'s `SETUP_HOOK_BLOCK` (renamed to the new
  4 tools) and `SETUP_HOOK_MARKER` (was `"find_relevant_code"` — deliberately
  decoupled from any single tool name, now the block's own stable heading
  text, so a future rename can't silently break `codegraph init`'s own
  "already injected" detection the same way again).
- `.claude/settings.local.json` — this project's own MCP permission
  allowlist (5 old entries → 4 new).
- `crates/codegraph-mcp/tests/mcp_server.rs` — tool-count/name tests
  (including a pre-existing bug: a test named `..._contains_5_tools` whose
  body actually asserted 7 — fixed to be correctly named and assert 4), ~13
  `call_tool`/`dispatch` call sites, 3 new tests for the merged tools'
  exactly-one-of-two-args error paths.
- `CLAUDE.md` — updated following its own established convention of
  annotating a correction rather than deleting old text (see how it
  documented Plan 20's vector-search removal): the hard invariant, the
  crate-map bullet, the now-reversed "8th flagship tool" framing, the ship
  gate line, and — highest priority — the top-of-file "Working rules for
  agents" instructions every agent session (including this one) actually
  follows.
- **Confirmed not at risk**: `codegraph-api`'s REST routes (independent
  `ApiState`, zero MCP-tool-name coupling); the benchmark harness's own
  `mcp_tools` permission config (`mcp__codegraph__*` wildcard, not per-tool).

## Verification

- `cargo build/test/clippy --workspace` clean after each phase.
- Phase 1: bit-identical `strict-rank-eval.py` MRR/pass@5 on all three
  corpora vs. the pre-change baseline (lexical 0.2783/17-40, keycloak
  0.5887/33-40, erpnext 0.1910/30-90) — proves ranking is untouched.
  Live-verified against the running daemon: a real result's snippet went
  from 600 chars to full body (2444 chars); `include_source` returned a
  2093-line file correctly windowed and 1-based-numbered.
- Phase 2: manual `tools/list` (exactly 4, correct schemas) and `tools/call`
  exercises for every branch of `search_code`/`inspect` against the live
  daemon; re-ran `strict-rank-eval.py` post-consolidation — bit-identical to
  the Phase-1-verified baseline.
- Two separate commits (payload sizing, then consolidation) so any change in
  the harness's tool-calls-per-question metric can be attributed to the
  right cause, per this project's own convention (Plan 22 item 3: measure
  each arm on its own).

## Explicitly out of scope / flagged for a separate decision

- **Item 5 from the original investigation** (grouping `find_relevant_code`
  results by file instead of a flat per-symbol list) — not approved, would
  change response shape in a way that needs `strict-rank-eval.py` updated in
  the same change (it reads a flat `results[].symbol` field), same shape of
  risk as the corpus-staleness bug this session already caught once.
- **Adaptive/reduced *listed* tool surface per repo size** (the competitor's
  single biggest lever — all tools exist, but fewer are listed for small
  repos) — investigated and recommended as its own future decision, not
  bundled into this consolidation. This plan implements a flat 4-tool
  surface for every project size.
- Re-running the erpnext competitor comparison with the new 4-tool surface
  to measure the actual tool-calls-per-question effect — flagged as the
  final verification step, not yet executed as of this document.
