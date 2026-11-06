# CLAUDE.md — graphrelay

# Always try to use GraphRelay MCP tools first before other tools.

> **Rename (2026-10-13):** this project was renamed from `codegraph` to **GraphRelay** (CLI binary `gr`, crates `graphrelay-*`, data dir `~/.graphrelay`, config `graphrelay.toml`, env `GRAPHRELAY_*`, MCP server `graphrelay`). Hard rename, no fallback: existing `~/.codegraph` indexes are not read. `plans/`, `checklist.md` and `benchmarks/dataset/results/` are historical records and keep the old name. The competitor arm `codegraph-colby` (colbymchenry/codegraph) is a different product and is unchanged.

## Working rules for agents

Follow these without exception:

1. **Use GraphRelay first.** Before reading files, grepping or exploring, query the GraphRelay MCP tools: `search_code` (`intent` for natural-language search, `implements` for implementations of a named interface/trait/base class), `inspect` (`symbol` for one symbol's definition/callers/callees, `path` for a file's or module's contents; `include_source: true` for the full file text in place of Read) and `trace_impact` before changing anything. Fall back to direct reads only when GraphRelay cannot answer (config files, build scripts, non-indexed content such as JSON fixtures).
2. **Plans are authoritative.** Read the relevant `plans/*.md` before implementing. Each states success criteria, invariants and step order.
3. **`checklist.md` is the source of truth for status.** Check the relevant section before starting; after every significant change (even one item) flip `[ ]` to `[x]` (done) or `[~]` (in progress) immediately and keep the Progress Summary table at the bottom current.
4. **When unsure how something should behave, read the plan first.** Do not invent behavior that contradicts it.
5. **Do not break `## Key invariants`.** They are correctness constraints found through testing.

## What this project is

A single Rust binary (`gr`) that provides graph-aware code search via MCP, REST and a web UI on one port (default `:3030`). The thesis is **fused lexical+graph retrieval**: SQLite FTS5/BM25 + graph expansion + personalized PageRank + rerank in one pipeline.

**There is no vector search.** Plan 20 (2026-09-25) removed it entirely (no ONNX/BGE model, no cloud embeddings, no LanceDB, no HyDE). What older docs and plans call `embedding_degraded` / "no-semantic" mode is now the only mode. See `plans/20-remove-vector-search.md`. `EdgeSource::Semantic` means SCIP compiler-backed edges, unrelated to embeddings.

## Benchmarking — the source of truth

**One benchmark counts: the agent benchmark in `benchmarks/dataset/`.** An LLM agent (Claude Code CLI) answers 30 hard ERPNext questions with Grep/Read/Glob plus, in the `graphrelay` arm, our MCP server; the `baseline` arm (no MCP) is the control; a blind judge grades key-fact coverage. Agents: `claude-sonnet-5` and `openrouter-glm-5-3-flash` (LiteLLM proxy in `harness/proxy`, needs `OPENROUTER_API_KEY`). Default arms are `graphrelay` + `baseline`; optional (`--arms`): `graphify`, `codegraph-colby` (competitors), `graphrelay-jev` / `graphrelay-laya` (our tool with a judge on).

```sh
benchmarks/dataset/run-sot.sh --dry-run                      # prerequisites, spends nothing
benchmarks/dataset/run-sot.sh --repeats 3                    # build, index into an isolated $HOME, run, grade, report
benchmarks/dataset/run-sot.sh --no-build --questions Q01,Q02  # comma-separated ids; reuse target/release/gr
```

It indexes ERPNext into an isolated `$HOME`, so your real `~/.graphrelay` is untouched. Questions are frozen in `tasks/sot_questions.txt`. Results go to `results/sot/<label>/<model>/rep<k>/` (`SUMMARY.md`, `grades.json`, `transcripts/`); read `SUMMARY.md` only after the script has exited.

**Evidence so far: nothing is reliably better than the no-tool baseline.** Sonnet 5.5, ERPNext, 30 questions × 3 repeats: baseline 70.6%, graphrelay 71.2% (+0.6, CI [-4.0, +5.0]), with ~30% fewer tool calls and ~1.3× the cost per question; graphify and colby are lower. Every later change (twin merge, string-dispatch and self-call edges, the v0.1.0 build at 69.5%) is a tie. Earlier single-repeat runs on both models (2026-10-06/07) are also ties; the only interval excluding 0 is colby on GLM. Single repeats swing 5-14 points per arm and the judge alone varies up to 25 points per answer, so use `--repeats 3`, compare per question with a paired bootstrap, and treat partial grade files as not a result. Details: `docs/benchmarks.md`, `plans/31-…` to `plans/36-…`.

Q22 (period closing voucher) is the one confirmed loss: the agent stops after one `search_code` result that looks complete (63.3% vs 75.8% over 10 repeats). Wording, `also_in_file`, payload trim, nested-result dedupe and `other_matches` were all tried and did not move it (Plans 31, 33, 34).

**Retired on 2026-09-30** (recoverable at commit `5d413ea`): the retrieval-only suites, `bench-reports/`, the old `benchmarks` binary, the older agent-benchmark results and most competitor setup scripts. Numbers quoted in older plans and the checklist (MRR, 40/40, 33/40, …) came from those suites: history, not gates. They could not carry decisions (65-70% of queries contained the target's name, 22-25% of gold labels were ambiguous, effects sat inside the noise; see `plans/26-reliable-benchmark.md`). Any `bench-reports/…` path in a plan no longer exists. The only retained measurement tool is `benchmarks/src/bin/offline_index.rs` (indexing time, memory, upgrade behaviour).

## Build, test, release

```sh
cargo build --workspace          # dev build
cargo build --release            # ~78 MB stripped (`strip = true` is set in [profile.release])
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

The binary reads `graphrelay.toml` from CWD upward, then falls back to defaults. There is no model to download.

**Releasing:** bump `version` in the workspace `Cargo.toml` (and `Cargo.lock`) first, merge, then push a `vX.Y.Z` tag. `.github/workflows/release.yml` builds macOS arm64, Linux x86_64 and Linux arm64 and publishes the release (no macOS Intel build). Check `gr --version` on the downloaded binary. `scripts/install.sh --version=vX.Y.Z` installs it.

## Crate map — one responsibility per crate

| Crate | Owns |
|---|---|
| `graphrelay-core` | Domain types (`Symbol`, `Edge`, `Chunk`, `IndexedChunk`, `RetrievalResult`), config, the retrieval pipeline, `ChunkBuilder`, the `QueryPipeline` dyn trait. Zero I/O; `async-trait` only for that trait. |
| `graphrelay-parser` | Tree-sitter extraction (`extract()`, `extract_incremental()` for the single-file path, `extract_many()` parallel via rayon for the full index), the language registry, and `synthesis/` (dynamic-dispatch edges, see Indexing). |
| `graphrelay-store` | `GraphDb` (SQLite incl. the FTS5 index and migrations), `StoreCoordinator` (graph write + FTS write), `upsert_batch()` (one transaction per batch). |
| `graphrelay-watcher` | `FileWatcher` (notify-rs, debounce) and `IndexWorker` (async loop; owns `Arc<RwLock<IndexStatus>>`). The live loop batches graph writes for a debounced set of paths before one `flush_chunk_batch`. |
| `graphrelay-mcp` | MCP JSON-RPC over HTTP. Exactly 4 tools (see invariants). |
| `graphrelay-api` | REST routes for the web UI (`/api/*`); `ApiState` holds `Arc<dyn QueryPipeline>`. |
| `graphrelay-ui` | Embedded web UI assets via `rust-embed` (Cytoscape graph view, search, inspector, status bar). |
| `graphrelay-scip` | Optional SCIP enrichment: ingests a pre-computed `.scip` index and merges `Semantic`-sourced cross-file edges (`GraphDb::apply_scip_merge`, one transaction; only call descriptors become `Calls`). Feature-gated (`scip`); `[scip] auto_run` defaults to `false`, an existing `.scip` file is still ingested, `[scip] scip_path` is honored. |
| `graphrelay-decision-engine` | Optional relevance judges behind one `RelevanceJudge` trait: `jev` (TypeSafe AI, one call for the whole pool) and `laya` (self-hosted, one request per candidate). Both off by default, additive bonus on top of the always-on ranking, never a replacement; Jev wins if both are on. Measured gains were small and corpus-dependent and they never beat plain graphrelay in the agent benchmark (`docs/relevance-judges.md`). |

Dependency rule: nothing depends on `graphrelay-mcp` or `graphrelay-api` except `main.rs`; `graphrelay-core` has zero internal deps. Root-level modules in `src/`: `offload.rs` (runs `/mcp` and `/api` handlers under `block_in_place` behind a 16-permit semaphore) and `origin_guard.rs` (Origin/Host guard; there is no permissive CORS).

## Key invariants — do not break these

- **SQLite upsert BEFORE chunk build.** `index_path_graph_only()` / `GraphDb::upsert_batch()` write symbols first and `resolve_edges()` runs before chunk building, so `build_chunks_for_pending()` sees resolved callers/callees. Holds for the live loop and the parallel full index. Reversing it silently yields chunks with empty neighbor context.
- **The checksum commit is gated on the FTS5 write.** A file whose `upsert_fts_batch` fails must stay at the checksum-0 pending sentinel so it is retried. This is enforced in one transaction, `GraphDb::commit_chunk_batch` (pending-token check + FTS write + checksum commit); each graph write stamps a fresh `files.pending_token` and only chunks built from that token are accepted. Do not split those three steps.
- **Symbol IDs are `xxhash64(qualified_name)`**, content-addressed, never a row id or random. `qualified_name` is `{path relative to the project root}::{EnclosingType}::{name}` (`Extractor::with_root`), so ids are stable across checkouts; same-named siblings get `#N`. Changing how names are built changes ids, so it needs a migration that resets the affected files to pending (012 relative names, 016 Go receivers, 017 Python decorated twins). IDs are u64 stored as i64 in SQLite: read them with `get::<_, i64>(n)? as SymbolId`.
- **Rerank weights sum to ~1.0** (β=0.667, γ=0.333); `Config::validate()` warns beyond ±0.05. There is no vector weight.
- **MCP tools stay at 4:** `search_code` (exactly one of `intent` / `implements`), `inspect` (exactly one of `symbol` / `path`; an ambiguous multi-file `path` is a hard error), `trace_impact`, `watch_status`. More listed tools cause more agent mis-picks (Plan 23). `search_code`/`inspect` are thin routers over the older handler modules in `graphrelay-mcp/src/tools/`. Adding a tool means updating both `handle_tools_list()` and `handle_tools_call()`. Tool descriptions follow one shape (what / USE WHEN / DON'T USE / FLOW / OUTPUT / LIMITS) with caps interpolated from handler constants; a test pins it (Plan 31).
- **`chunk_fts` has no foreign-key cascade from `symbols`.** `GraphDb::delete_file()` deletes the matching rows by rowid (`== symbol_id`) in the same transaction. Do not add a delete path that skips this.
- **Synthesized edges are additive and rebuilt.** Everything in `EdgeSource::Synthesized` is deleted (`delete_synthesized_edges`) and regenerated on every full index, so a synthesis pass never needs a migration. Synthesis failures are warn-only: after changing a pass, check the log line "N edge(s) recovered".

## Retrieval pipeline — stage order matters

```
lexical_search()        → top-K symbols from FTS5 (bm25(), real corpus IDF): the sole candidate channel
expand_graph()          → + callers/callees of lexical hits (1 hop), score decay
personalized_pagerank() → query-seeded graph centrality
enrich()                → full Symbol + snippet
rerank()                → β·graph_score + γ·max(BM25_norm, lexical_score)
                          (+ name-token bonus, + damped graph credit for direct hits),
                          sorted with a SymbolId tiebreak for determinism
apply_jev()             → optional, off by default: widen the pool to jev.top_n, add jev.weight × relevance,
                          re-sort, truncate to final_n. Any judge error falls back to the ranking above.
```

Stages live in `graphrelay-core/src/retrieval/`; `RetrievalPipeline<G>` owns only a `GraphStore`. Details worth knowing:
- **Expansion is one hop.** `GraphDb::neighbors()` ignores `hops` beyond non-zero, so `expand_hops.max(2)` only changes the decay multiplier (0.6 vs 0.36). Real 2-hop traversal is an open experiment (Plan 21 item 15).
- **Lexical stage:** `query()` adds each whole camelCase identifier in the query as an extra FTS term (rerank still uses `tokenize_query_split`); `fts_match` skips terms in more than `FTS_MAX_DF_FRACTION` (0.2) of chunks on indexes of ≥1,000 chunks; indexed text decomposes identifiers into stemmed tokens (`tokenize_name`, shared with `tokenize_query` via `split_words`), and BM25 `avg_len`/`doc_len` use the same tokens. `TEST_FILE_PENALTY` demotes test files unless the query is about tests.
- **A language filter applies before the candidate budget** (4× over-fetch), and pinned hits are never truncated by it.
- **`watch_status.semantic_pending`** is the lexical-indexing backlog; the name is a leftover, not an embedding queue.
- Tuning history (stemming, synonym table, PPR, `name_match_bonus` 0.15→0.22, the `getActiveEditorState` tie-break, the keycloak generalization gap) was measured on the retired suites. The full text is in the history of this file at `939f6b7`; do not re-attempt a rejected lever (token-fraction reward, BM25 column weights, caller-credit, relative score floor) without re-running neighbouring parameter values on repeated runs. Treat any single-agent "improvement" claim as unproven until independently re-measured.

## Indexing pipeline

Two strictly ordered phases (`IndexWorker::index_path_graph_only()` then `flush_chunk_batch()`, in `graphrelay-watcher/src/worker.rs`):
1. SQLite: upsert the file record, symbols and edges; resolve cross-file edges.
2. Build graph-aware chunks (queries SQLite for neighbors, so it must follow phase 1) and write them to the FTS5 index.

After a full index, `run_dispatch_synthesis` (gated on `[watcher] synthesis_enabled`, default `true`) adds `Synthesized` edges in chunks of 200 files: `c_fnptr` (C function pointers), `js_callback` (`.on()`/`.emit()` pairs), `py_string_dispatch` (dotted-string targets such as `frappe.enqueue("a.b.func")`, Plan 35) and `py_self_calls` (`self.method()` up the `Inherits` chain, Plan 36). ERPNext gets ~365 such edges: real, but too few to move a 30-question mean. A superseded entry (a newer live save during a full index) is re-indexed once from disk in a batch (`reindex_superseded_batch`).

Rapid-save throughput is ~85 ms per file in a debug build (100 files ≈ 8 s), so `rapid_saves_p95_latency_and_no_duplicates` has a 45 s deadline for slower CI runners.

## Config (`graphrelay.toml`)

Config discovery walks CWD upward; `GRAPHRELAY_DATA_DIR` overrides the data directory. Data lives in `~/.graphrelay/<hash8-of-root>/` per project. A project indexed before Plan 20 may still have `~/.graphrelay/<hash8>/vectors/` (the old LanceDB store); it is never read and is safe to delete.

## Known gaps

- Cross-file call resolution is by bare name. On ERPNext ~31% of Python call edges resolve; ~83% of the rest target external code (Frappe, stdlib, tests), ~17% are ambiguous method calls. `get_doc({"doctype": …})`, `Class.method` string targets and `hooks.py` tables are not linked.
- JS `.on()`/`.emit()` synthesis produced 0 edges on every benchmark corpus (no real `EventEmitter` idiom in them); untested against one that uses it both ways.
- Astro SFC delegation is not done (Vue and Svelte are).
- `memory_budget::resolve_pool_size()` reads live `sysinfo` memory, not a cgroup limit: set `[watcher] memory_budget_mb` in containers.
- `extract_incremental()` is a real incremental parser (~1.3× on a 6,400-line file); making the symbol/edge queries incremental via `Tree::changed_ranges()` is not attempted.
- No large-repo (Linux kernel / Swift compiler scale) indexing benchmark has been run; ERPNext (4,825 files) indexes in ~30 s on an idle 8-core machine.
- Plan 28 leftovers: no read-only connection pool, and the `incremental_cache` / `SourceLoader` caches are unbounded.

## Languages supported

31 languages via tree-sitter: Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, C#, Ruby, Kotlin, Swift, PHP, Scala, Lua (+ tsx/jsx), Vue, Svelte (SFC `<script>` delegated to JS/TS), Bash, Zig, Elixir, Julia, Fortran, OCaml, Common Lisp, Verilog/SystemVerilog, Pascal, Dart, Objective-C, Groovy, PowerShell, and JSON (top-level keys only). `LanguageRegistry::registered_languages()` is the single source of truth; `graphrelay-core::config::default_languages()` must stay in sync (checked by the `default_languages_cover_every_registered_language` test). The watcher derives file extensions from the registry, never from a hand-kept copy.

Adding a language needs: a tree-sitter grammar in `Cargo.toml` that is ABI 14 (read the candidate's `src/parser.c` `#define LANGUAGE_VERSION`; ABI 15 is incompatible with our `tree-sitter = "0.24"`, and "latest" is not safe), a `LanguageSupport` impl in `graphrelay-parser/src/language/`, and registry registration. The test `every_symbol_query_compiles_against_its_grammar` compiles every language's queries. Binary size grows a lot with grammar count (parse tables sit in read-only data): Verilog and OCaml are ~15-19 MB each; measure the real stripped binary before estimating.

## Coding conventions

- No comments unless the WHY is non-obvious (hidden constraint, workaround, subtle invariant). No docstrings on internal functions.
- `anyhow::Result` in binary/application code; `thiserror` for library error types.
- Async only at the boundary (axum handlers, watcher loop). Pure logic in `graphrelay-core` is synchronous and free of `tokio`, `axum` and I/O traits.

## Ship gate

`checklist.md` has the full list. Hard gates before tagging:
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass.
- A `benchmarks/dataset/run-sot.sh --repeats 3` run is recorded under `results/sot/` for each release candidate, and a coverage drop for the `graphrelay` arm against the previous run is investigated before tagging. No numeric gate exists (the old MRR and p95 gates were defined on retired suites).
- Binary ≤ 200 MB stripped (raised from 50 MB when Plan 19 added grammars; currently ~78 MB).
- All 4 MCP tools verified end to end with Claude Code.
