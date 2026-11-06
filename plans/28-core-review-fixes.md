# Plan 28 — Core review fixes (2026-10-30)

## Context
A read-only review of indexing, retrieval and the MCP tools (three parallel passes, each finding re-read against the code) found 17 issues: one security hole, one bug that silently dropped real source files from the index, and performance, ranking and protocol problems. All 17 are fixed here. Ranking-affecting items (8, 10, 11) are **not** benchmark-validated yet.

## The 17 fixes
Security / correctness
1. Origin and Host guard on the daemon (`src/origin_guard.rs`); permissive CORS removed from the MCP and REST routers. Any website could read indexed source through `localhost`.
2. Ignore patterns match whole path segments (`watcher.rs`). `dist/**` used to drop `distribution.py`, `vendor/**` dropped `landed_cost_vendor_invoice`, `.git/**` dropped `.github`.
3. Migrations run in one transaction each (`migrations.rs`); migration 015 drops two duplicate indexes.
4. Hot-path DB cost: `enrich` loads symbols in one `IN (...)` query, callers/callees are fetched only for the returned results, PageRank reads callee ids only, MCP/REST handlers run under `block_in_place` (`src/offload.rs`).
5. Full index no longer overwrites a newer live save: a superseded entry is re-indexed from disk once (`worker.rs`), so no stale symbols or ghost file rows.
6. Chunk build uses one batched names-only neighbor query per file and propagates errors instead of committing empty context (`call_neighbor_names_for_file`).
7. The full-index walk prunes ignored directories with `filter_entry`.

Retrieval
8. BM25 `avg_len` and `doc_len` use the same `tokenize_name` tokens.
9. `candidate_top_k` and `final_top_n` are wired into the MCP and REST callers; a non-default `expand_hops` logs a warning (graph expansion is always 1 hop). `top_n` is clamped to 1-30.
10. Judges: Jev has an overall deadline (retries included); an unscored candidate gets the mean returned score instead of an implicit 0.
11. The language filter restricts lexical hits and graph neighbors before the candidate budget is spent (4x over-fetch).

MCP tools
12. `lines` and `focus_line` are 1-based everywhere (`tools::line_range`), matching `source`.
13. `inspect` path mode caps the `symbols` list (200, or 50 with source) and splits the file once.
14. Symbol-name resolution uses `in_degrees_batch` instead of one `COUNT(*)` per candidate; ambiguous-path errors list at most 10 paths.
15. `implements`: typed `SymbolNotFound` (DB errors are no longer "not found"), `total_found`/`truncated`, resolution note, description corrected.
16. `trace_impact` follows Inherits/Implements/References for non-callable kinds, clamps depth to 1-6, and stops at a 5,000-node budget (`affected_count_is_lower_bound`).
17. Directory lookup escapes LIKE wildcards and is case-sensitive; `delete_files_under` uses a binary range.

Also: JSON-RPC parse/invalid-request errors and `"id": null` handled, protocol version negotiated, schema bounds, `watch_status.last_error`, `gr index` uses the daemon's config loader, debounce window starts at the first event, Jev/ranking sorts use `total_cmp`.

## Success criteria
- `cargo test --workspace` (539 tests) and `cargo clippy --workspace -- -D warnings` clean. Done.
- A `run-sot.sh` run before tagging shows no `graphrelay` coverage drop vs the last run (items 8, 10, 11 change ranking). **Open** — the Gitea structural run with Sonnet 5.5 is the first data point.

## Not done
- Read-only connection pool (item 4 uses `block_in_place`, the mutex remains).
- Bounded `incremental_cache`/`SourceLoader` caches; overflow re-index still runs inline in the live loop.

## Follow-up fix (2026-10-30): per-file neighbour-name query scanned every Calls edge
`GraphDb::call_neighbor_names_for_file` (the batched replacement for per-symbol caller/callee lookups) let SQLite drive the join from `idx_edges_kind`, so each of the 4,825 ERPNext files visited all 91,173 Calls edges (about 900M row visits). A fresh ERPNext index went from 31 s (recorded) to more than 5 minutes, almost all of it in the chunk/FTS phase. The query now starts from `symbols(file_id)` (CROSS JOIN order and `+e.kind`); output is byte-identical on the real index. Measured: chunk phase more than 4.5 min -> 11.5 s, total 29.7 s with an 8-core parse pool (Colby 14 s, graphify 72 s on the same checkout). Test `neighbor_names_query_starts_from_the_files_symbols` pins the plan.
