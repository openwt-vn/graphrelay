# Plan 11 — Startup, Operations & AppState Wiring

## Goal
Define the complete startup sequence, `AppState` assembly, first-run experience (model download, config discovery), readiness signaling, graceful shutdown, and operational concerns (data cleanup, error propagation, queue overflow) that were missing across Plans 01–10.

## Success Criteria
- Binary is ready to serve MCP/REST requests within < 2 s on a warm machine (model already downloaded)
- First-run prints clear progress and doesn't hang silently
- `GET /health` returns `{"ready": true}` only after full_index completes
- SIGTERM / SIGINT triggers graceful shutdown: in-flight requests finish, watcher flushes pending batch
- Config file is discovered without requiring `--config` when a `codegraph.toml` or Git root exists
- Invalid config entries produce clear startup errors, not panics

---

## 1. Startup Sequence

```
main()
  │
  ├─ 1. Parse CLI args (port, config path override)
  │
  ├─ 2. Discover config (CWD upward search or --config flag)
  │
  ├─ 3. Load & validate Config  ←── error → print diagnostic, exit 1
  │
  ├─ 4. Ensure ONNX model exists
  │        ├─ Already cached → continue
  │        └─ Missing → download with progress bar → verify sha256
  │
  ├─ 5. Load EmbeddingModel (ONNX session open + warmup inference)
  │
  ├─ 6. Open stores (SQLite WAL + LanceDB)
  │        └─ Run SQL migrations if needed
  │
  ├─ 7. Build AppState (wrap everything in Arc<>)
  │
  ├─ 8. Bind HTTP server to :3030  ←── error → "port in use", exit 1
  │
  ├─ 9. Run full_index() with progress reporting
  │        └─ Set IndexState = Indexing
  │
  ├─ 10. Spawn watcher task
  │
  ├─ 11. Set IndexState = Fresh; emit "Ready on http://localhost:3030"
  │
  └─ 12. Serve HTTP requests until SIGTERM / SIGINT
           └─ Graceful shutdown (see §8)
```

---

## 2. Config Discovery

Search for `codegraph.toml` in this priority order:

1. `--config <path>` CLI flag (absolute or relative to CWD)
2. `CODEGRAPH_CONFIG` environment variable
3. CWD and each parent directory (upward walk) for `codegraph.toml`
4. CWD and each parent for `.git/` (use that dir as project root, load defaults)
5. If nothing found: use defaults with `root = "."` (CWD as workspace root)

```rust
// src/config_discovery.rs

pub fn find_config(explicit: Option<&Path>) -> anyhow::Result<(Config, PathBuf)> {
    // 1. Explicit path
    if let Some(p) = explicit {
        let cfg = Config::load(p)?;
        return Ok((cfg, p.parent().unwrap_or(Path::new(".")).to_owned()));
    }

    // 2. Env var
    if let Ok(p) = std::env::var("CODEGRAPH_CONFIG") {
        let p = PathBuf::from(p);
        let cfg = Config::load(&p)?;
        return Ok((cfg, p.parent().unwrap_or(Path::new(".")).to_owned()));
    }

    // 3. Upward search for codegraph.toml
    let mut dir = std::env::current_dir()?;
    loop {
        let candidate = dir.join("codegraph.toml");
        if candidate.exists() {
            let cfg = Config::load(&candidate)?;
            return Ok((cfg, dir));
        }
        // 4. Git root fallback
        if dir.join(".git").exists() {
            let cfg = Config::defaults_for_root(&dir);
            return Ok((cfg, dir));
        }
        match dir.parent() {
            Some(p) => dir = p.to_owned(),
            None    => break,
        }
    }

    // 5. Default: CWD
    let dir = std::env::current_dir()?;
    Ok((Config::defaults_for_root(&dir), dir))
}
```

---

## 3. Config Validation

```rust
// crates/codegraph-core/src/config.rs

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        // Embedding model
        let known_models = ["bge-small-en-v1.5", "bge-code-v1", "voyage-code-3"];
        if !known_models.contains(&self.embedding.model.as_str()) {
            anyhow::bail!(
                "unknown embedding model '{}'. Known models: {}",
                self.embedding.model,
                known_models.join(", ")
            );
        }

        // Languages
        let known_langs = ["rust","typescript","javascript","python","go","java",
                           "c","cpp","csharp","ruby","kotlin","swift","php","scala"];
        for lang in &self.index.languages {
            if !known_langs.contains(&lang.as_str()) {
                // Warn but don't fail — unknown languages are silently skipped
                tracing::warn!("unknown language '{}' in config — skipping", lang);
            }
        }

        // Port range
        if self.server.port < 1024 {
            anyhow::bail!("server.port must be >= 1024 (got {})", self.server.port);
        }

        // Retrieval weights must sum to ~1.0 (allow small floating point error)
        let sum = self.retrieval.alpha + self.retrieval.beta + self.retrieval.gamma;
        if (sum - 1.0).abs() > 0.05 {
            tracing::warn!(
                "retrieval weights (α={}, β={}, γ={}) sum to {:.2} (should be ~1.0)",
                self.retrieval.alpha, self.retrieval.beta, self.retrieval.gamma, sum
            );
        }

        Ok(())
    }
}
```

---

## 4. AppState Assembly

```rust
// src/main.rs (expanded)

pub struct AppState {
    pub graph_db:      Arc<GraphDb>,
    pub vector_db:     Arc<VectorDb>,
    pub pipeline:      Arc<RetrievalPipeline>,
    pub source_loader: Arc<SourceLoader>,
    pub watch_status:  Arc<RwLock<IndexStatus>>,
    pub coordinator:   Arc<StoreCoordinator>,
    pub config:        Arc<Config>,
}

async fn build_app_state(
    config: Arc<Config>,
    project_root: &Path,
) -> anyhow::Result<Arc<AppState>> {
    let data_dir = data_dir_for_project(project_root)?;
    std::fs::create_dir_all(&data_dir)?;

    // Open stores
    let graph_db  = Arc::new(GraphDb::open(&data_dir.join("graph.db"))?);
    let vector_db = Arc::new(VectorDb::open_or_create(
        &format!("file://{}/vectors", data_dir.display()),
        384,  // bge-small dim; overridden after model load
    ).await?);

    // Load embedding model
    let model_dir = data_dir.join("models");
    std::fs::create_dir_all(&model_dir)?;
    let model = load_model_from_config(&config.embedding, &model_dir).await?;

    // Warm up model (first inference is slow due to ONNX JIT)
    tracing::info!("warming up embedding model...");
    model.embed_one("warmup").await?;

    let embedder     = Arc::new(BatchEmbedder::new(model.clone(), config.embedding.batch_size));
    let source_loader = Arc::new(SourceLoader::new(100));  // LRU 100 files
    let coordinator  = Arc::new(StoreCoordinator::new(graph_db.clone(), vector_db.clone()));

    let pipeline = Arc::new(RetrievalPipeline {
        model:        model.clone(),
        vector_db:    vector_db.clone(),
        graph_db:     graph_db.clone(),
        source_cache: source_loader.clone(),
        config:       config.retrieval.clone(),
    });

    let watch_status = Arc::new(RwLock::new(IndexStatus {
        state:           IndexState::Stale,
        last_indexed_at: None,
        pending_count:   0,
        total_symbols:   0,
        total_files:     0,
        last_error:      None,
    }));

    Ok(Arc::new(AppState {
        graph_db, vector_db, pipeline,
        source_loader, watch_status, coordinator, config,
    }))
}
```

---

## 5. Data Directory

```rust
fn data_dir_for_project(root: &Path) -> anyhow::Result<PathBuf> {
    // Override via env var
    if let Ok(dir) = std::env::var("CODEGRAPH_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }

    // Default: ~/.codegraph/<sha256(canonicalized_root)[:8]>/
    let canonical = root.canonicalize()?;
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        canonical.hash(&mut h);
        format!("{:016x}", h.finish())
    };
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(".codegraph").join(&hash[..8]))
}
```

---

## 6. Readiness Signal & Health Endpoint

Add to the REST API router:

```rust
// crates/codegraph-api/src/routes/health.rs

pub async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    let status = state.watch_status.read().await;
    let ready  = status.state == IndexState::Fresh;
    let code   = if ready {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (code, axum::Json(serde_json::json!({
        "ready":         ready,
        "state":         status.as_str(),
        "total_symbols": status.total_symbols,
        "total_files":   status.total_files,
    })))
}
```

`GET /health` returns 200 when ready, 503 while indexing/stale.

**MCP clients should retry connection** until `/health` returns 200 before sending tool calls. Document this in README.

---

## 7. Error Propagation to MCP / REST

When `IndexState != Fresh`, tool calls and REST calls must handle it consistently:

```rust
// crates/codegraph-mcp/src/tools/mod.rs

pub fn check_index_ready(status: &IndexStatus) -> Option<JsonRpcResponse> {
    match status.state {
        IndexState::Indexing => Some(JsonRpcResponse::err(
            None, -32000, "index is still building — retry in a few seconds"
        )),
        IndexState::Error => Some(JsonRpcResponse::err(
            None, -32000, &format!(
                "index error: {}",
                status.last_error.as_deref().unwrap_or("unknown")
            )
        )),
        IndexState::Fresh | IndexState::Stale => None, // proceed; stale = allow but warn
    }
}
```

REST endpoints return `503 Service Unavailable` when indexing, `200` with `meta.index_state = "stale"` when stale (results may be incomplete but available).

---

## 8. Graceful Shutdown

```rust
// src/main.rs

async fn run_until_shutdown(
    server:  axum::serve::Serve<..>,
    watcher: Arc<IndexWorker>,
    status:  Arc<RwLock<IndexStatus>>,
) -> anyhow::Result<()> {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate()
    )?;

    tokio::select! {
        _ = ctrl_c => { tracing::info!("received SIGINT, shutting down"); }
        #[cfg(unix)]
        _ = sigterm.recv() => { tracing::info!("received SIGTERM, shutting down"); }
        result = server => { result?; }
    }

    // Allow up to 5 s for in-flight requests to finish
    // (axum GracefulShutdown handles this)
    // Allow watcher to flush its current batch (up to 1 s)
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        watcher.flush_pending(),
    ).await.ok();

    tracing::info!("shutdown complete");
    Ok(())
}
```

`IndexWorker::flush_pending()` processes any events already in the debounce window before exiting.

---

## 9. Watcher Queue Overflow

When the event queue reaches `max_queue = 1000`, instead of dropping events, log a warning and trigger a full re-index:

```rust
// crates/codegraph-watcher/src/watcher.rs

fn send_event(tx: &Sender<..>, event: Event) {
    match tx.try_send(Ok(event)) {
        Ok(_)  => {}
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            tracing::warn!(
                "watcher event queue full — will trigger full re-index after drain"
            );
            // Signal overflow via a special sentinel
            let _ = tx.try_send(Err(notify::Error::generic("queue_overflow")));
        }
        Err(_) => {}
    }
}

// In IndexWorker::run():
if batch.contains_overflow_sentinel() {
    tracing::info!("queue overflow detected — running full re-index");
    full_index(&self.config.root, &self).await?;
}
```

---

## 10. Data Retention / Cleanup

Add a `--clean` flag to the binary:

```bash
codegraph --clean    # deletes ~/.codegraph/<project-hash>/ and exits
```

No automatic cleanup — let users manage disk. Document the data directory location in `watch_status()` output and in README.

Data directory grows proportionally to codebase size:
- SQLite: ~1 KB/symbol → ~100 MB for 100k symbols
- LanceDB: ~1.5 KB/symbol (384 f32 + metadata) → ~150 MB for 100k symbols
- Tree cache: in-memory only, not persisted

---

## 11. ONNX Weights License

`bge-small-en-v1.5` is licensed under **MIT** (BAAI). Safe to use, safe to redistribute, safe for commercial use. Document in README.

`bge-code-v1`: verify license before shipping (BAAI, likely MIT but confirm).
`Voyage-code-3`: commercial API — users supply their own `VOYAGE_API_KEY`.

---

## 12. `--index-only` Mode (for CI)

Plan 01 mentions `--index-only` flag. Operationalize it:

```rust
if cli.index_only {
    full_index(&config.root, &worker).await?;
    let status = state.watch_status.read().await;
    println!(
        "Indexed {} files ({} symbols) in {:.1}s",
        status.total_files, status.total_symbols, elapsed.as_secs_f64()
    );
    return Ok(());
}
```

Useful for pre-warming the index in CI/CD pipelines before starting the server.

---

## 13. Implementation Steps

1. Implement `find_config()` with upward CWD search and Git-root fallback
2. Implement `Config::validate()` with model name, language, and port checks
3. Implement `build_app_state()` orchestrating all component initialization
4. Implement `data_dir_for_project()` with `CODEGRAPH_DATA_DIR` override
5. Add `GET /health` endpoint to REST API router
6. Implement `check_index_ready()` guard in MCP dispatcher
7. Add 503 response when `IndexState::Indexing` in REST handlers
8. Implement graceful shutdown with `tokio::signal` and `IndexWorker::flush_pending()`
9. Implement queue overflow sentinel → full re-index trigger
10. Implement `--clean` flag
11. Implement `--index-only` mode
12. Add model warm-up call after ONNX session opens
13. Document data directory and license in README

---

## 14. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Config search depth | Unlimited / max 10 parent dirs | Max 10 — prevents infinite loop edge cases |
| Stale results policy | Return stale / block / warn | Return with `meta.index_state = "stale"` — agents should check `watch_status()` |
| Shutdown timeout | 1 s / 5 s / configurable | 5 s for HTTP drain, 1 s for watcher flush |
| Data dir hash | SHA-256 / DefaultHasher / path slug | DefaultHasher (fast, good enough for a local path hash) |
| First-run progress | Rich terminal UI / plain println | Plain `println!` with percentage — simpler, works in all terminals |
