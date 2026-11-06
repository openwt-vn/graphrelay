# Plan 21 — Retrieval/indexing/tools audit fixes (Jev-off pipeline)

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

Source: an independent Opus 5.5 code-level audit (2026-09-26), requested by
the user to answer "does the current lexical+graph pipeline (Jev off) do
its best for codebase search?" Full findings are in the audit's report
(not committed as a separate file — see this plan's citations below and
`checklist.md`'s Plan 21 section for the verbatim key numbers). Every item
below traces to a specific, code-verified finding, not speculation.

**Headline finding that reframes everything else**: the project's own
benchmark metric (`EVAL_BACKEND=string` in `scripts/llm-task-eval.py`)
counts a hit as "expected name appears as a substring anywhere in the
top-5 formatted context" (including callers/callees text), not ranking
quality. Re-scored on strict rank, keycloak's reported 35/40 is a strict
pass@5 of 24/40, with mean MRR ≈0.36 — **below the CLAUDE.md ship gate of
MRR ≥ 0.50**, which has never actually been checked this way. Every
hand-tuned rerank constant in this codebase (`name_match_bonus`,
`TEST_FILE_PENALTY`, `DIRECT_HIT_GRAPH_CREDIT_DAMPING`) was fitted against
this lenient metric, on top of a graph with the extraction/resolution bugs
below. **Fix the metric and the graph before re-trusting any rerank
constant.**

## Tier 1 — cheap, clearly-good correctness fixes — DONE 2026-09-26

All 8 items implemented, tested, and measured. New regression tests added:
`resolve_edges_never_crosses_a_language_boundary`, `delete_symbols_for_file_also_removes_fts_rows`,
`implementors_of_excludes_references_edges`, `implementors_of_follows_transitive_inherits_chain`,
`implementors_of_terminates_on_a_cycle` (all `codegraph-store`),
`typescript_class_method_call_attributed_to_method_not_enclosing_class`
(`codegraph-parser`), `lang_filter_restricts_results_to_the_requested_language`
(`codegraph-core`). Full workspace clean: `cargo build/test/clippy --workspace`.

**Directly verified correctness win** (not visible in the aggregate
benchmark, which doesn't exercise this tool): `find_implementations` on
keycloak's `RealmModel` now returns **7 real implementors** (including a
transitive one reached only via `CachedRealmModel extends StorageProviderRealmModel
extends RealmModel`), up from **0** before the fix (all 15 results were
`References`-sourced parameter-mention noise).

**Aggregate strict-scorer numbers before vs. after Tier 1** (`scripts/strict-rank-eval.py`,
top_n=30, Jev off, all three corpora freshly re-indexed):

| Corpus | pass@1 before → after | pass@5 before → after | MRR before → after |
|---|---|---|---|
| keycloak | 7/40 → 7/40 | 23/40 → 22/40 | 0.3362 → 0.3340 |
| erpnext | 2/90 → 1/90 | 15/90 → 14/90 | 0.0930 → 0.0962 |
| lexical | 11/40 → 8/40 | 17/40 → 17/40 | 0.3406 → 0.2969 |

**Honest read: roughly a wash on the aggregate metric, not a clear win —
exactly as this plan's own "Execution notes" anticipated.** Per-task diffs
show mostly ±1 rank shifts in both directions (e.g. lexical's `resetEditor`
moved from rank 1→2, `getActiveEditorState` 1→2/2→3), consistent with the
narrowest-span and same-language fixes adding new, previously-missing edges
that introduce new near-tied competing candidates — not candidates
disappearing or genuine misses appearing. `rerank()`'s constants
(`name_match_bonus`, `TEST_FILE_PENALTY`, `DIRECT_HIT_GRAPH_CREDIT_DAMPING`,
β/γ) are all still tuned against the *old*, buggier graph; Tier 1's real
value is in per-tool/per-query correctness (verified directly above, plus
by the new regression tests) rather than moving this particular aggregate
number — that re-tuning is Tier 3's explicit job, gated on Tier 1+2 first
per this plan's own ordering, not a sign Tier 1 didn't work.

Original item list (for reference — all done):

1. **Add a strict rank-based scorer as the primary gate.** `pass@1`/`pass@5`
   and true MRR (sum of `1/rank`, divided by task count) computed on the
   `symbol` field of the real MCP response, not substring-in-context
   matching. Add erpnext as a third regression corpus (it has never been
   part of any sweep). Re-label/re-check the CLAUDE.md ship gate against
   this real metric instead of the lenient one.
2. **Fix call-edge attribution to the narrowest containing span**, not the
   first symbol whose span contains the call (`codegraph-parser/src/extractor.rs`
   around line 513). Measured: 140 TS/JS class methods in keycloak with
   zero outgoing Calls edges because calls were credited to the enclosing
   class instead. Also affects Dart, Groovy, ObjC, Elixir (module-before-function
   query order) and C# (constructor-before-class).
3. **`resolve_edges` should prefer same-language matches** and never
   resolve a `Calls` edge to a symbol in a different language (measured:
   23% of erpnext's cross-file Calls edges are cross-language nonsense —
   Python→JS, Java→JSON). This doesn't fully fix ambiguous same-language
   resolution (that's Tier 2 item 10) but removes the worst false edges
   cheaply.
4. **`find_implementations`**: drop `References`-sourced edges from its
   result set (or rank `Inherits`/`Implements` strictly first) and add
   transitive closure over `Inherits`. Measured: `RealmModel` currently
   returns 0 real implementations in its top 15 (all parameter-mention
   noise from References).
5. **Delete FTS5 rows for a file's symbols before re-inserting them**, in
   both the live-watcher upsert path and the parallel full-index
   `upsert_batch` path — currently only `delete_file` (whole-file removal)
   cleans up FTS rows; a same-file re-index (edit-and-save) leaves orphaned
   rows that keep matching queries and silently occupy `top_k` slots.
6. **`lang_filter`**: either wire it into the pipeline query (it's parsed
   in `find_relevant_code.rs` and `codegraph-api` but never read by
   `RetrievalPipeline::query`) or remove it from the tool schema so it
   stops silently doing nothing.
7. **Cap `callers`/`callees` in tool output** (currently unbounded beyond
   SQL's own 200-row limit — a single hub result can cost several thousand
   tokens), add a count + truncation flag. Soften `trace_impact`'s tool
   description (it currently claims a caller "can be missed by text search
   but not by a graph edge," which is false when the edge is unresolved —
   173,724 unresolved edges measured in keycloak) and add an ambiguity note
   when the root symbol's name is shared by multiple definitions.
8. **Doc fixes**: CLAUDE.md's "expand_hops ≥ 2" claim is wrong (`neighbors()`
   is 1-hop only, confirmed at the SQL level — only the score-decay factor
   changes, not actual hop count); the JS/TS string-keyed callback claim is
   stale (`synthesis/js_callback.rs` exists and is wired in, just measured
   at 0 edges produced on both benchmark corpora — investigate whether that
   means it's not firing or the corpora don't exercise it, and fix the doc
   to say what's actually true either way).

## Tier 2 — structural fixes — items 9+10 DONE 2026-09-27, item 11 deferred

Full account: `bench-reports/plan21-tier2-qualified-name-and-resolution-2026-09-27.md`.

Three background-agent attempts at this tier stalled on a 600s
no-progress watchdog (root cause not confirmed, likely a long-blocking
command with no intermediate output). The third attempt's in-progress
work was salvaged and finished directly rather than risking a fourth
stall — items 9 and 10 are both implemented, tested, and committed. New
tests: `resolve_edges_ambiguous_name_with_no_import_signal_stays_unresolved`
(updated from the old in-degree-guess assertion), `resolve_edges_import_signal_disambiguates_over_in_degree`
(new, proves the import signal — not in-degree — decides an ambiguous case
via a deliberately-higher-in-degree decoy candidate).

**Sanity-checked directly against the fresh keycloak index**: `DefaultKeycloakSession`
(the audit's own cited example) now has both a real `Class` symbol and a
real, separate constructor `Function` symbol — previously one silently
overwrote the other. Symbol counts after full fresh re-index (required —
item 9 changes every `SymbolId`): keycloak 82,469→**91,175** (+10.6%),
erpnext 41,566→**43,704** (+5.1%), lexical 8,531→**9,423** (+10.5%).

**Strict-scorer results vs. the true pre-Tier-1 baseline** (not vs.
Tier-1-alone, since Tier 1's own numbers were "roughly a wash" and this is
the more meaningful comparison):

| Corpus | pass@5 before → after | MRR before → after |
|---|---|---|
| keycloak | 23/40 → **24/40** | 0.336 → **0.362** |
| erpnext | 15/90 → **16/90** | 0.093 → **0.111** |
| lexical | 17/40 → 16/40 | 0.341 → 0.296 |

keycloak and erpnext improved; lexical dipped slightly (single run, not
yet repeated — flagged for Tier 3, not chased down further here, plausibly
because JS/TS doesn't have the Java/C#-constructor-collision problem item
9 targets so it saw none of the upside while item 10's stricter ambiguity
policy may have cost it some previously-lucky guessed edges). Lenient eval
also run: keycloak 35/40 unchanged, erpnext **23→29/90 improved**, lexical
40/40 unchanged.

**Item 11 (SCIP) — FULLY WORKING, 2026-09-27.** Full account across three
reports (`bench-reports/plan21-tier3-and-scip-2026-09-27.md`,
`.../plan21-scip-fully-working-2026-09-27.md`). Not one bug, four:

1. Two independent copies of the same PATH-joining bug (`src/main.rs`'s CLI
   `extended_scip_path()` and `codegraph-watcher`'s daemon-side
   `extended_path()`) silently prevented indexer binaries from being found.
2. **The real blocker**: `merger::merge_index` resolved each SCIP
   *occurrence* (not document) via a leading-wildcard `files.path LIKE '%' ||
   relative_path` — unindexable, a genuine O(occurrences × files) full-table
   scan. Measured: didn't complete in 25+ minutes on a real 54MB file, in
   complete isolation from the daemon. Fixed: resolve `file_id` once per
   *document* via the indexed exact-match `file_by_path`; `find_symbol_at`/
   `enrich_symbol_from_scip` now take `file_id` and match on the indexed FK.
   **Measured after: 32.2 seconds** for the same file.
3. `auto_run` re-ran the external indexer unconditionally on every restart
   before checking for an existing valid file — caught live re-triggering
   keycloak's ~28min Maven build a second time. Fixed (twice — the first
   attempt conflated "nothing new to ingest" with "no file exists" and still
   re-triggered auto-run every time): gated on the file's mere existence.
4. `detect_project_language` misdetects polyglot repos (erpnext, Python with
   a root `package.json` for JS tooling, got detected as TypeScript) —
   **not fixed**, worked around via `[scip] auto_run = false` per-project.

**End-to-end verified on all three corpora**: keycloak 153,938 symbols
enriched, erpnext 35,371, lexical 10,357 — real `Semantic` edges, live,
confirmed via direct DB query and the `/api/status` `scip_present` field.

**Retrieval-quality impact, real and corpus-dependent, not a universal
win**: keycloak strict pass@5 24→**32/40 (80%)**, MRR 0.362→**0.557** —
**the first result in this project's history to clear the CLAUDE.md MRR≥0.50
ship gate**. Erpnext dipped slightly (28→25/90, reproduced twice, not
noise). Lexical mixed (pass@5 16→17/40, MRR 0.281→0.236). Not chased down
at the per-task level — flagged as a real open question (should
`Semantic`-sourced edges be weighted differently in `rerank()`?) for
whoever picks this up next.

Original item list (for reference — 9, 10 done; 11 deferred):

9. **`qualified_name` format**: change from `"{path}::{name}"` to
   `"{path}::{EnclosingType}::{name}"` with overload disambiguation, per
   the original Plan 02 §8 design (never actually implemented). Measured
   impact of the current bug: ~36% of keycloak Java classes are stored as
   `Function` (constructor overwrites class in id-collision dedup) because
   both share a bare `path::name` id when the constructor's name equals
   the class name. **This changes every `SymbolId`** (content-addressed
   hash of `qualified_name`) — requires a full fresh re-index of every
   project (acceptable; indexes are disposable cache per existing project
   convention) and touching every place that parses or reconstructs a
   qualified name (`resolve_symbol_verbose`'s `rsplit_once("::")`, FTS5's
   bare-name extraction in `graph_db.rs`). Full regression suite (all 3
   corpora, strict scorer from item 1) required before/after.
10. **Import-aware / locality-first cross-file resolution.** Currently
    `resolve_edges` matches by bare name repo-wide and picks the highest
    in-degree definition when ambiguous — creates false hubs (measured:
    one `getId()` overload absorbs 4,270 calls; 68% of ambiguously-named
    keycloak definitions get zero call edges as a result). Use the
    already-extracted import data (extracted but never consulted, per
    Plan 02 §6 step 2) to narrow candidates before falling back to
    in-degree; when still ambiguous with no signal, leave the edge
    unresolved rather than routing it to a global winner. Also: stop
    blindly re-resolving surviving cross-file edges to a new "winner" on
    every save of an unrelated file (measured: editing file A can silently
    re-route file B's already-correct call edges).
11. **Turn on SCIP for both benchmark corpora and measure it for real.**
    `codegraph-scip` is built and shipped but has never actually been run
    against keycloak (`scip-java`) or erpnext (`scip-python`) — both
    indexes currently contain zero `Semantic`-sourced edges. This directly
    addresses item 10's false-hub problem with compiler-backed ground
    truth, still model-free. Measure before/after on the strict scorer.

## Tier 3 — items 12-15 DONE 2026-09-27, item 16 partial

Full account: `bench-reports/plan21-tier3-and-scip-2026-09-27.md`.

Items 12-15 (body-text tokenization, acronym/digit splitting, synonym
down-weighting, `expand_graph` round-robin) all implemented, tested
(2 new pinned-stem regression tests for item 13), and measured. **Real
strict-scorer result**: erpnext pass@5 16/90→**28/90** (MRR 0.111→0.172,
nearly doubled — item 12's body-tokenization fix was indeed the largest
lever, as the audit predicted), lexical pass@5 unchanged at 16/40 (MRR
0.296→0.281, a small dip not chased down). Full workspace clean throughout
(build/test/clippy).

Item 16 (joint β/γ/damping sweep) was **scoped down after an automated
sweep script caused real process-management chaos** (concurrent daemon
instances on one port, a background-task lifetime limit silently killing
an in-progress run, and one cleanup `git checkout` that briefly discarded
items 12-15's own uncommitted work before it was reconstructed from
context). Replaced with one careful, hand-executed, single-variable
comparison (β=0.667/γ=0.333 shipped vs. β=0.5/γ=0.5, damping fixed at
0.75, erpnext+lexical only): β=0.5 shows a small, consistent MRR
improvement on both corpora (0.1720→0.1795 erpnext, 0.2810→0.2961 lexical)
with unchanged pass@5 — **not shipped**, a single 2-point comparison
doesn't meet this project's own validation bar (multi-run, neighboring-
value plateau check). Flagged as a promising lead for a real follow-up
sweep. The `graph_score`/`lexical_score` correlation-logging diagnostic
was not attempted (time budget).

12. **Tokenize body/snippet text the same way query text is tokenized**
    (shared `split_words` + Snowball stemming) instead of FTS5's plain
    `unicode61` on `chunk_text`/`file_path`. Measured: `chunk_text:"creat"`
    matches 11 rows, `chunk_text:"create"` matches 5,778 — body text
    essentially only matches on exact surface form today, so most of the
    graph-context text `ChunkBuilder` writes into each chunk barely helps
    matching. Do **not** use FTS5's built-in `porter` tokenizer — it's
    original Porter stemming, not Snowball, and would reintroduce the
    exact query/index stem mismatch this project already fixed once
    (2026-08-17's `readEditorState` investigation). This is likely the
    single largest recall lever on this list and needs the full bar before
    shipping.
13. **Acronym/camelCase-aware tokenization for names with runs of capitals
    or digits** (`isSSOAuthentication` → `ssoauthent`, `sha256UrlEncodedHash`
    → `sha256url…` — both are real benchmark targets). Needs an FTS
    backfill, not just a live-query change.
14. **Down-weight synonym-expanded terms** relative to the user's original
    query terms (currently plain `OR`, equal weight) — e.g. ~0.3-0.5x via
    a second FTS pass or query-weighting mechanism.
15. **Fix `expand_graph`'s per-seed candidate cap** — currently effectively
    random within a seed (`ORDER BY 1` = symbol-id hash order) and filled
    seed-by-seed so one high-fan-in seed can starve later seeds entirely.
    Round-robin across seeds and/or order neighbors by something meaningful
    (edge kind, name-match to query) instead of hash order. Test genuine
    2-hop expansion (`neighbors()` currently ignores `hops` beyond 0/1) as
    a separate, explicitly-labeled experiment, not bundled with the cap fix.
16. **Joint β/γ/`DIRECT_HIT_GRAPH_CREDIT_DAMPING` re-sweep** — the audit
    found that at damping=1.0 the direct-hit formula becomes mathematically
    identical to the expanded-neighbor formula, meaning the "damping sweep"
    already shipped was actually a partial, disguised re-sweep of β. Do a
    real joint sweep post-Tier-2, plus log the correlation between
    `graph_score` and `lexical_score` on direct hits to check the audit's
    hypothesis that personalized-PageRank's lexical-weighted restart makes
    "graph credit" partly an echo of lexical score rather than independent
    signal.

## Explicitly not re-attempting

Per the audit's own note, avoid re-proposing anything `CLAUDE.md` already
documents as tried-and-failed: the own-token-fraction name score, a `bm25`
column rebalance or `chunk_text` length discount, graph-score caller-credit,
the relative-score floor, and ungated recall widening.

## Execution notes

- Tier 1 items are mostly independent of each other; can be done together
  in one pass with one verification cycle.
- Tier 2 item 9 (qualified_name format) is the highest blast-radius change
  in this plan — full fresh re-index of all registered projects is expected
  and fine (disposable cache), but every downstream consumer of qualified
  names must be re-checked, not just `SymbolId` generation.
- Tier 3 must not start until Tier 1+2 are committed and the strict scorer
  (item 1) is in place — old rerank constants are known-stale once the
  underlying graph changes.
- Update `checklist.md` after each tier (or each item, if a single item
  takes substantial independent effort) — don't batch to the end.
