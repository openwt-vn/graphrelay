# Plan 22 — Retrieval hygiene and agent-facing signals

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

Source: five candidate techniques identified during external research into
code-search/IR practice (2026-09-27), each re-verified here against **this**
codebase — the exact functions each would touch, whether the motivating
problem is real on our own three benchmark corpora (keycloak, erpnext,
lexical), and what is specific to our architecture (30+ tree-sitter
languages, one shared daemon, a stateless MCP transport). Nothing below is
transcribed on trust; every "measured" claim was measured against the
local corpus checkouts on 2026-09-27, and every "unverified" claim is
labeled as such.

**Headline findings from verification:**

1. **Generated-file detection does not exist today** (only ignore globs
   like `dist/**` / `*.min.js` in `default_ignore_index()`, and the
   path-only `is_test_file` down-rank in `rerank`). But the naive version of
   it would be actively harmful on our own corpora: **514 of erpnext's 3,642
   source files** contain an indented `# begin: auto-generated types` /
   `# This code is auto-generated. Do not modify anything in this block.`
   region *inside a hand-written class* — a loose "generated … do not
   modify" banner match would down-rank ~14% of erpnext, including the real
   implementations most erpnext tasks target. The design has to be built
   around that false-positive hazard, not around the true positives.
2. **Our current corpora cannot show a win for generated-file down-ranking**
   — only a regression gate. Under a strict leading-comment-block rule, the
   only true positives across all three corpora are 38 JAXB-generated
   classes in keycloak's `saml-core-api/…/dom/saml/v2/ac/`, and no keycloak
   benchmark task targets them. Any value claim needs a corpus that
   actually contains codegen output.
3. **File-path mentions in queries are real in our own benchmark suites**:
   16 of 90 erpnext tasks (`erp-48`…`erp-67`) and 4 of 40 lexical tasks
   (`lex-bug-04`, `lex-sg-02`, `lex-sg-12`, `lex-sg-15`) name a file or a
   repo-relative path in the question text. Most resolve to exactly one
   indexed file; three don't resolve at all (the commit moved or renamed
   the file — a real hazard for pinning).
4. **A structural FTS gap found along the way**: `GraphDb::upsert_fts_batch`
   writes `chunk_fts.file_path` **raw** (absolute path, FTS5 `unicode61`
   tokenizer only), while `qualified_name` and `chunk_text` both get the
   shared `tokenize_name` decomposition + stemming. So a stemmed query term
   essentially never matches the `file_path` column even though `bm25()`
   weights it 2.0: `billing_status.py` → query terms `bill`/`status`, but
   the column holds `billing_status` / `py` as unicode61 tokens. Logged
   here as a finding; fixing it is a measured comparison arm under item 3,
   not silently folded in.
5. **The live-edit staleness window is not the debounce window.**
   `debounce_ms` defaults to **50ms**. The real window is the whole batch's
   processing time — graph writes, the repo-wide `resolve_edges()`, and
   `flush_chunk_batch` — which CI's `rapid_saves_p95_latency_and_no_duplicates`
   has measured at ~10.8s (see `checklist.md`). During that window
   `SourceLoader` can hand back a silently wrong snippet (details in item 2).
6. **`PPR_ITERATIONS = 4` is ~32% short of its fixed point.** With restart
   probability α = 0.25, un-restarted mass after k iterations is
   (1−α)^k: 0.75⁴ ≈ 0.32; getting under 1e-3 needs k ≈ 24. That follows
   directly from our own constants, and it's the reason a sweep is worth
   doing at all. The value was set for latency and
   has never been swept.
7. **Session-level response dedup is blocked by the current transport, not
   just un-built.** `handle_initialize` advertises protocol `2024-11-05`
   over a plain `POST /mcp`; the server issues no session identifier; one
   daemon is shared across subagents and parallel agent sessions. There is
   currently no way to tell two callers apart.

## Tiering

| Tier | Items | Why this tier |
|---|---|---|
| 1 — additive, no ranking change | 2 (possibly-stale signal) | Pure response-schema addition; cannot move any benchmark number; normal verification bar. |
| 2 — ranking changes, single-variable, full three-corpus bar | 3 (path pinning), 1 (generated-file down-rank) | Change what gets returned; item 3 has a real diagnostic subset, item 1 is regression-gate-only on current corpora. |
| 3 — empirical sweep | 4 (PPR iterations) | No right answer without measurement; coupled to Plan 21 item 16's open β/γ/damping sweep. |
| 4 — design discussion only | 5 (session dedup) | Architecturally invasive, blocked by the transport; needs measurement and a design decision before any spec. |

Order within Tier 2: item 3 before item 1 (item 3 has a measurable subset
and changes PPR seeding, which item 1 doesn't; landing it first gives item
1 a stable baseline). Tier 3 runs only after every Tier 2 item that ships
is committed — both Tier 2 items change the candidate pool that PPR walks
from.

---

## Tier 1

### Item 2 — per-result "possibly stale" signal

**What exists today:** only aggregate signals — `IndexStatus.pending_count`
(set per live-loop batch in `IndexWorker::run`), `semantic_pending` (FTS
backlog), and `watch_status`'s `state`. Per file, there is the checksum-0
sentinel (`index_path_graph_only` writes `checksum = 0` until
`flush_chunk_batch`'s FTS write commits) — a real per-file "graph written,
FTS not yet" marker that nothing on the query side reads. No tool response
says "*this particular result* may be out of date."

**The concrete harm** (`crates/codegraph-core/src/retrieval/source_loader.rs`):
`SourceLoader::snippet` slices the file by the symbol's *indexed*
`span.start_line..=end_line`. `SourceLoader::invalidate(path)` is only
called from `index_path_graph_only`, i.e. *after* the watcher has picked the
file up. Between a save and that point:
- **Cache miss** → `load()` reads the *new* disk content and slices it with
  the *old* spans: a silently shifted/wrong snippet (the existing
  `"symbol span out of bounds — stale index entry?"` warning only catches
  the case where the file got shorter than the span).
- **Cache hit** → the snippet is consistent with the index but stale
  relative to disk; the agent may then edit against lines that no longer
  exist.
- **Deleted file** → symbols stay indexed until the watcher processes the
  removal; `load()` fails, snippet is empty, the result still appears.

**Design:**
- Detection is a cheap per-result `stat`, evaluated once per distinct
  result file at response-build time, not a hook into the watcher's
  internal debounce state (which lives inside `FileWatcher::next_batch` and
  isn't shared with any query path). A result is `possibly_stale` when any
  of:
  1. the file no longer exists / `stat` fails;
  2. disk mtime is later than `files.indexed_at` (RFC3339, set in
     `Extractor::build_extraction`);
  3. disk size ≠ `files.byte_size`;
  4. `files.checksum == 0` (the pending-FTS sentinel).
  This covers the debounce window, the in-flight batch, a watcher overflow
  awaiting its full re-index, and edits made while the daemon was down —
  none of which a watcher-internal hook alone would cover.
- Known false-negative to document, not hide: `indexed_at` is stamped in
  `build_extraction` *after* the file was read, so an edit landing between
  the read and the stamp can have mtime < `indexed_at`. Rule 3 catches most
  of these; rule 4 catches the rest until the FTS flush. Mtime resolution
  on some filesystems (1–2s) is the same class of edge case.
- **Optional, only if the stat approach proves insufficient**: a shared
  `DashSet<PathBuf>` of "event seen, not yet indexed" paths, inserted in
  `FileWatcher`'s notify callback and cleared after that path's
  `flush_chunk_batch`. Needs plumbing an `Arc` from `codegraph-watcher`
  through `src/server.rs` into `McpAppState`/`ApiState`; only covers
  watcher-observed changes. Not the default.
- **Where it surfaces** — the 4 tools that return snippets and/or line
  ranges: `find_relevant_code`, `explain_symbol`, `find_implementations`,
  `get_file_context` (`crates/codegraph-mcp/src/tools/`). A helper next to
  `truncate_snippet`/`capped_names` in `tools/mod.rs` takes `&GraphDb` + a
  path and returns the flag (via `file_by_path`, indexed exact match). The
  field is **emitted only when true** — zero token cost in the common
  case — as `"possibly_stale": true` on the affected result, plus one
  top-level `note` telling the agent to read the file directly rather than
  trust the snippet/line range (same mechanism as `find_relevant_code`'s
  existing `concentration_note`). `trace_impact`/`summarize_module` return
  names and files only; out of scope for the first pass.
- Optionally mirrored on REST `/api/search` results (same helper,
  `codegraph-api`) — not required for this item.

**Invariants:** a response-schema change on existing tools only (the same
precedent as `jev_active` on `watch_status`); still exactly 7 MCP tools;
no change to retrieval or ranking; `codegraph-core` stays I/O-free (the
check lives in the tool layer, not the pipeline).

**Verification (normal bar):** unit tests for each of the four staleness
conditions using a temp file + a real `GraphDb` (modify after index; grow/
shrink; delete; checksum-0 sentinel); a test that a fresh, unchanged file
produces **no** field at all; a live check editing a file under a running
daemon and querying before the batch completes. `cargo build/test/clippy
--workspace` clean. No benchmark run needed — benchmark corpora are static.

---

## Tier 2

Shared validation bar for both items (this project's established bar, see
Plan 21): strict scorer (`scripts/strict-rank-eval.py`, pass@1/pass@5/MRR)
on **all three corpora**, lenient eval alongside, each measurement
reproduced ≥2× on a fully-settled index (`semantic_pending: 0`, not just
`state: fresh`), one daemon at a time, SCIP state recorded per corpus
(SCIP is now live on all three — hold it fixed across before/after), and
at least one neighboring constant value checked for a plateau rather than
a knife-edge. No per-task anecdote alone justifies shipping.

### Item 3 — file-path pinning in query parsing

**What exists today:** `GraphDb::files_by_suffix` (built for
`get_file_context`/`/api/file/*` to accept repo-relative paths) already
resolves `accounts/…/billing_status.py` or a bare `LexicalEditor.ts` to
indexed files, LIKE-escaped. It is not wired into retrieval at all.
`tokenize_query_split` shreds a path through `split_words` into generic
stems (`runs/[runId]/+page.svelte` → `run`, `page`, `svelt`; the `Id` half
of `runId` falls under the 3-char floor) that then `OR`-match across the
whole index, and — per headline finding 4 — never match the `file_path`
column itself.

**Measured motivating subset** (diagnostic only — see bar above):

| Corpus | Tasks naming a path/file | Resolve to exactly one indexed file | Don't resolve |
|---|---|---|---|
| erpnext_90 | 16 (`erp-48`…`erp-67`) | 10 of the 13 spot-checked mentions (the other 6 tasks' paths not yet checked) | 3 path mentions (`…/purchase_receipt/services/stock_reservation.py`, `erpnext/stock/services/serial_batch_bundle.py`, `tax_service.py`) — commit moved/renamed them |
| lexical | 4 | 4 (`LexicalUpdates.ts`, `LexicalEditor.ts`, `LexicalEditorState.ts`, `LexicalTextNode.ts`) | 0 |
| keycloak | 0 | — | — |

Two erpnext tasks (`erp-58`, `erp-62`) name *two* paths — a
refactor moving code from one file to another — so the named file is not
always where the answer lives.

**Design:**
- **Detection** (`codegraph-core/src/retrieval/pipeline.rs`, before
  `tokenize_query_split`): scan the raw query for path-like substrings —
  a token containing `/` or `\`, or a bare `name.ext` where `ext` is
  alphanumeric and 1–5 chars. Don't hard-code a 30+-language extension list
  in `codegraph-core` (it can't depend on `codegraph-parser`); instead the
  store lookup *is* the validator — a substring only counts as a path if
  it resolves to ≥1 indexed file. Strip surrounding backticks/quotes/
  trailing punctuation first.
- **Resolution**: new `GraphStore` trait method with an empty default
  (same pattern as `lexical_search`, so mocks and `codegraph-core` stay
  I/O-free), implemented in `GraphDb` over `files_by_suffix` +
  `symbols_in_file`. Note `files_by_suffix` is a leading-wildcard `LIKE`
  (full `files` scan) — fine once per path mention per query on
  ~10k-file corpora, but measure it; Plan 21's SCIP merge showed how badly
  this pattern scales when called per-row.
- **Ambiguity policy**: exactly one match → pin. 2–3 matches → pin all
  (a bare `status.py` is genuinely ambiguous; the user named it, so all
  candidates are fair). More than 3 → don't pin (too vague to trust), leave
  the text in the FTS query.
- **Per-file cap**: a large file (erpnext's `accounts_controller.py`-sized)
  can hold hundreds of symbols, and each admitted candidate costs
  `enrich()` a `callers_of`/`callees_of`/snippet load. Admit at most
  `PATH_PIN_MAX_SYMBOLS` (start at 20, provisional) per pinned file,
  chosen by FTS score of the *remaining* query text restricted to that
  file (`rowid IN (SELECT id FROM symbols WHERE file_id = ?)`), falling
  back to span order if the remaining text matches nothing in the file.
- **Admission and scoring**: pinned symbols enter `expand_graph` as direct
  hits (direct hits are already never capped). Rank is still decided by
  `rerank`, plus a flat `PATH_PIN_BONUS` (provisional, swept per the bar)
  — "guaranteed admission," not a hard override of ranking, since the
  named file isn't always where the answer lives (`erp-58`/`erp-62`).
- **PPR seeding**: pinned symbols seed personalized PageRank with their own
  lexical score (floored at the existing `1e-3`), *not* a uniform weight —
  seeding 20 symbols from one file at full weight would swamp the walk
  with that file's call neighborhood.
- **Penalty bypass**: a pinned file is exempt from `TEST_FILE_PENALTY` and
  from item 1's generated-file penalty — the user named it explicitly.
- **`lang_filter` still applies** after enrichment, unchanged.

**Measured arms (single-variable each, full bar):**
- A: pin + **strip** the path substring from the FTS query text.
- B: pin + **keep** the path text in the FTS query (stripping discards real
  signal words like `billing`/`status` — this is a genuine open question,
  not a formality).
- C (separate change, separate measurement, only after A/B): tokenize
  `chunk_fts.file_path` with `tokenize_name` like the other two columns —
  a content-format migration (`010_…sql`, `DELETE FROM chunk_fts` +
  startup backfill, same pattern as 008/009). This changes every query,
  not just path-bearing ones, so it gets its own before/after on all three
  corpora and does not ride along with pinning.

**Tests:** detection unit tests (repo-relative path, bare filename,
Windows separators, backticked path, a URL-like string that must not
resolve, a path that resolves to nothing), ambiguity policy (1 / 2–3 / >3
matches), the per-file cap, and a pipeline test with a mock `GraphStore`
proving a pinned symbol is admitted even when FTS alone would miss it.

### Item 1 — generated-file detection and soft down-rank

**What exists today:** nothing generated-specific. `default_ignore_index()`
(`codegraph-core/src/config.rs`) excludes `dist/**` and `*.min.js`
outright (a hard exclude, not a down-rank); `is_test_file` +
`TEST_FILE_PENALTY` in `pipeline.rs` is the closest existing analogue and
the model for this item's shape.

**Corpus scan (2026-09-27, header-bytes grep over `git ls-files` source
files, deliberately loose patterns to find the hazards):**

| Corpus | Source files | Loose banner hits | What they actually are |
|---|---|---|---|
| erpnext | 3,642 | 514 | **All false positives**: an indented `# begin: auto-generated types … Do not modify anything in this block … # end: auto-generated types` type-hint region inside hand-written doctype classes (e.g. `accounts/doctype/account/account.py` lines 26–86) |
| keycloak | 9,396 | 38 | **True positives**: JAXB-generated SAML classes (`// This file was generated by the JavaTM Architecture for XML Binding(JAXB)…`), after a license header |
| lexical | 1,067 | 2 | **Both false positives**: `packages/lexical-website/sidebars.js` (prose: "can be generated from the filesystem") and a test file with `@generated` inside a string literal |

Filename-convention hits (`*.pb.go`, `*_pb2.py`, `*.g.dart`,
`*.generated.*`, `*_generated.*`, `*.designer.cs`, …): **0 in all three
corpora.**

**Consequence**: on today's benchmarks this item is at best a no-op and at
worst a large regression (if the rule is loose). Its value is for codebases
with real codegen output (protobuf/gRPC stubs, ORM/JAXB models, `mockgen`
mocks, `build_runner` output) — which none of our three corpora are.

**Design:**
- **Filename rules** (`is_generated_file`, next to `is_test_file`): a short,
  conservative list of unambiguous codegen conventions — `.pb.go`,
  `_grpc.pb.go`, `.pb.h`/`.pb.cc`, `_pb2.py`/`_pb2_grpc.py`, `.g.dart`,
  `.freezed.dart`, `.designer.cs`, `.g.cs`, `zz_generated.*`,
  `*.generated.*`/`*_generated.*`. Deliberately not per-language exhaustive;
  the banner rule below is the general mechanism.
- **Content-banner rule — comment-syntax-agnostic, anchored, leading-block
  only.** Rather than maintaining a per-language banner table across 30+
  languages (unmaintainable, and the table would drift the same way the
  watcher's old hand-written extension map did), scan only the file's
  **leading comment block**: lines from the top of the file, stopping at
  the first line that isn't blank or a comment (a line whose first
  non-whitespace chars are a comment token: `//`, `#`, `/*`, `*`, `--`,
  `;`, `%`, `<!--`, `(*`, `'`, `!`, `REM`, capped at the first ~40 lines /
  4KB). Within that block, match a small set of phrase patterns after
  stripping the comment token: `^Code generated .* DO NOT EDIT\.?$`
  (a widely adopted cross-tool convention), a standalone `@generated`
  token, `<auto-generated`, and a same-line co-occurrence of
  `generated`/`auto-generated`/`autogenerated` with `do not edit`/
  `do not modify`/`DO NOT EDIT`. Shebang and encoding lines (`#!`,
  `# -*- coding`) count as comments.
  - Rejects erpnext's region by construction: that region starts after
    `import` and `class` lines, and is indented.
  - Rejects lexical's `@generated`-in-a-string (not in a comment, not in
    the leading block) and `sidebars.js` (prose doesn't match an anchored
    pattern).
  - Accepts keycloak's JAXB banner (inside the leading comment block,
    after the license header).
  - Known incompleteness, documented not hidden: languages whose
    generators emit no banner, or emit it after a `package`/`module` line,
    are missed. A miss costs nothing (status quo); a false positive costs
    ranking. Bias toward misses.
- **Where the content check runs — query time, via `SourceLoader`.**
  `enrich()` already calls `SourceLoader::snippet` for every candidate,
  which loads and caches the *whole file*. A banner check on the first
  few KB of that cached content is essentially free. Add
  `is_generated: bool` to `EnrichedCandidate`, set in `enrich()`, cache the
  per-path verdict alongside the content (invalidated by the same
  `SourceLoader::invalidate`). No migration, no re-index, no `Symbol`/
  `GraphStore` schema change. Alternative if profiling says otherwise: an
  index-time `files.generated` column set in `Extractor::build_extraction`
  (source bytes already in hand) — needs a migration + backfill, not the
  default.
- **Penalty**: `GENERATED_FILE_PENALTY`, flat subtraction in `rerank`'s
  scoring closure (same reasoning as `TEST_FILE_PENALTY`: subtracted, not
  multiplied). Starting value 0.15, **provisional and unswept** — there is
  no corpus on which a sweep can show a win yet. **Combination with the
  test penalty: `max`, not sum** (a `mockgen` mock under `/test/` is one
  kind of noise, not two; summing would double-punish exactly the files
  the brief cares least about while stacking to 0.30, a scale never
  validated).
- **Query gate**: skip the penalty when the query is about generated code
  (`generated`, `codegen`, `proto`, `protobuf`, `stub`, `schema` — raw
  query text, word-boundary matched, same approach as
  `query_mentions_tests`). Pinned files (item 3) are exempt.
- **Hand-written mocks** (keycloak has 11 mock-ish paths) are **out of
  scope** — a `mocks/` directory is a path convention with a very
  different false-positive profile; if wanted, it's a separate item.
- **Partial-file generated regions** (erpnext's type-hint block) are out
  of scope — the symbols inside such regions are declarations, not the
  functions agents search for.

**Tests (these pin the corpus-scan findings as permanent regression
fixtures):** an erpnext-shaped Python file with the indented in-class
region → **not** generated; a TS test file containing `"@generated"` in a
string → not generated; a `sidebars.js`-shaped prose comment → not
generated; a JAXB-shaped Java file with license header then banner →
generated; a Go `// Code generated by protoc-gen-go. DO NOT EDIT.` file →
generated; a `_pb2.py` with encoding line + banner → generated; a filename
rule hit with no banner → generated; query-gate on/off; `max` combination
with the test penalty.

**Validation:** the three-corpus strict scorer is a **regression gate
only** — expected result is "unchanged" on lexical/erpnext and
"unchanged or noise-level" on keycloak; any erpnext movement at all means
the rule is leaking and must be fixed before shipping. To claim value,
add a fourth, small corpus that actually contains codegen output (e.g. a
Go or Python service with checked-in protobuf/gRPC stubs) with a handful
of hand-written tasks whose correct answers are the real implementation
beside a generated stub. Until that exists, ship (if at all) labeled
"regression-neutral, value unmeasured," the same honesty standard
`SYNONYM_TERM_WEIGHT` was shipped under.

---

## Tier 3

### Item 4 — re-sweep `PPR_ITERATIONS`

**What exists today** (`crates/codegraph-core/src/retrieval/pipeline.rs`):
`PPR_ITERATIONS = 4`, `PPR_RESTART_PROB = 0.25`, `PPR_MAX_NODES = 100`,
documented as "a bounded approximation for latency (ship gate: p95 ≤
100ms), not a converged-to-precision PageRank." Never swept.

**Cost model (from the code, to be confirmed by measurement):** each
iteration visits ≤ `PPR_MAX_NODES` (100) nodes; `callees_of` results are
memoized in `neighbor_cache`, so extra iterations only pay a DB call for
nodes reaching the top-100 for the first time. Extra iterations are
therefore likely cheap relative to `enrich()` (which does `callers_of` +
`callees_of` + a snippet load for every one of ~100 candidates) — but
"likely" is exactly what this item exists to replace with a number.

**Confounders the sweep must control:**
- **Coupling with existing constants.** β (0.667) and
  `DIRECT_HIT_GRAPH_CREDIT_DAMPING` (0.75) were both tuned against the
  4-iteration score distribution. More iterations spread mass deeper
  (beyond the 1-hop candidate pool) and flatten it among pool members,
  changing what `graph_score / max_graph_score` means. Run this jointly
  with — or immediately before — Plan 21 item 16's still-open β/γ/damping
  sweep, not as an independently-shipped constant change.
- **SCIP is now live on all three corpora**, so `callees_of` walks a
  denser graph than any earlier tuning saw. Fix and record SCIP state per
  corpus for the whole sweep.
- **Truncation.** `PPR_MAX_NODES` truncation each iteration means the walk
  converges to a *truncated* fixed point, not true PPR. Log per-iteration
  L1 delta ‖scoresₖ − scoresₖ₋₁‖₁ so "converged" is observed, not assumed.
- **Determinism.** `next`/`scores` are `HashMap`s whose iteration order is
  randomized per process; float accumulation order can differ run to run.
  Repeat each configuration and confirm top-N stability (the project has
  been bitten by pool-size-level nondeterminism before — see
  `getActiveEditorState` in `CLAUDE.md`).

**Procedure:**
- **Step 0 — latency baseline that doesn't exist yet.** The only p95
  numbers on record are from the synthetic oracle corpus
  (`benchmarks/src/main.rs`); `scripts/strict-rank-eval.py` records no
  timing. Add per-call wall-time capture to the eval script (client side)
  and a temporary `tracing` span per pipeline stage (server side) so
  PPR's share of query time is known on real corpora before sweeping.
- **Step 1 — sweep** `PPR_ITERATIONS ∈ {2, 4, 8, 12, 25}` plus a variant
  with an L1-delta early stop (e.g. stop when delta < 1e-3, max 25).
  Metrics per value: strict pass@1/pass@5/MRR on all three corpora, p50/p95
  latency on real corpora, L1-delta curve, top-N determinism across
  repeats.
- **Step 2 — decide.** Ship a new value only if it's on a plateau (≥2
  neighbors comparable), reproduces ≥2×, and keeps real-corpus p95 ≤
  100ms. "4 stays" is an acceptable, documented outcome.

**Process lessons carried from Plan 21 item 16 (mandatory):** one daemon
at a time on one port; gate every measurement on `semantic_pending: 0`;
switch configurations by swapping prebuilt binaries (or a temporary env/
config override), **never** by `git checkout` in a working tree with
uncommitted work; no long-running background sweep that a task-lifetime
limit can silently kill mid-run.

---

## Tier 4 — design discussion only (not an implementation spec)

### Item 5 — session-aware response deduplication

**Idea:** remember which source content a given agent session has already
received, and on a later call return a compact back-reference instead of
re-sending byte-identical snippets.

**Why this is not specced for implementation:**
- **No session identity exists.** `crates/codegraph-mcp/src/server.rs`
  exposes a plain `POST /mcp`; `handle_initialize` advertises protocol
  `2024-11-05` and returns no session identifier. Later MCP revisions
  define an HTTP transport with a server-issued session header (believed
  to be the 2025-03-26 revision's Streamable HTTP transport — **verify
  against the spec before designing on it**); adopting it is a transport
  upgrade in its own right, with its own client-compatibility testing.
- **One daemon, many callers.** Subagents, parallel Claude Code sessions,
  and other MCP clients all hit the same daemon and port. Keying on
  anything weaker than a real session id (IP, timing) would cross-
  contaminate agents.
- **Client-side context loss makes back-references a correctness risk,
  not just an efficiency trade-off.** MCP clients compact, summarize, or
  truncate earlier tool results. A back-reference to content the agent no
  longer holds is worse than a duplicate — the agent silently loses the
  snippet. Any design needs an escape hatch (e.g. a `full: true` re-fetch
  argument on existing tools) and must default to off.
- **Stateless-per-call is a current architectural property** (per
  `CLAUDE.md`'s description of the MCP layer); session state would add
  memory growth, eviction policy, and daemon-restart semantics.
- **The 7-tool invariant** means a re-fetch path must be an argument on
  existing tools, not a new tool.

**Step 0 before any design work — measure whether the problem is big
enough:** use the existing `codegraph_mcp::tool_call` logging (in
`handle_tools_call`) plus saved benchmark agent transcripts to compute the
actual fraction of snippet bytes that are byte-identical repeats within
one agent run. If duplicate content is a small share of tool-output
tokens, drop the item. Cheaper alternatives to weigh against it first:
tighter default `truncate_snippet` limits, and a caller-supplied
`exclude_symbols` argument (client-driven, stateless, no session needed).

**Deliverable for this item:** a short design note with the measured
duplicate rate and a go/no-go recommendation — not code.

---

## What needs the full validation bar

- **Items 1 and 3** (ranking changes): strict scorer on all three corpora,
  ≥2 reproduced runs on a settled index, neighbor-value plateau check for
  each new constant (`PATH_PIN_BONUS`, `PATH_PIN_MAX_SYMBOLS`,
  `GENERATED_FILE_PENALTY` — the last only once a corpus exists that can
  move it), zero-regression requirement on the other two corpora. Item 3's
  arm C (file_path tokenization) is measured separately from A/B.
- **Item 4**: the full sweep procedure above, plus real-corpus p95 ≤ 100ms.
- **Item 2** needs only the normal bar (unit + live check + workspace
  clean) — it can't change a ranking.
- **Item 5**: measurement + design note only.

## Explicitly provisional / unverified

- Every new constant named above is a starting value, not a tuned one.
- The PPR cost model is inferred from code, not yet measured.
- The MCP session-transport revision date in item 5 is from memory and
  must be checked against the spec.
- The claim that a fourth, codegen-heavy corpus would show item 1's value
  is a hypothesis; it may show nothing.

## Explicitly not re-attempting

Per `CLAUDE.md` and Plan 21: the own-token-fraction name score, a `bm25`
column-weight rebalance or `chunk_text` length discount, graph-score
caller-credit, the relative-score floor, and ungated recall widening.
Item 3's arm C (tokenizing `file_path`) is **not** a column-weight
rebalance — the weights stay 3.0/2.0/1.0; only the column's content
tokenization changes to match its siblings.

## Execution notes

- Item 2 is independent of everything else and can land first.
- Items 3 and 1 are ranking changes: one at a time, each with its own
  before/after, item 3 first.
- Item 4 only after items 1/3 settle, jointly with Plan 21 item 16.
- Item 5 produces a document, not a diff.
- Update `checklist.md` after each item — don't batch to the end.
- Doc comments written for any of these items should describe the
  technique on its own technical merits; don't attribute it to any other
  tool.
