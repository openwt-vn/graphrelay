# How retrieval works

## Pipeline

```
lexical_search()              →  top-K symbols from SQLite FTS5 (real candidate
                                  generation: BM25, camelCase/snake_case
                                  decomposition, stemming, synonym expansion)
expand_graph()                →  + callers/callees/inherits/implements/references
                                  of the top hits, score decay
personalized_pagerank()       →  query-seeded graph-centrality score
enrich()                      →  full Symbol + source snippet
rerank()                      →  β·graph_score + γ·BM25
                                  (+ name-token bonus, + damped graph credit
                                  for direct hits)
```

No embedding model, no vector index, no cloud call — every stage above is a local SQLite query or an in-memory graph walk. This isn't a fallback mode; it's the only pipeline graphrelay has. See [How retrieval works without embeddings](#how-retrieval-works-without-embeddings) below for the honest tradeoffs that come with that choice.

Indexing is incremental and fast enough that you generally won't notice it happening:

```
file change (notify-rs, 50 ms debounce)
  → tree-sitter parse
  → SQLite upsert (symbols + edges)
  → graph-aware chunk build
  → SQLite FTS5 upsert (lexical index)
```

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                  graphrelay (single process)              │
│                                                          │
│  ┌──────────┐   ┌──────────┐   ┌──────────────────────┐ │
│  │ Watcher  │──▶│ Indexer  │──▶│ SQLite                │ │
│  │(notify)  │   │(tree-sit)│   │ (graph + FTS5 lexical)│ │
│  └──────────┘   └──────────┘   └──────────────────────┘ │
│                                          │               │
│  ┌───────────────────────────────────────▼─────────────┐ │
│  │     lexical-search → graph-expand → rerank          │ │
│  └──────────────────┬──────────────────────────────────┘ │
│                     │                                    │
│  ┌──────────────────▼──────────────────────────────────┐ │
│  │       HTTP (axum :3030)                             │ │
│  │   /mcp  ·  /api/*  ·  / (web UI)                   │ │
│  └─────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

Everything above runs in one process, on one port. There's no separate database server to start, no message queue, no sidecar — the daemon watches your files, keeps the index current, and answers queries, all by itself.

---

## Retrieval without embeddings

graphrelay has no embedding model, full stop — no download, no ONNX inference, no vector index, and no config flag to turn one on. This isn't a degraded fallback mode sitting next to a "real" one; it's the only retrieval pipeline graphrelay has. Every ranked result comes from SQLite FTS5 full-text search (real corpus-wide BM25, not just a tiebreaker), camelCase/snake_case identifier decomposition, a curated synonym table, and personalized-PageRank graph centrality, with graph expansion widened to compensate for there being no vector recall at all.

Why build it this way rather than add embeddings back in:

- **It's fast to index and fully offline from the first run.** No model download, no inference cost, no network dependency ever — nothing to fetch before you get a working index.
- **It uses less memory** — no inference runtime held open.
- **It's a real, if honest, retrieval-quality tradeoff, not a free lunch.** A query whose vocabulary doesn't overlap the code's has no path to a match the way an embedding's learned "meaning" similarity would; this pipeline needs real vocabulary overlap between the query and the code. (Earlier per-suite hit rates quoted here came from retrieval-only benchmarks retired on 2026-09-30; see Benchmarks.)

---
