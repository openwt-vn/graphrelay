# Plan 10 — Benchmarking & Validation

## Goal
Establish honest, reproducible measurements for token reduction, retrieval quality (MRR), indexing performance, and query latency — and set a clear bar for "v1 is shippable."

## Success Criteria (v1 ship gate)
- MRR > 0.5 on retrieval benchmark
- Token reduction ≥ 8× average on the 6-repo suite used by code-review-graph (so claims are comparable)
- Cold-start to first query < 2 s on a 100k LOC repo
- Incremental re-index latency < 500 ms p95
- Full pipeline query latency < 100 ms p95
- Binary size < 50 MB (not counting downloaded ONNX model)

---

## 1. Benchmark Suite — Repos

Use repos that are publicly available, representative, and have known ground truth for code navigation:

| # | Repo | LOC | Language(s) | Why |
|---|------|-----|-------------|-----|
| 1 | `rust-lang/regex` | ~25k | Rust | Pure Rust, well-structured |
| 2 | `tokio-rs/tokio` | ~60k | Rust | Large, many cross-module calls |
| 3 | `pallets/flask` | ~35k | Python | Popular Python codebase |
| 4 | `expressjs/express` | ~10k | JS | Small JS, well-known |
| 5 | `golang/go` stdlib | ~300k | Go | Large Go codebase |
| 6 | `microsoft/TypeScript` | ~300k+ | TypeScript | Large TS self-hosting |

For the MRR benchmark, use repos 1–4 (where we can create ground truth manually). Repos 5–6 for scalability testing.

---

## 2. MRR Benchmark

### Ground Truth Creation

For each benchmark repo, create a `benchmarks/<repo>/queries.json` file:

```json
[
  {
    "query": "how does the regex DFA construction work?",
    "relevant_symbols": [
      "src/dfa.rs::Dfa::new",
      "src/dfa.rs::Dfa::add_state",
      "src/compile.rs::Compiler::compile"
    ]
  },
  {
    "query": "where is the HTTP request parsing logic?",
    "relevant_symbols": [
      "lib/request.js::Request::parse",
      "lib/middleware.js::bodyParser"
    ]
  }
]
```

Target: 30–50 queries per repo, 200 queries total. Label the top-3 relevant symbols manually.

### MRR Calculation

```rust
// benchmarks/src/mrr.rs

pub fn compute_mrr(results: &[QueryResult]) -> f64 {
    let reciprocal_ranks: Vec<f64> = results.iter().map(|r| {
        // Find the rank of the first relevant result
        r.retrieved.iter()
            .enumerate()
            .find(|(_, sym)| r.relevant.contains(sym))
            .map(|(rank, _)| 1.0 / (rank + 1) as f64)
            .unwrap_or(0.0)
    }).collect();

    reciprocal_ranks.iter().sum::<f64>() / reciprocal_ranks.len() as f64
}
```

Run:
```bash
cargo bench --bench mrr -- --repos regex flask express tokio
```

Target: MRR > 0.5 (vs. code-review-graph's self-reported 0.35).

---

## 3. Token Reduction Benchmark

### Methodology

For each query in the benchmark suite, compare:
- **Baseline (sequential read)**: tokens consumed by reading all files in the repo until the answer is found
- **codegraph**: tokens in the `find_relevant_code` response

```rust
pub struct TokenReductionResult {
    pub query:           String,
    pub baseline_tokens: usize,   // tiktoken count of sequential file read
    pub codegraph_tokens: usize,  // tiktoken count of retrieval response
    pub reduction_factor: f64,
}

pub fn measure_token_reduction(query: &Query, repo_path: &Path) -> TokenReductionResult {
    // Baseline: read all source files until the relevant symbol is found
    let baseline = estimate_sequential_read_tokens(repo_path);

    // codegraph: actual response from find_relevant_code
    let response  = retrieve(query);
    let cg_tokens = count_tokens(&serde_json::to_string(&response).unwrap());

    TokenReductionResult {
        query:            query.text.clone(),
        baseline_tokens:  baseline,
        codegraph_tokens: cg_tokens,
        reduction_factor: baseline as f64 / cg_tokens as f64,
    }
}
```

Report avg and peak reduction across the 6-repo suite.

Target: ≥ 8× average (same standard as code-review-graph's published metric).

---

## 4. Indexing Performance Benchmark

```rust
// benchmarks/src/indexing.rs

pub async fn bench_full_index(repo_path: &Path) -> IndexingStats {
    let start = Instant::now();

    let (graph_db, vector_db) = open_stores_temp().await;
    let pipeline = build_pipeline(graph_db.clone(), vector_db.clone());

    full_index(repo_path, &pipeline).await.unwrap();

    let elapsed      = start.elapsed();
    let total_symbols = graph_db.total_symbol_count().unwrap();
    let total_files   = graph_db.total_file_count().unwrap();

    IndexingStats {
        repo:          repo_path.to_owned(),
        elapsed,
        total_symbols,
        total_files,
        symbols_per_sec: total_symbols as f64 / elapsed.as_secs_f64(),
    }
}
```

Targets:
- 100k LOC → full index < 2 s (cold-start to first query)
- 300k LOC → < 10 s (acceptable for large repos)

---

## 5. Incremental Re-index Latency

```rust
pub async fn bench_incremental(repo_path: &Path, file: &Path) -> Duration {
    // Pre-index the repo
    let watcher = setup_watcher(repo_path).await;

    // Modify a file
    let original = fs::read_to_string(file).unwrap();
    let modified = original.replace("fn foo(", "fn foo_modified(");
    fs::write(file, &modified).unwrap();

    let start = Instant::now();

    // Wait for watcher to complete re-index
    watcher.wait_for_fresh().await;

    let elapsed = start.elapsed();

    // Restore file
    fs::write(file, original).unwrap();
    elapsed
}
```

Run 50 trials across 5 different files. Report p50, p95, p99.

Target: p95 < 500 ms.

---

## 6. Query Latency Benchmark

```rust
pub async fn bench_query_latency(
    pipeline: &RetrievalPipeline,
    queries:  &[&str],
    n_runs:   usize,
) -> LatencyStats {
    let mut latencies = Vec::new();

    for _ in 0..n_runs {
        for query in queries {
            let start = Instant::now();
            pipeline.query(RetrievalQuery { text: query.to_string(), .. }).await.unwrap();
            latencies.push(start.elapsed());
        }
    }

    LatencyStats::from_samples(latencies)
}

pub struct LatencyStats {
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
}
```

Target: p95 < 100 ms on a warm index.

---

## 7. Binary Size Budget

```bash
cargo build --release
ls -lh target/release/codegraph
# Target: < 50 MB (excluding ONNX model file)
```

If over budget:
- Enable `opt-level = "z"` and `lto = true` in `[profile.release]`
- Strip debug symbols: `strip = true`
- Check for large embedded assets (vendor JS)

---

## 8. Benchmark CLI

Add a `bench` subcommand to the binary:

```bash
codegraph bench --repo ~/repos/flask --queries benchmarks/flask/queries.json
# Output:
# MRR:              0.61
# Avg token reduction: 11.3×
# Peak token reduction: 47×
# Query p95:        82 ms
# Full index time:  1.4 s (35,000 LOC)
```

---

## 9. CI Benchmark Regression Guard

Add to CI (runs on main branch only, not PRs — too slow):

```yaml
# .github/workflows/bench.yml
name: Benchmarks
on:
  push:
    branches: [main]

jobs:
  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: git clone --depth=1 https://github.com/pallets/flask /tmp/flask
      - run: git clone --depth=1 https://github.com/expressjs/express /tmp/express
      - run: cargo build --release
      - run: |
          ./target/release/codegraph bench \
            --repo /tmp/flask \
            --queries benchmarks/flask/queries.json \
            --assert-mrr 0.50 \
            --assert-p95-ms 100
      - run: |
          ./target/release/codegraph bench \
            --repo /tmp/express \
            --queries benchmarks/express/queries.json \
            --assert-mrr 0.50
```

`--assert-mrr` and `--assert-p95-ms` flags make the command exit non-zero if thresholds aren't met.

---

## 10. Call Resolution Accuracy Benchmark

The spec claims "~5–10% accuracy loss vs. LSP for 'find references / rename'." Validate this claim:

```rust
pub struct CallResolutionResult {
    pub total_call_sites:     usize,
    pub resolved_local:       usize,   // Stage 1 hits
    pub resolved_import:      usize,   // Stage 2 hits
    pub resolved_fuzzy:       usize,   // Stage 3 hits
    pub unresolved:           usize,   // all stages failed
    pub resolution_rate:      f64,     // (resolved / total)
    pub false_positive_rate:  f64,     // wrong target / resolved (requires ground truth)
}
```

Ground truth: manually annotate 100 call sites across the benchmark repos (50 in `regex`, 50 in `flask`), noting the correct target symbol. Compare against what codegraph resolves.

Target: ≥ 90% resolution rate, ≤ 10% false positive rate (validates the spec's stated tradeoff).

---

## 11. Privacy Audit

Since the spec's key differentiator is "no data leaves the machine", add a network traffic check to CI:

```yaml
# In bench.yml CI job
- name: Privacy audit — no outbound connections during indexing
  run: |
    # Run indexing under strace (Linux) or tcpdump and assert no unexpected connections
    strace -e trace=connect -f ./target/release/codegraph \
      --index-only --root /tmp/flask 2>&1 | grep -v "127.0.0.1\|ENOENT\|ECONNREFUSED" \
      | grep "connect(" && echo "FAIL: unexpected network call" && exit 1 || echo "PASS"
```

For macOS CI (no strace): use `lsof -i` snapshot before and after indexing, diff for new connections.

Acceptable connections: none (model weights are pre-downloaded; no runtime network calls in default mode).

---

## 12. Comparison Table (to be filled after measurement)

| Metric | codegraph (target) | codegraph (actual) | code-review-graph | Context Engine |
|--------|-------------------|-------------------|-------------------|----------------|
| MRR | > 0.50 | TBD | 0.35 (self-reported) | n/a |
| Avg token reduction | ≥ 8× | TBD | 8.2× | claimed, unmeasured |
| Peak token reduction | — | TBD | 49× | — |
| Index: 100k LOC cold-start | < 2 s | TBD | ~10 s | daemon-based |
| Incremental update p95 | < 500 ms | TBD | < 2 s | configurable |
| Query latency p95 | < 100 ms | TBD | — | — |
| Call resolution rate | ≥ 90% | TBD | n/a | n/a |
| Call false-positive rate | ≤ 10% | TBD | n/a | n/a |
| Network calls during index | 0 | TBD | 0 | cloud UI |

---

## 11. Implementation Steps

1. Create `benchmarks/` directory with `Cargo.toml` (bench crate)
2. Write `benchmarks/src/mrr.rs` with `compute_mrr()` and query runner
3. Write `benchmarks/src/indexing.rs` with `bench_full_index()`
4. Write `benchmarks/src/latency.rs` with `bench_query_latency()`
5. Create ground-truth `queries.json` files for regex, flask, express (30 queries each)
6. Implement `bench` subcommand in main binary
7. Run benchmarks against all 4 repos; record results
8. Add CI workflow that asserts MRR ≥ 0.50 and p95 ≤ 100 ms on each push to main
9. Update comparison table with actual measurements
10. Publish results in README

---

## 12. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Token counting | tiktoken / estimate | tiktoken-rs (Rust binding) — same tokenizer as GPT-4/Claude |
| Ground truth creation | Manual / automated | Manual for quality — label 200 queries once, then automated runs |
| Benchmark cadence | Every PR / main-only | Main-only — too slow for PR CI |
| Baseline comparison | Sequential read / grep | Sequential read (simulates naive agent) — same standard as code-review-graph |
| Reporting format | JSON / human-readable | Both — JSON for CI assertions, human-readable for display |
