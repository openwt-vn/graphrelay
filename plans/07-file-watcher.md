# Plan 07 — File Watcher & Incremental Indexing

## Goal
Implement a file system watcher using `notify-rs` that detects source file changes and re-indexes only the changed files, keeping the code graph and vector store fresh with < 500 ms p95 latency from save to indexed.

## Success Criteria
- File save → index updated in < 500 ms p95
- No re-indexing of unchanged files
- `watch_status()` accurately reflects indexer state (fresh / indexing / stale / error)
- Debounce prevents multiple rapid saves from triggering redundant work
- Watcher gracefully handles file renames, deletions, and new file creation
- Watcher ignores configured directories (`.git`, `node_modules`, `target`)

---

## 1. State Machine

```
                       start()
                          │
                          ▼
                     ┌─────────┐
                ┌───▶│  Fresh  │◀──────────────────┐
                │    └────┬────┘                   │
                │         │ file change             │ index complete
                │         ▼                        │
                │    ┌──────────┐  debounce (50ms) │
                │    │ Pending  │──────────────────▶│ → Indexing
                │    └──────────┘
                │
                │    ┌──────────┐
                │    │  Error   │◀── index fails (logged, stays indexable)
                └────┤  (stale) │
                     └──────────┘
```

---

## 2. `IndexStatus` Type

```rust
// crates/codegraph-watcher/src/status.rs

#[derive(Clone, Debug, Serialize)]
pub struct IndexStatus {
    pub state:           IndexState,
    pub last_indexed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub pending_count:   usize,
    pub total_symbols:   u64,
    pub total_files:     u64,
    pub last_error:      Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    Fresh,
    Indexing,
    Stale,
    Error,
}

impl IndexStatus {
    pub fn age_seconds(&self) -> Option<i64> {
        self.last_indexed_at.map(|t| {
            (chrono::Utc::now() - t).num_seconds()
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self.state {
            IndexState::Fresh    => "fresh",
            IndexState::Indexing => "indexing",
            IndexState::Stale    => "stale",
            IndexState::Error    => "error",
        }
    }
}
```

---

## 3. Watcher Core

```rust
// crates/codegraph-watcher/src/watcher.rs

use notify::{RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
use crossbeam_channel::{bounded, Receiver};

pub struct FileWatcher {
    _watcher:  RecommendedWatcher,
    events:    Receiver<notify::Result<Event>>,
    root:      PathBuf,
    ignores:   Vec<glob::Pattern>,
}

impl FileWatcher {
    pub fn new(root: PathBuf, ignore_patterns: &[String]) -> anyhow::Result<Self> {
        let (tx, rx) = bounded(1000);

        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })?;

        watcher.watch(&root, RecursiveMode::Recursive)?;

        let ignores = ignore_patterns.iter()
            .filter_map(|p| glob::Pattern::new(p).ok())
            .collect();

        Ok(Self {
            _watcher: watcher,
            events:   rx,
            root,
            ignores,
        })
    }

    pub fn next_batch(&self, debounce_ms: u64) -> Vec<PathBuf> {
        use std::collections::HashSet;
        use std::time::{Duration, Instant};

        let mut changed: HashSet<PathBuf> = HashSet::new();
        let deadline = Instant::now() + Duration::from_millis(debounce_ms);

        // Collect initial event
        match self.events.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(event)) => self.process_event(event, &mut changed),
            _ => return vec![],
        }

        // Drain remaining events within debounce window
        while Instant::now() < deadline {
            match self.events.recv_deadline(deadline) {
                Ok(Ok(event)) => self.process_event(event, &mut changed),
                _ => break,
            }
        }

        changed.into_iter().collect()
    }

    fn process_event(&self, event: Event, changed: &mut std::collections::HashSet<PathBuf>) {
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                for path in event.paths {
                    if self.should_index(&path) {
                        changed.insert(path);
                    }
                }
            }
            _ => {}
        }
    }

    fn should_index(&self, path: &Path) -> bool {
        // Must be a file (not a directory)
        if path.is_dir() { return false; }

        // Must have a known extension
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None    => return false,
        };
        if !KNOWN_EXTENSIONS.contains(&ext) { return false; }

        // Must not match ignore patterns
        let rel = path.strip_prefix(&self.root).unwrap_or(path);
        let rel_str = rel.to_string_lossy();
        !self.ignores.iter().any(|p| p.matches(&rel_str))
    }
}

const KNOWN_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "java",
    "c", "h", "cpp", "cc", "cxx", "cs", "rb", "kt", "swift", "php", "scala",
];
```

---

## 4. Indexer Worker

The indexer runs in a dedicated `tokio` task, consuming batches from the watcher:

```rust
// crates/codegraph-watcher/src/queue.rs

pub struct IndexWorker {
    watcher:      Arc<FileWatcher>,
    extractor:    Arc<Extractor>,
    embedder:     Arc<BatchEmbedder>,
    coordinator:  Arc<StoreCoordinator>,
    status:       Arc<RwLock<IndexStatus>>,
    tree_cache:   DashMap<PathBuf, tree_sitter::Tree>,   // for incremental parsing
    config:       WatcherConfig,
}

impl IndexWorker {
    pub async fn run(self: Arc<Self>) {
        loop {
            // Block until a batch of changed paths arrives (with debounce)
            let changed_paths = tokio::task::spawn_blocking({
                let w = self.watcher.clone();
                let d = self.config.debounce_ms;
                move || w.next_batch(d)
            }).await.unwrap_or_default();

            if changed_paths.is_empty() { continue; }

            self.update_status(|s| {
                s.state         = IndexState::Indexing;
                s.pending_count = changed_paths.len();
            }).await;

            for path in &changed_paths {
                if let Err(e) = self.index_path(path).await {
                    tracing::error!("failed to index {:?}: {e}", path);
                    self.update_status(|s| s.last_error = Some(e.to_string())).await;
                }
            }

            self.update_status(|s| {
                s.state           = IndexState::Fresh;
                s.last_indexed_at = Some(chrono::Utc::now());
                s.pending_count   = 0;
                s.last_error      = None;
            }).await;
        }
    }

    async fn index_path(&self, path: &Path) -> anyhow::Result<()> {
        if !path.exists() {
            // File deleted
            return self.coordinator.delete_file(path).await;
        }

        let source = tokio::fs::read(path).await?;
        let checksum = xxhash_rust::xxh64::xxh64(&source, 0);

        // Skip if unchanged
        if let Some(stored) = self.coordinator.graph.file_checksum(path)? {
            if stored == checksum {
                tracing::debug!("skipping unchanged file: {:?}", path);
                return Ok(());
            }
        }

        // Parse (incremental if we have a previous tree; fallback to full parse on error)
        let extraction = if let Some(prev_tree) = self.tree_cache.get(path) {
            let change = compute_file_change(path, &source, &prev_tree);
            self.extractor.extract_incremental(&prev_tree, &change)
                .or_else(|e| {
                    tracing::warn!("incremental parse failed for {:?}: {e} — falling back to full parse", path);
                    self.extractor.extract(path, &source)
                })?
        } else {
            self.extractor.extract(path, &source)?
        };

        // Update tree cache with the newly parsed tree
        self.tree_cache.insert(path.to_owned(), extraction.tree.clone());

        // Phase 1: Write symbols + edges to SQLite FIRST so that callers/callees
        // are queryable before building embedding chunks (avoids bootstrap chicken-and-egg).
        // Note: for new files there are no callers yet from other files — that's expected;
        // chunks will improve as the repo is fully indexed.
        {
            let file_id = self.coordinator.graph.upsert_file(&extraction.file_record)?;
            self.coordinator.graph.delete_symbols_for_file(file_id)?;
            self.coordinator.graph.upsert_symbols_batch(&extraction.symbols)?;
            self.coordinator.graph.upsert_edges(&extraction.edges)?;
        }

        // Invalidate SourceLoader cache for this file so stale snippets are not served
        self.source_loader.invalidate(path);

        // Phase 2: Build graph-aware chunks (callers/callees now queryable in SQLite)
        let chunks = self.build_chunks(&extraction).await?;

        // Embed
        let embedded = self.embedder.embed_chunks(chunks).await?;

        // Phase 3: Write vectors to LanceDB (best-effort)
        if let Err(e) = self.coordinator.vector.upsert(&embedded).await {
            tracing::error!("vector upsert failed for {:?}: {e}", path);
            self.coordinator.graph.set_meta("vector_dirty", "true")?;
        }

        Ok(())
    }

    async fn build_chunks(&self, extraction: &FileExtraction) -> anyhow::Result<Vec<Chunk>> {
        let mut chunks = Vec::new();
        for symbol in &extraction.symbols {
            // callers_of returns empty vec for brand-new symbols — acceptable, chunks improve over time
            let callers = self.coordinator.graph.callers_of(symbol.id).unwrap_or_default();
            let callees = self.coordinator.graph.callees_of(symbol.id).unwrap_or_default();
            let body    = extract_body_text(symbol, &extraction.source);
            let chunk   = ChunkBuilder::build(symbol, &body, &callers, &callees, 1500);
            chunks.push(chunk);
        }
        Ok(chunks)
    }
}
```

---

## 5. Initial Full Index

On startup, before starting the watcher loop, do a full index of the workspace:

```rust
pub async fn full_index(
    root: &Path,
    worker: &Arc<IndexWorker>,
    extractor: &Arc<Extractor>,
) -> anyhow::Result<()> {
    use rayon::prelude::*;

    let files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| worker.watcher.should_index(e.path()))
        .map(|e| e.path().to_owned())
        .collect();

    tracing::info!("full index: {} files", files.len());

    // Parse all files in parallel (CPU-bound)
    let extractions: Vec<FileExtraction> = files
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read(path).ok()?;
            extractor.extract(path, &source).ok()
        })
        .collect();

    // Embed + store serially (I/O-bound + GPU-bound)
    for extraction in extractions {
        worker.index_path(&extraction.file_record.path.clone()).await?;
    }

    Ok(())
}
```

---

## 6. Watch Config

```toml
# codegraph.toml
[watcher]
debounce_ms = 50
max_queue   = 1000
ignore      = [".git/**", "node_modules/**", "target/**", "dist/**", "*.min.js", "*.lock"]
```

---

## 7. File Change Detection (xxhash64)

```toml
# Cargo.toml
[dependencies]
xxhash-rust = { version = "0.8", features = ["xxh64"] }
```

```rust
fn file_changed(path: &Path, stored_checksum: u64) -> anyhow::Result<bool> {
    let content  = std::fs::read(path)?;
    let checksum = xxhash_rust::xxh64::xxh64(&content, 0);
    Ok(checksum != stored_checksum)
}
```

This is more reliable than mtime on network filesystems and avoids false positives from touch commands.

---

## 8. Rename Handling

`notify-rs` emits a `Rename { from, to }` event on some platforms and separate `Remove + Create` on others. Handle both:

```rust
EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
    // from: old path, to: new path
    self.coordinator.delete_file(&event.paths[0]).await?;
    self.index_path(&event.paths[1]).await?;
}
EventKind::Remove(_) => {
    self.coordinator.delete_file(&event.paths[0]).await?;
}
```

---

## 9. Status Exposure

`watch_status()` MCP tool reads `Arc<RwLock<IndexStatus>>` directly — no additional I/O:

```rust
async fn update_status(&self, f: impl FnOnce(&mut IndexStatus)) {
    let mut status = self.status.write().await;
    f(&mut *status);
    // Update symbol/file counts from DB every 10 updates (avoid hot-path overhead)
    if status.pending_count == 0 {
        if let Ok(total_files) = self.coordinator.graph.total_file_count() {
            status.total_files = total_files;
        }
    }
}
```

---

## 10. Implementation Steps

1. Add `notify`, `walkdir`, `xxhash-rust`, `glob` to `codegraph-watcher/Cargo.toml`
2. Implement `IndexStatus` and `IndexState`
3. Implement `FileWatcher::new()` with notify-rs
4. Implement `FileWatcher::next_batch()` with debounce
5. Implement `should_index()` with glob ignore patterns
6. Implement `IndexWorker::run()` main loop
7. Implement `IndexWorker::index_path()` with change detection
8. Implement `full_index()` for startup
9. Wire into `main.rs`: full_index → spawn watcher task → share `Arc<RwLock<IndexStatus>>`
10. Test: create a file, verify it appears in SQLite + LanceDB within 500 ms
11. Test: modify a file, verify only that file is re-indexed (not the whole workspace)
12. Test: delete a file, verify symbols are removed from both stores
13. Benchmark: 100 rapid saves → verify < 500 ms p95 for each, no duplicates

---

## 11. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Debounce implementation | crossbeam timer / tokio::time | crossbeam `recv_deadline` — simpler, no async overhead in the tight loop |
| Full index parallelism | rayon (CPU) vs tokio (async) | rayon for parsing (CPU-bound), tokio for embedding + DB (I/O-bound) |
| Tree cache | DashMap<PathBuf, Tree> | Yes — avoids full re-parse on every save in the common case |
| Event queue bound | 1000 / unbounded | 1000 — prevents OOM on very active directories (e.g., during `git checkout`) |
| Ignore patterns | glob / regex | glob — simpler, `.gitignore`-compatible format |

### Implementation note (2026-07-31): startup indexing split, not yet parallel

The "rayon for parsing, tokio for embedding + DB" recommendation above was never
implemented — `full_index` parses files in a strict sequential loop, and profiling
this repo's actual startup cost found the sequential parse itself was *not* the
dominant cost (graph pass ≈ 9s for 80 files / 1368 symbols). Two other things were:

1. `scip.auto_run` (on by default) ran `rust-analyzer scip .` as a **blocking**
   `std::process::Command::status()` call inside the graph pass, unwrapped in
   `spawn_blocking` — ~50s on this repo, and it stalled a tokio worker thread for
   its whole duration on top of just being slow.
2. Embedding was batched per-file (a handful of chunks per call) instead of across
   files, so `BatchEmbedder` rarely filled its configured `batch_size`.

Both are fixed: `full_index` is now `full_index_graph()` (parse + SQLite only) →
`run_scip_enrichment()` + `embed_backlog()` (SCIP and cross-file-batched embedding,
run concurrently via `tokio::join!`, in a background task independent of marking
the project `Fresh`). See `crates/codegraph-watcher/src/worker.rs` and the Plan 07
checklist entry for the measured before/after numbers.

Parallel *parsing* across files is still open. It's blocked on more than just
adding `rayon::par_iter()` over the file list: `GraphDb` (`codegraph-store/src/graph_db.rs`)
wraps a single `Mutex<Connection>`, so concurrent `index_path_graph_only` calls
would immediately serialize on every SQLite write anyway. Doing this for real
needs either a connection pool (multiple readers, one writer, WAL mode already
on) or a batched/queued writer in front of the mutex — parsing itself (tree-sitter,
CPU-bound) can already run off the async runtime via `rayon`, but there's no
throughput to gain until the SQLite write path stops being the serialization point.
