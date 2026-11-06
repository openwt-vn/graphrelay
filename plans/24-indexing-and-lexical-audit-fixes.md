# Plan 24 — Indexing, SCIP and Lexical-Search Audit Fixes

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

Status: **IMPLEMENTED 2026-09-29** (user goal: implement all phases). Phases 0-4 done; final-commit validation and upgrade-path test results are in `bench-reports/plan24-final-validation-2026-09-29.md`. Deviations from the draft: SCIP demoted (not kept/removed) by the 1.8 gate; 3.2 measured and not shipped; 3.4 shipped as a df cap of 0.2 (not 0.5) with a cached, batched vocab lookup; 2.1 became a migration-driven reset.
Date: 2026-09-29

## Provenance and trust level

Two independent Opus 5.5 read-only reviews of the indexing pipeline and
`lexical_search()`. The second pass tried to falsify the first and built an offline
harness (real `RetrievalPipeline` on `.backup` copies of the three live DBs, Jev off).
The harness reproduced the recorded baselines (keycloak 33/40 MRR 0.5887, erpnext
30/90 0.1910, lexical 17/40 0.2786).

**All numbers below are reviewer-reported and have NOT been re-measured by the
author of this plan.** Per this project's history (two self-reported gains that did
not reproduce), every phase must re-measure before and after. Phase 0 exists to make
that possible. Harness location (scratch, not in repo):
`<tmp>` — if it is worth keeping, move it into `scripts/` in Phase 0.

## Decision on fuzzy search: DEFER

Evidence (reviewer-measured): across 170 benchmark tasks, fuzzy matching could fix
about 1 miss (erp-44/erp-36, morphology, not a typo). Real typos: 1 in 170, and that
task already hits. 48 real agent `search_code` intents contained 492 terms, 0 with no
exact FTS hit. Near-neighbour precision is poor (`feat` → bear/beat/best/boat).
The dominant addressable classes are the tokenization gap (Phase 3.1) and SCIP graph
corruption (Phase 1), not spelling.

**Trigger to revisit:** after Phase 0.2 logs intents, collect >= 200 real agent
queries. Revisit if > 2% have an original no-hit term whose edit-distance <= 2
neighbour appears in the target, or >= 3 benchmark misses remain in the near-spelling
class after Phases 1-3.
**If ever built:** gate on no-hit original terms only; FTS5 native prefix query on
the truncated stem (prefix length >= 5) at weight <= `SYNONYM_TERM_WEIGHT`;
deterministic tiebreak; no new FTS table (avoids re-creating the `chunk_fts`
no-cascade invariant surface and a new backfill trigger).

## Findings (verified by second pass unless noted)

### Correctness — SCIP layer (highest severity, missed by the first review)
- M1 `find_symbol_by_scip` resolves SCIP `local N` ids globally (`merger.rs:137`,
  `graph_db.rs:1589`). Cross-file bogus `Calls` edges: keycloak 140,947, erpnext
  28,778, lexical 11,270. Corrupts chunk text, PageRank, `expand_graph`, `trace_impact`.
  **Every current baseline includes these edges.**
- M2 `enrich_symbol_from_scip` matches `(file_id, start_line)` only
  (`graph_db.rs:1514-1532`): a parameter on the method's line overwrites the method's
  SCIP id. ~43% of keycloak symbols end up with `scip_symbol='local N'`.
- M3 overload display names `foo(+2` (`merger.rs:198-201`); 51,788 keycloak semantic
  edges, ~51k can never resolve.
- M4 every reference occurrence becomes `Calls` (`merger.rs:133-147`), incl. field
  reads; 109,160 semantic edges with empty `to_name` in keycloak.
- #8 duplicate semantic edges (273 groups keycloak, 7 lexical). Root cause corrected:
  `resolve_edges` (`graph_db.rs:1236`) selects `to_id IS NULL` without a source filter
  and name-resolves semantic edges SCIP left unresolved — also a misattribution.
- #18/19 SCIP merge runs concurrently with the chunk backlog (`server.rs:170-173`);
  `merge_index` deletes all semantic edges up front in autocommit and reinserts at the
  end; an FK violation from a concurrent save aborts the insert after the delete.

### Correctness — indexing path
- #1 live save uses full `delete_symbols_for_file` (`worker.rs:277`); also
  `upsert_batch` (`graph_db.rs:245-281`) and `coordinator.index_file`
  (`coordinator.rs:28`). Commit c894701 claims a keep-unchanged-symbols fix but never
  touched `worker.rs`. Each save destroys outgoing SCIP edges (permanently — `auto_run`
  only fires when no `.scip` exists), SCIP kind/`scip_symbol`, and synthesized edges,
  and re-routes incoming edges by name.
- #3 shared checksum-0 sentinel can't identify the writer (`worker.rs:373,407`):
  startup backlog vs live save leaves orphan `chunk_fts` rows and drops correct
  chunks. Also `build_chunks_for_pending` re-reads disk (`:292`) but uses DB byte
  spans that may belong to another version.
- #4 live write is 4 separate lock acquisitions (`worker.rs:275-281`); racing the
  startup full index yields old+new symbol union and duplicate edges (startup only).
- #5 `backfill_fts_index` (`worker.rs:918-952`) writes reduced text (no
  callers/callees/class context) after every FTS-format migration
  (`FTS_BACKFILL_TRIGGERS`), never rebuilt for unchanged files, and races the full
  index. `checklist.md:1282` already records keycloak MRR 0.306 (backfill) vs 0.589
  (`--fresh`).
- #2 no pruning of `files` rows absent from the walk; directory Remove/Rename events
  dropped (`watcher.rs:128,146-154`).
- #6 `worker.rs:146,153-163` clears `last_error`/sets `Fresh` after a failed FTS write.
- #7 `resolve_edges` errors discarded (`worker.rs:141,198`).
- M8 overflow flag cleared then the `full_index` call returns early while the startup
  guard is held (`watcher.rs:85-86`, `worker.rs:890-893`); dropped events never
  recovered.
- #16 `resolve_edges` takes 600-704 ms per call on keycloak (654,212 permanently
  unresolved edges rescanned) while holding the single `Mutex<Connection>`; blocks all
  queries after every live-save batch.

### Lexical quality / latency
- #9 nondeterministic ordering: path-pin merge iterates a `HashMap`
  (`pipeline.rs:158-162`) feeding `expand_graph`'s order-dependent 100 cap; score ties
  in `lexical_search` (`graph_db.rs:455-465`). 6 of 170 tasks returned >1 distinct
  result list in 20 repeats. Moves MRR; does not flip pass@5 today.
- #10 whole compound identifier never emitted; sub-tokens < 3 chars dropped. `getId`,
  `toUI` produce zero terms (empty result, `pipeline.rs:165`); `IOError` → `error`.
  13.7-18.5% of symbol names lose a subtoken. kc-bug-01 and kc-impact-02 name the
  target literally yet miss. Compound is already indexed by unicode61 on the raw
  qualified name.
- #11/#12 absolute path in `qualified_name`/`file_path`/chunk header (4-5 copies per
  row): root-path words match every row (zero IDF), and add FTS latency. keycloak
  `lexical_search` for a short query 11 ms → 58 ms when one root-path word is added;
  end-to-end 55-63 → 103-107 ms (over the 100 ms gate). On real keycloak queries
  `lexical_search` is ~82% of warm end-to-end time (contradicts checklist item 4).
- #13 appended `tokenize_name(chunk_text)` doubles every word's tf and doc length.
- #14 body budget `1500 − prefix − 200`; header averages 364 chars on keycloak;
  15% of keycloak symbols exceed 1,100 bytes.
- #15 `split_words` floor is bytes, not chars (symmetric; non-ASCII only).

### Measurement
- M7 `strict-rank-eval.py` does not strip `#N` overload suffix or leading `$`.
  Reviewer-reported raw vs normalized: lexical 17/40 (MRR 0.279) → 34/40 (0.558);
  keycloak 33/40 (0.589) → 35/40 (0.676); erpnext 30/90 unchanged. Normalized scoring
  credits any overload of the expected name. erp-29, erp-86 targets don't exist.
- M6 SymbolIds depend on checkout root (absolute qualified names); CLAUDE.md's
  "stable across rebuilds" holds only for the same root; `graph_db.rs:322` doc
  comment wrongly says relative.

## Phases

Every phase: `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`
clean; re-measure with the calibration harness (must reproduce baselines before
changing anything); >= 3 runs on all three corpora (keycloak, erpnext, lexical);
update `checklist.md` per item as it lands. No phase ships on a single run.

### Phase 0 — Measurement hygiene (S, no invariant risk)
- [x] 0.1 `strict-rank-eval.py`: strip `#N`/leading `$`; report raw + normalized;
      re-baseline all three corpora.
- [x] 0.2 Persist `search_code` intents to a log file (needed for the fuzzy trigger).
- [x] 0.3 Deterministic tie-breaks: `.then(id)` in `lexical_search` sort; sort pinned
      merge by (score desc, id) before `expand_graph`; `ORDER BY rank, rowid` in
      `fts_match`/`symbols_for_path_pin`. Success: 0 distinct variants over 20 repeats
      on all 170 tasks; baselines unchanged except tie cases.
- [x] 0.4 Move the offline harness + calibration gate into `scripts/`/`benchmarks/`.

### Phase 1 — SCIP correctness (M-L; changes the graph under every baseline — do first)
Requires fresh index + SCIP re-ingest, then multi-run re-baseline. Expect all
baselines to move; record before/after honestly.
- [x] 1.1 Skip `local N` symbols in enrichment and edge creation.
- [x] 1.2 Match enrichment by name/column range, not `start_line` alone.
- [x] 1.3 Handle overload suffix (`(+N)`) correctly.
- [x] 1.4 `Calls` only for call-like occurrences (or tag others `References`).
- [x] 1.5 `resolve_edges`: add `AND source != 'semantic'`.
- [x] 1.6 Unique index / dedupe on edge identity `(from_id,to_id,to_name,kind,source)`.
- [x] 1.7 SCIP merge in one transaction (`GraphDb::apply_scip_merge`); at startup SCIP
      ingest runs before the chunk backlog when a `.scip` file already exists (else stays
      concurrent — the external indexer can take hours). Also: `[scip] scip_path` is now
      honored (was hardcoded), migration 010 heals existing DBs (deletes semantic edges,
      clears `local N` ids, resets ingest marker, resets file checksums only when SCIP
      had been ingested) and adds a semantic-scoped unique edge index.
- [x] 1.8 **SCIP A/B gate (decides SCIP's future; Phases 2-3 wait on it).** After
      1.1-1.7 and a fresh index + SCIP re-ingest, run SCIP-on vs SCIP-off on keycloak,
      erpnext and lexical, >= 3 runs per arm, harness calibration first, pass@5 and
      strict MRR (normalized and raw). Background: `bench-reports/plan21-scip-fully-working-2026-09-27.md`
      measured keycloak 24/40 -> 32/40 (MRR 0.362 -> 0.557) but erpnext 28/90 -> 25/90
      and lexical MRR 0.281 -> 0.236, all on a graph the second review found corrupted
      (M1/M2), so the cause of the keycloak gain is unknown.
      **Method (pinned 2026-09-29, before any arm was run):** arms are built with
      `benchmarks/src/bin/offline_index.rs` into scratch DBs (never live DBs) and
      scored with `offline_eval` + `scripts/offline-eval-score.py` (Jev off). Three
      arms per corpus: A = SCIP off (HEAD), B = SCIP on, *unfixed* code (HEAD~), C =
      SCIP on, after 1.1-1.7. Querying is deterministic (0.3), so repetition means
      **independent index builds**: >= 2 builds per arm; the build-to-build spread of
      the same arm is the noise floor N (max abs diff of normalized MRR). If two builds
      are byte-identical, say so and stop repeating. B is run first to check the old
      keycloak 24->32 gain even reproduces with this build method.
      **Deciding metric:** normalized MRR (primary); raw MRR, raw/normalized pass@5
      reported alongside. Compare C against A (per corpus, mean of builds).
      Decision rule:
      - **Keep** (`auto_run` stays default) if keycloak dMRR_norm >= +0.05 (and above
        2N) AND erpnext and lexical dMRR_norm >= -0.02.
      - **Remove** if all three corpora have |dMRR_norm| <= max(0.02, 2N) AND
        |dpass@5| <= 1 (keycloak, lexical) / <= 2 (erpnext): SCIP is neutral. Delete
        `codegraph-scip`, the merge path and SCIP-specific resolve/enrich handling,
        and simplify Phase 2 items that preserve SCIP edges (2.2's SCIP
        `kind`/`scip_symbol` preservation).
      - **Demote** to opt-in (`auto_run = false`, documented as corpus-dependent) in
        every other case (keycloak gain below threshold but non-neutral elsewhere, or
        any corpus regressing beyond -0.02).
      **OUTCOME (2026-09-29): DEMOTE.** C-A normalized MRR: lexical +0.0439, erpnext
      +0.0037, keycloak +0.0088 (noise floor 0: independent builds were identical); the
      unfixed layer (B) was harmful everywhere (-0.111/-0.040/-0.075) and the old
      "keycloak 24->32" gain did not reproduce (a clean SCIP-off build already beats it).
      Keep needed keycloak >= +0.05 (no); Remove needed all three neutral (lexical is
      not). `[scip] auto_run` now defaults to false; an existing `.scip` is still
      ingested. Full account: `bench-reports/plan24-scip-ab-gate-2026-09-29.md`.
      **ADDENDUM (2026-09-29, final validation):** re-measured with the shipped code
      (SCIP on vs off, same builds otherwise): keycloak +0.054, lexical +0.047, erpnext
      -0.001 normalized MRR, which *would* meet the Keep thresholds. The rule was pinned
      for the Phase 1 arms and is not changed after seeing numbers, so the recorded
      outcome stays DEMOTE; flipping `ScipConfig::default().auto_run` is a user decision
      (an existing `.scip` is ingested either way). See
      `bench-reports/plan24-final-validation-2026-09-29.md`.
      Record the outcome in `checklist.md` and this plan.

### Phase 2 — Indexing-path correctness (M each)
Implemented 2026-09-29 (independent of the 1.8 gate except that 2.2's SCIP-preservation clauses become dead code if the gate says Remove).
Design notes: 2.1 became a migration-driven reset (FTS-format migrations set `files.checksum = 0`; the normal full index rebuilds full chunk text) and `backfill_fts_index` was deleted. 2.3 uses `files.pending_token` (migration 011) and one atomic `GraphDb::commit_chunk_batch` (token check + FTS write + checksum commit). 2.6 shipped the scoped `resolve_edges_for_files` (equality-tested against the full pass); a separate read-only connection/pool was NOT done (the scoped pass removes the ~0.7 s lock hold that motivated it).
- [x] 2.1 Backfill via full `ChunkBuilder` path (`flush_chunk_batch`) after
      `resolve_edges`, under the full-index guard, instead of reduced text. Must land
      before any content-format migration. Invariant: SQLite upsert before chunk build.
- [x] 2.2 Keep-ids single-transaction writes in the live path, `upsert_batch` and
      `coordinator.index_file`: upsert file; delete file's `source='syntax'` outgoing
      edges; delete symbols not in keep-set + their `chunk_fts` rows; upsert symbols
      without clobbering SCIP `kind`/`scip_symbol`; insert edges. (Edges have no
      unique key, so the explicit edge delete is mandatory.) Invariants:
      SQLite-before-chunk-build; `chunk_fts` cleanup on symbol removal.
- [x] 2.3 Writer-specific pending token (or single flush queue) instead of shared
      checksum-0; build chunks from the same bytes that were extracted. Invariant:
      checksum commit gated on FTS write.
- [x] 2.4 Prune `files` absent from the walk via `delete_file` (keeps `chunk_fts`
      invariant); handle directory Remove/Rename by prefix (`files_in_prefix`);
      index new directories.
- [x] 2.5 Status/errors: don't clear `last_error`/set `Fresh` after failure; log
      `resolve_edges` errors; queue overflow re-index instead of dropping it.
- [x] 2.6 Incremental `resolve_edges` scoped to the batch's files/names; consider a
      read-only connection or pool for queries.
- [x] 2.7 Startup GC: delete `chunk_fts` rows with no matching symbol.

### Phase 3 — Lexical quality and latency
- [x] 3.1 Query-side: also emit whole identifier, lowercased and unstemmed, as an
      original term (S). Expected to fix kc-bug-01, kc-impact-02 — untested.
- [x] 3.2 **MEASURED, NOT SHIPPED (2026-09-29).** keycloak dMRR_norm -0.0048 (V4r 0.7119 vs V4 0.7167), lexical +0.0084, erpnext -0.0064: keycloak missed the +0.01 bar. The plumbing (`set_project_root`, env switch) was removed; Phase 4 already makes `qualified_name` root-relative and the 3.4 df cap skips root-path words at query time. **Ship rule (stated 2026-09-29 before the keycloak arm was read):** ship only
      if keycloak dMRR_norm (root-stripped vs not, both with Phase 4) >= +0.01 AND lexical
      and erpnext dMRR_norm >= -0.01; otherwise record "measured, not shipped" and remove
      the plumbing. Already known then: lexical +0.0084, erpnext -0.0064.
      Remove the root path from FTS content only (keep `symbols.qualified_name`
      unchanged so IDs don't change). Content-format migration; depends on 2.1.
      Risk: arm-C history shows path-token changes can badly hurt keycloak.
- [x] 3.3 Stop duplicating raw words in appended tokens (#13); bundle with 3.2.
- [x] 3.4 FTS-stage latency: drop/cap terms with df > ~50% before `MATCH`. Validate
      p95 <= 100 ms and MRR.

### Phase 4 — needs explicit user sign-off
- [x] 4.1 Relative-path qualified names (M6). Changes every SymbolId, i.e. touches the
      "Symbol IDs are xxhash64(qualified_name)" invariant; needs a fresh index
      everywhere.

## Open questions for the reviewer (user) — resolved by the user's "implement all phases" goal
1. Phase order: followed (SCIP first, gate before Phase 2 was finalized).
2. Phase 4 (relative paths): done.
3. Fuzzy search: still deferred with the stated trigger (`CODEGRAPH_INTENT_LOG` collects the queries).
4. Scope: all phases.
