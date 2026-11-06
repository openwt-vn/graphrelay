# Plan 05 — Hybrid Retrieval Pipeline

## Goal
Implement the core differentiator: **two-stage fused retrieval** (vector search → graph expansion → rerank) that produces a single ranked list of `RetrievalResult`, achieving MRR > 0.5 on internal benchmarks.

## Success Criteria
- Full retrieval pipeline (embed query + ANN + graph expand + rerank): < 100 ms p95
- MRR > 0.5 measured on benchmark test suite (see Plan 10)
- Callers and callees of top hits are always included in results
- Results are stable (same query → same order, no randomness)
- Pipeline is testable without a running server (pure function on in-memory stores)

---

## 1. Pipeline Overview

```
query: &str
  │
  ▼ Stage 0: Query embedding
  embed(query) → f32[384]
  │
  ▼ Stage 1: Vector search
  LanceDB ANN top-K (K=20 default) → Vec<VectorHit>
  │    VectorHit { symbol_id, chunk_text, vec_score }
  │
  ▼ Stage 2: Graph expansion
  For each VectorHit.symbol_id:
    SQLite: callers_of(id) + callees_of(id)  (1 hop)
  Deduplicate all symbol_ids → expanded_set
  │
  ▼ Stage 3: Enrichment
  SQLite: fetch full Symbol + file snippets for expanded_set
  │
  ▼ Stage 4: Scoring & rerank
  score(s) = α·vec_score(s) + β·in_degree_norm(s) + γ·recency(s)
  Sort descending → top-N (N=5 default)
  │
  ▼ Stage 5: Serialization
  Vec<RetrievalResult>  →  MCP tool / REST response
```

---

## 2. Type Definitions

```rust
// crates/codegraph-core/src/retrieval/mod.rs

pub struct RetrievalQuery {
    pub text:       String,
    pub top_k:      usize,       // vector search K (default 20)
    pub final_n:    usize,       // final result count (default 5)
    pub expand_hops: u8,         // graph hops (default 1)
    pub lang_filter: Option<String>,
}

pub struct RetrievalResult {
    pub rank:       usize,
    pub score:      f32,
    pub symbol:     Symbol,
    pub file_path:  PathBuf,
    pub snippet:    String,       // source lines around the symbol
    pub callers:    Vec<SymbolRef>,
    pub callees:    Vec<SymbolRef>,
    pub vec_score:  f32,          // raw vector cosine similarity
}

pub struct SymbolRef {
    pub id:             SymbolId,
    pub qualified_name: String,
    pub kind:           SymbolKind,
}
```

---

## 3. Stage 0 — Query Embedding

```rust
async fn embed_query(
    model: &dyn EmbeddingModel,
    text: &str,
) -> anyhow::Result<Vec<f32>> {
    // BGE asymmetric: query prefix for retrieval
    let prefixed = format!(
        "Represent this sentence for searching relevant passages: {text}"
    );
    model.embed_one(&prefixed).await
}
```

---

## 4. Stage 1 — Vector Search

```rust
async fn vector_search(
    vector_db: &VectorDb,
    query_vec: &[f32],
    query: &RetrievalQuery,
) -> anyhow::Result<Vec<VectorHit>> {
    let filter = query.lang_filter.as_deref().map(|l| format!("language = '{l}'"));
    vector_db
        .search(query_vec, query.top_k, filter.as_deref())
        .await
}
```

This returns up to `top_k` hits ordered by cosine similarity descending.

---

## 5. Stage 2 — Graph Expansion

```rust
fn expand_graph(
    graph_db: &GraphDb,
    hits: &[VectorHit],
    hops: u8,
) -> anyhow::Result<HashMap<SymbolId, f32>> {
    // Map: symbol_id → best_vec_score (used for scoring expanded nodes)
    let mut candidates: HashMap<SymbolId, f32> = HashMap::new();

    for hit in hits {
        // Original hit keeps its score
        candidates.entry(hit.symbol_id)
            .and_modify(|s| *s = s.max(hit.vec_score))
            .or_insert(hit.vec_score);

        // Expand neighbors (cap total candidate set at 100 to prevent explosion on hub symbols)
        if candidates.len() >= 100 { continue; }
        let neighbors = graph_db.neighbors(hit.symbol_id, hops)?;
        for neighbor in neighbors {
            if candidates.len() >= 100 { break; }
            // Neighbors inherit a decayed score
            let decayed = hit.vec_score * 0.6_f32.powi(hops as i32);
            candidates.entry(neighbor.id)
                .and_modify(|s| *s = s.max(decayed))
                .or_insert(decayed);
        }
    }

    Ok(candidates)
}
```

Decay factor 0.6 per hop means: a direct hit scores at 100% of vec_score, a 1-hop neighbor at 60%, a 2-hop at 36%.

---

## 6. Stage 3 — Enrichment

Fetch full Symbol records and source snippets for all candidates:

```rust
fn enrich(
    graph_db: &GraphDb,
    source_loader: &SourceLoader,   // reads file content from disk, cached
    candidates: &HashMap<SymbolId, f32>,
) -> anyhow::Result<Vec<EnrichedCandidate>> {
    let mut out = Vec::new();

    for (&sym_id, &inherited_vec_score) in candidates {
        let Some(symbol) = graph_db.symbol_by_id(sym_id)? else { continue };
        let callers  = graph_db.callers_of(sym_id)?;
        let callees  = graph_db.callees_of(sym_id)?;
        let in_degree = callers.len() as f32;
        let snippet  = source_loader.snippet(&symbol)?;

        out.push(EnrichedCandidate {
            symbol, callers, callees, in_degree,
            vec_score: inherited_vec_score,
            snippet,
        });
    }

    Ok(out)
}
```

---

## 7. Stage 4 — Scoring & Rerank

```rust
fn rerank(
    candidates: Vec<EnrichedCandidate>,
    config: &RetrievalConfig,
    final_n: usize,
) -> Vec<RetrievalResult> {
    let max_in_degree = candidates.iter()
        .map(|c| c.in_degree)
        .fold(0f32, f32::max)
        .max(1.0);

    let mut scored: Vec<(f32, EnrichedCandidate)> = candidates
        .into_iter()
        .map(|c| {
            let in_degree_norm = c.in_degree / max_in_degree;
            // v1: α + β must sum to 1.0; γ (recency) is 0 until indexed_at per-symbol lands
            // v1 defaults: α=0.8, β=0.2, γ=0.0 — config validation ensures sum ≈ 1.0
            let score = config.alpha * c.vec_score
                      + config.beta  * in_degree_norm;
            (score, c)
        })
        .collect();

    // Stable sort (same input → same output)
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    scored.into_iter()
        .take(final_n)
        .enumerate()
        .map(|(rank, (score, c))| RetrievalResult {
            rank: rank + 1,
            score,
            vec_score:  c.vec_score,
            symbol:     c.symbol,
            file_path:  c.symbol.file_path.clone(),
            snippet:    c.snippet,
            callers:    c.callers.into_iter().map(SymbolRef::from).collect(),
            callees:    c.callees.into_iter().map(SymbolRef::from).collect(),
        })
        .collect()
}
```

Default weights for v1 (tunable via config):
- `α = 0.8` — semantic relevance is primary signal
- `β = 0.2` — high in-degree (popular symbols) get a boost
- `γ = 0.0` — recency bonus disabled in v1 (no per-symbol `indexed_at` yet); enable in v2 with `α=0.7, β=0.2, γ=0.1`

**Important**: `config.alpha + config.beta` must equal 1.0 in v1. `Config::validate()` (Plan 11 §3) warns if the sum deviates by > 0.05.

---

## 8. Orchestrator

```rust
// crates/codegraph-core/src/retrieval/pipeline.rs

pub struct RetrievalPipeline {
    pub model:        Arc<dyn EmbeddingModel>,
    pub vector_db:    Arc<VectorDb>,
    pub graph_db:     Arc<GraphDb>,
    pub source_cache: Arc<SourceLoader>,
    pub config:       RetrievalConfig,
}

impl RetrievalPipeline {
    pub async fn query(&self, req: RetrievalQuery) -> anyhow::Result<Vec<RetrievalResult>> {
        // Stage 0
        let query_vec = embed_query(&*self.model, &req.text).await?;

        // Stage 1
        let hits = vector_search(&self.vector_db, &query_vec, &req).await?;
        if hits.is_empty() {
            return Ok(vec![]);
        }

        // Stage 2
        let candidates = expand_graph(&self.graph_db, &hits, req.expand_hops)?;

        // Stage 3
        let enriched = enrich(&self.graph_db, &self.source_cache, &candidates)?;

        // Stage 4
        Ok(rerank(enriched, &self.config, req.final_n))
    }
}
```

---

## 9. Source Loader (Snippet Extraction)

```rust
pub struct SourceLoader {
    cache: DashMap<PathBuf, Arc<String>>,
}

impl SourceLoader {
    pub fn snippet(&self, symbol: &Symbol) -> anyhow::Result<String> {
        let source = self.load(&symbol.file_path)?;
        let lines: Vec<&str> = source.lines().collect();

        // ±3 lines context window around symbol definition
        let start = symbol.span.start_line.saturating_sub(3) as usize;
        let end   = (symbol.span.end_line as usize + 3).min(lines.len());

        Ok(lines[start..end].join("\n"))
    }

    fn load(&self, path: &PathBuf) -> anyhow::Result<Arc<String>> {
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }
        let content = Arc::new(std::fs::read_to_string(path)?);
        self.cache.insert(path.clone(), content.clone());
        Ok(content)
    }

    /// Called by IndexWorker after each file is re-indexed so snippet() returns fresh content.
    /// Must be called BEFORE build_chunks() in index_path() (see Plan 07 §4).
    pub fn invalidate(&self, path: &Path) {
        self.cache.remove(path);
    }
}
```

**Cache invalidation contract**: `IndexWorker::index_path()` (Plan 07) calls `source_loader.invalidate(path)` immediately after the SQLite write and before building chunks. This eliminates the race where a concurrent HTTP request reads a stale snippet while the watcher is mid-update.

---

## 10. `trace_impact` Implementation

Used by the `trace_impact` MCP tool — finds all symbols that would break if a given symbol changed:

```rust
pub fn trace_impact(
    graph_db: &GraphDb,
    symbol_id: SymbolId,
    max_depth: u8,
) -> anyhow::Result<ImpactGraph> {
    let affected = graph_db.impact_graph(symbol_id, max_depth)?;

    Ok(ImpactGraph {
        root:     symbol_id,
        affected: affected.iter()
            .map(|s| ImpactNode {
                symbol: s.clone(),
                path:   s.file_path.clone(),
                reason: "directly or transitively calls the changed symbol".to_string(),
            })
            .collect(),
        depth:    max_depth,
    })
}
```

---

## 11. Retrieval Config

```toml
# codegraph.toml
[retrieval]
vector_top_k  = 20     # K for ANN search
expand_hops   = 1      # graph hops after vector stage
final_top_n   = 5      # N returned to agent
alpha         = 0.7    # vector score weight
beta          = 0.2    # in-degree weight
gamma         = 0.1    # recency bonus weight (v2)
```

---

## 12. Implementation Steps

1. Define all types in `codegraph-core/src/retrieval/mod.rs`
2. Implement `embed_query()` with BGE prefix
3. Implement `vector_search()` delegating to `VectorDb`
4. Implement `expand_graph()` with hop decay
5. Implement `SourceLoader` with `DashMap` cache
6. Implement `enrich()` fetching callers/callees + snippets
7. Implement `rerank()` with configurable weights
8. Implement `RetrievalPipeline::query()` orchestrator
9. Implement `trace_impact()` using SQLite recursive CTE
10. Unit test each stage in isolation with mock stores
11. Integration test: index 3-file fixture, query → verify top result is correct, MRR = 1.0
12. Benchmark end-to-end pipeline: must be < 100 ms p95

---

## 13. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Graph expansion scope | Callers only / callees only / both | Both — gives richer structural context |
| Hop decay | 0.5 / 0.6 / 0.7 | 0.6 — tunable; expose in config |
| Expanded set size cap | Unlimited / cap at 100 | Cap at 100 — prevents explosion on highly-connected nodes |
| Source loader cache | Unbounded / LRU(100) | LRU(100) — avoids OOM on large repos |
| Recency signal | File mtime / SQLite indexed_at | SQLite `indexed_at` — more reliable than mtime on some filesystems |
