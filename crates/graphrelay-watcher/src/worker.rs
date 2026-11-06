use crate::watcher::FileWatcher;
use graphrelay_core::{
    config::{ScipConfig, WatcherConfig},
    types::{Chunk, Edge, FileId, IndexState, IndexStatus, IndexedChunk, Symbol},
    ChunkBuilder,
};
use graphrelay_parser::{
    extractor::extract_body_text,
    synthesis::{c_fnptr, js_callback, py_self_calls, py_string_dispatch},
    Extractor,
};
use graphrelay_store::{ChunkCommit, StoreCoordinator};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Files are grouped into batches of this size before a `flush_chunk_batch`
/// call, so a large backlog issues one bounded FTS5 transaction at a time
/// rather than one huge one — mirrors `full_index_graph_blocking`'s
/// `FULL_INDEX_CHUNK_SIZE`. Before Plan 20 removed vector search, this same
/// grouping existed to fill real `BatchEmbedder` batch sizes for ONNX
/// efficiency; the grouping itself is still worth keeping for bounded
/// transaction size, just no longer tied to an embedder's batch size.
const CHUNK_BATCH_SIZE: usize = 32;

/// How many times the live loop re-attempts a file whose graph write or chunk
/// commit failed before leaving it pending for the next change/re-index.
const MAX_RETRIES: u32 = 3;

pub struct IndexWorker {
    pub watcher:      Arc<FileWatcher>,
    pub extractor:    Arc<Extractor>,
    pub coordinator:  Arc<StoreCoordinator>,
    pub status:       Arc<RwLock<IndexStatus>>,
    pub config:       WatcherConfig,
    pub source_loader: Arc<graphrelay_core::retrieval::SourceLoader>,
    pub scip_config:  ScipConfig,
    full_index_active: Arc<AtomicBool>,
}

/// Held for the full lifetime of one `full_index_graph` → `run_scip_enrichment`
/// → `index_chunk_backlog` sequence (across the split, backgrounded call sites
/// in `src/server.rs`, not just within a single function call). Prevents two
/// full indexes — e.g. startup plus a watcher-overflow recovery — from running
/// concurrently over the same files, which would widen the stale-write race
/// window `flush_chunk_batch`'s checksum re-checks are built to narrow.
pub struct FullIndexGuard(Arc<AtomicBool>);

impl Drop for FullIndexGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Result of building one pending file's chunks.
enum ChunkBuild {
    Built(Vec<Chunk>),
    /// The file changed or was deleted after its graph write; a newer event
    /// re-indexes it, so this is dropped silently (not a failure).
    Superseded,
}

/// A file whose graph data (symbols, edges) is already committed to SQLite but
/// whose chunks have not yet been written to the lexical (FTS5) index.
#[derive(Debug, Clone)]
pub struct PendingChunk {
    pub path:     PathBuf,
    pub file_id:  FileId,
    pub checksum: u64,
    /// Identifies the graph write this chunk work belongs to; the commit step
    /// only accepts chunks built from the file's *current* token (Plan 24 2.3).
    pub token:    u64,
}

impl IndexWorker {
    /// Resolved location of the SCIP index: `[scip] scip_path` (default
    /// `.graphrelay/index.scip`), relative paths taken from the project root.
    /// Previously hardcoded, which made the config option a dead setting.
    pub fn scip_file(&self, root: &Path) -> PathBuf {
        if self.scip_config.scip_path.is_absolute() {
            self.scip_config.scip_path.clone()
        } else {
            root.join(&self.scip_config.scip_path)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        watcher:      Arc<FileWatcher>,
        extractor:    Arc<Extractor>,
        coordinator:  Arc<StoreCoordinator>,
        status:       Arc<RwLock<IndexStatus>>,
        source_loader: Arc<graphrelay_core::retrieval::SourceLoader>,
        config:       WatcherConfig,
        scip_config:  ScipConfig,
    ) -> Self {
        Self {
            watcher, extractor, coordinator,
            status, config, source_loader, scip_config,
            full_index_active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Attempt to start a full index. Returns `None` if one is already in
    /// progress (see `FullIndexGuard`); the caller should skip its run rather
    /// than race the in-flight one.
    pub fn try_begin_full_index(&self) -> Option<FullIndexGuard> {
        if self.full_index_active.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(FullIndexGuard(self.full_index_active.clone()))
        }
    }

    pub async fn run(self: Arc<Self>, shutdown: Arc<tokio::sync::Notify>) {
        let mut retry: HashMap<PathBuf, u32> = HashMap::new();
        loop {
            let mut changed_paths = {
                let w = self.watcher.clone();
                let d = self.config.debounce_ms;
                tokio::select! {
                    result = tokio::task::spawn_blocking(move || w.next_batch(d)) => {
                        result.unwrap_or_default()
                    }
                    _ = shutdown.notified() => {
                        self.flush_pending().await;
                        return;
                    }
                }
            };

            // Paths whose last attempt failed are retried (bounded) alongside
            // whatever the watcher just delivered, instead of waiting for the
            // next edit or a restart.
            for path in retry.keys() {
                if !changed_paths.contains(path) {
                    changed_paths.push(path.clone());
                }
            }
            let prior_attempts = std::mem::take(&mut retry);

            if !changed_paths.is_empty() {
                {
                    let mut status = self.status.write().await;
                    status.state         = IndexState::Indexing;
                    status.pending_count = changed_paths.len();
                }

                // Graph-only writes for every path in this debounced batch first,
                // collecting the chunk backlog, then ONE `flush_chunk_batch` call
                // for the whole batch below — mirrors `full_index_graph`'s split
                // from the per-file `index_path` used elsewhere. A burst of rapid
                // saves previously did one FTS5 write per file sequentially;
                // batching amortizes the per-call overhead the same way the
                // full-index path already does.
                let mut backlog = Vec::with_capacity(changed_paths.len());
                let mut failed_paths: Vec<PathBuf> = Vec::new();
                let mut stale_error: Option<String> = None;
                let mut soft_error: Option<String> = None;
                for path in &changed_paths {
                    match self.index_path_graph_only(path).await {
                        Ok(Some(pending)) => backlog.push(pending),
                        Ok(None) => {}
                        Err(e) => {
                            tracing::error!("failed to index {:?}: {e}", path);
                            stale_error = Some(e.to_string());
                            failed_paths.push(path.clone());
                        }
                    }
                }

                // Re-resolve cross-file edges after the batch's graph writes but
                // before building chunks below, so callers/callees for symbols in
                // this batch are complete by the time `flush_chunk_batch` builds
                // their chunk context — same ordering `full_index_graph` uses.
                // Scoped to this batch's files: the full pass costs ~0.7 s on a
                // large repo and blocks every query while it holds the connection.
                let file_ids: Vec<FileId> = backlog.iter().map(|p| p.file_id).collect();
                let graph = self.coordinator.graph.clone();
                match tokio::task::spawn_blocking(move || graph.resolve_edges_for_files(&file_ids)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        tracing::error!("edge resolution failed: {e}");
                        soft_error = Some(format!("edge resolution failed: {e}"));
                    }
                    Err(e) => {
                        tracing::error!("edge resolution task panicked: {e}");
                        soft_error = Some(format!("edge resolution task panicked: {e}"));
                    }
                }

                let batch_paths: Vec<PathBuf> = backlog.iter().map(|p| p.path.clone()).collect();
                match self.flush_chunk_batch(&mut backlog).await {
                    Ok(unbuilt) => {
                        if !unbuilt.is_empty() {
                            stale_error = Some(format!("chunk build failed for {} file(s)", unbuilt.len()));
                            failed_paths.extend(unbuilt);
                        }
                    }
                    Err(e) => {
                        tracing::error!("batch chunk indexing failed for {} file(s): {e}", changed_paths.len());
                        stale_error = Some(e.to_string());
                        failed_paths.extend(batch_paths);
                    }
                }

                for path in failed_paths {
                    let attempts = prior_attempts.get(&path).copied().unwrap_or(0) + 1;
                    if attempts <= MAX_RETRIES {
                        retry.insert(path, attempts);
                    } else {
                        tracing::error!("giving up on {:?} after {MAX_RETRIES} retries; it stays pending until its next change or a re-index", path);
                    }
                }

                {
                    let total_files   = self.coordinator.graph.total_file_count().unwrap_or(0);
                    let total_symbols = self.coordinator.graph.total_symbol_count().unwrap_or(0);
                    let mut status = self.status.write().await;
                    status.pending_count   = 0;
                    status.total_files     = total_files;
                    status.total_symbols   = total_symbols;
                    if let Some(err) = stale_error {
                        // A failed graph/FTS write means part of the index is behind
                        // the disk. Never report Fresh over that (the old code
                        // cleared its own error a few lines later, so the failure
                        // was invisible).
                        status.state      = IndexState::Stale;
                        status.last_error = Some(err);
                    } else {
                        status.state           = IndexState::Fresh;
                        status.last_indexed_at = Some(chrono::Utc::now());
                        // Only clear an error this loop itself set. A background
                        // `index_chunk_backlog`/`run_scip_enrichment` task also
                        // writes `last_error` (prefixed "semantic indexing:"/
                        // "SCIP..."); an unrelated file save landing here must not
                        // erase that signal while lexical indexing catches up.
                        if let Some(err) = soft_error {
                            status.last_error = Some(err);
                        } else if !matches!(&status.last_error, Some(e) if e.starts_with("semantic indexing:")) {
                            status.last_error = None;
                        }
                    }
                }
            }

            // Recover from event queue overflow with a full re-index. If a full
            // index is already running (startup), don't lose the signal:
            // `full_index` would return early and `has_overflow` has already
            // cleared the flag, so the dropped events would never be recovered.
            if self.watcher.has_overflow() {
                if self.full_index_active.load(Ordering::SeqCst) {
                    self.watcher.mark_overflow();
                } else {
                    let root = self.watcher.root().to_owned();
                    tracing::warn!("watcher queue overflow — triggering full re-index of {:?}", root);
                    if let Err(e) = full_index(&root, &self).await {
                        tracing::error!("overflow re-index failed: {e}");
                    }
                }
            }
        }
    }

    async fn flush_pending(&self) {
        let paths = self.watcher.drain_all();
        if paths.is_empty() {
            return;
        }
        tracing::info!("flush_pending: {} file(s) before shutdown", paths.len());
        let mut backlog = Vec::with_capacity(paths.len());
        for path in &paths {
            match self.index_path_graph_only(path).await {
                Ok(Some(pending)) => backlog.push(pending),
                Ok(None) => {}
                Err(e) => tracing::warn!("flush_pending {:?}: {e}", path),
            }
        }
        // Resolve cross-file edges before building chunks below, same
        // ordering `run()`'s live loop uses, so chunk context for files in
        // this drain is complete rather than reflecting pre-drain edges.
        let file_ids: Vec<FileId> = backlog.iter().map(|p| p.file_id).collect();
        let graph = self.coordinator.graph.clone();
        match tokio::task::spawn_blocking(move || graph.resolve_edges_for_files(&file_ids)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!("flush_pending: edge resolution failed: {e}"),
            Err(e) => tracing::warn!("flush_pending: edge resolution task panicked: {e}"),
        }
        if let Err(e) = self.flush_chunk_batch(&mut backlog).await {
            tracing::warn!("flush_pending: batch chunk indexing failed: {e}");
        }
    }

    /// Full pipeline for one file: graph write followed immediately by chunk
    /// indexing. Kept for callers that want a single-file, no-batching
    /// pipeline; the live watcher loop and shutdown flush now batch
    /// `flush_chunk_batch` across a whole debounced/drained set of paths
    /// instead of calling this per file — see `run()` and `flush_pending()`.
    /// `full_index` instead splits these two phases so the
    /// graph becomes queryable long before the lexical index of the whole
    /// workspace finishes — see `full_index_graph` / `index_chunk_backlog`.
    pub async fn index_path(&self, path: &Path) -> anyhow::Result<()> {
        match self.index_path_graph_only(path).await? {
            Some(pending) => {
                let mut buf = vec![pending];
                self.flush_chunk_batch(&mut buf).await.map(|_| ())
            }
            None => Ok(()),
        }
    }

    /// Phase 1: parse + write symbols/edges to SQLite. Fast — no chunk build,
    /// no FTS5 write. Returns `None` if the file was deleted or is unchanged
    /// since the last index (checksum match).
    async fn index_path_graph_only(&self, path: &Path) -> anyhow::Result<Option<PendingChunk>> {
        if !path.exists() {
            let removed = tokio::task::block_in_place(|| -> anyhow::Result<Vec<PathBuf>> {
                if self.coordinator.delete_file(path)? {
                    return Ok(vec![path.to_owned()]);
                }
                // No file row at this exact path: a removed/renamed directory
                // is reported by the OS as the directory alone.
                self.coordinator.graph.delete_files_under(path)
            })?;
            self.extractor.invalidate_incremental_cache(path);
            for f in &removed {
                self.extractor.invalidate_incremental_cache(f);
                self.source_loader.invalidate(f);
            }
            return Ok(None);
        }

        let source = tokio::fs::read(path).await?;
        let checksum = xxhash_rust::xxh64::xxh64(&source, 0);

        // Skip if unchanged
        let stored = tokio::task::block_in_place(|| {
            self.coordinator.graph.file_checksum(path)
        })?;
        if stored == Some(checksum) {
            tracing::debug!("skipping unchanged file: {:?}", path);
            // Bump indexed_at even though content is unchanged — otherwise a
            // disk mtime change with no content change (git checkout/rebase/
            // stash, a fresh clone) leaves this file permanently flagged
            // possibly_stale (mtime > indexed_at never resolves without a
            // real re-index). See `touch_files_indexed_at`'s doc comment.
            let now = chrono::Utc::now().to_rfc3339();
            tokio::task::block_in_place(|| {
                self.coordinator.graph.touch_files_indexed_at(std::slice::from_ref(&path.to_owned()), &now)
            })?;
            return Ok(None);
        }

        // Incremental: diffs against this path's last-parsed source (cached
        // inside `Extractor`) and hands tree-sitter a real edited tree
        // instead of a from-scratch parse — this is the live single-file
        // save path, exactly the case a typical small edit to an otherwise
        // large/unchanged file benefits from. The full-index path still
        // uses `extract_many` (a fresh parser per file, no shared cache —
        // parallelism there already dominates any per-file incremental win).
        let extraction = tokio::task::block_in_place(|| {
            self.extractor.extract_incremental(path, &source)
        })?;

        // Invalidate source cache
        self.source_loader.invalidate(path);

        // Write to SQLite. Use checksum=0 as a "pending" sentinel so that if
        // chunk indexing fails or is never reached, the stored checksum won't
        // match the real one and the file will be retried. The real checksum
        // is committed only after the FTS5 write succeeds (see `flush_chunk_batch`).
        let pending_record = graphrelay_core::types::FileRecord {
            checksum: 0,
            ..extraction.file_record.clone()
        };
        // One transaction that keeps surviving symbols (Plan 24 2.2): the old
        // four separate writes deleted every symbol on each save, nulling other
        // files' edges and dropping SCIP data, and could interleave with a
        // concurrent full-index batch.
        let file_id = extraction.file_record.id;
        let token = tokio::task::block_in_place(|| {
            self.coordinator.graph.replace_file_graph(&pending_record, &extraction.symbols, &extraction.edges)
        })?;

        Ok(Some(PendingChunk { path: path.to_owned(), file_id, checksum, token }))
    }

    /// Build graph-aware chunks with class context (Improvement #3) for a file
    /// whose graph data is already in SQLite. Re-reads the file from disk and
    /// queries symbols back out rather than holding the original parse result
    /// in memory, so a large backlog of pending files costs disk cache hits
    /// instead of RAM.
    async fn build_chunks_for_pending(&self, pending: &PendingChunk) -> anyhow::Result<ChunkBuild> {
        let source = match tokio::fs::read(&pending.path).await {
            Ok(s) => s,
            // Deleted since its graph write: its delete event owns the cleanup.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ChunkBuild::Superseded),
            Err(e) => return Err(e.into()),
        };
        // Symbol byte spans in SQLite belong to the version this write indexed;
        // if the file changed since, slicing today's bytes with yesterday's spans
        // garbles chunk text. That is not a failure: the change that made it differ
        // is its own re-index (and its own pending token), so drop this entry the
        // same way a superseded token is dropped, without flagging the index stale.
        if xxhash_rust::xxh64::xxh64(&source, 0) != pending.checksum {
            return Ok(ChunkBuild::Superseded);
        }
        tokio::task::block_in_place(|| -> anyhow::Result<ChunkBuild> {
            let file_symbols = self.coordinator.graph.symbols_in_file(pending.file_id)?;
            // `?`, not a silent empty context: a chunk committed without its
            // neighbor names would keep the real checksum and never be rebuilt.
            let neighbor_names = self.coordinator.graph.call_neighbor_names_for_file(pending.file_id, 5)?;
            let no_names = (Vec::new(), Vec::new());
            let mut chunks = Vec::with_capacity(file_symbols.len());
            for symbol in &file_symbols {
                let (callers, callees) = neighbor_names.get(&symbol.id).unwrap_or(&no_names);
                let body = extract_body_text(symbol, &source);

                // Find enclosing class + sibling methods for richer chunk context
                let (class_name, siblings) = graphrelay_core::chunk::find_class_context(
                    symbol, &file_symbols,
                );
                let sibling_refs: Vec<&str> = siblings.to_vec();
                let chunk = ChunkBuilder::build_with_class(
                    symbol, &body, callers, callees, 1500,
                    class_name, &sibling_refs,
                );
                chunks.push(chunk);
            }
            Ok(ChunkBuild::Built(chunks))
        })
    }

    /// Phases 2-4 for a batch of files: build chunks for each, then hand them to
    /// `GraphDb::commit_chunk_batch`, which in ONE transaction re-checks each
    /// file's pending token, writes its FTS5 rows and commits its real checksum.
    /// Drains `buf`. Returns the paths that are still pending because their chunk
    /// build failed. A file that changed or vanished since its graph write is
    /// re-indexed once from disk right here: assuming "a newer event owns it" was
    /// wrong when the stale write came from the full index, whose read can predate
    /// a live save that was already processed, leaving old symbols (or a deleted
    /// file) in the index under a Fresh status. A failed commit rolls back checksums
    /// with the FTS write, so those files stay pending and retry.
    /// One retry for the superseded entries of a flush, done as a batch: graph
    /// writes per file, then ONE `resolve_edges_for_files` for all of them, then
    /// chunk builds. A file deleted since is removed by `index_path_graph_only`;
    /// one already re-indexed by a live event hits the checksum skip. If one
    /// changes yet again mid-retry, its own event follows. Returns the commits
    /// built and the paths that failed and must stay pending.
    async fn reindex_superseded_batch(&self, paths: &[PathBuf]) -> (Vec<ChunkCommit>, Vec<PathBuf>) {
        let mut failed = Vec::new();
        let mut pendings = Vec::new();
        for path in paths {
            match self.index_path_graph_only(path).await {
                Ok(Some(pending)) => pendings.push(pending),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("failed to re-index superseded {:?}: {e}", path);
                    failed.push(path.clone());
                }
            }
        }
        if pendings.is_empty() {
            return (Vec::new(), failed);
        }
        let ids: Vec<_> = pendings.iter().map(|p| p.file_id).collect();
        let graph = self.coordinator.graph.clone();
        let resolved = tokio::task::spawn_blocking(move || graph.resolve_edges_for_files(&ids)).await;
        if !matches!(resolved, Ok(Ok(_))) {
            tracing::warn!("failed to resolve edges for {} superseded file(s)", pendings.len());
            failed.extend(pendings.into_iter().map(|p| p.path));
            return (Vec::new(), failed);
        }
        let mut commits = Vec::new();
        for pending in pendings {
            match self.build_chunks_for_pending(&pending).await {
                Ok(ChunkBuild::Built(chunks)) => commits.push(ChunkCommit {
                    path:     pending.path,
                    token:    pending.token,
                    checksum: pending.checksum,
                    chunks:   chunks.into_iter().map(to_indexed_chunk).collect(),
                }),
                Ok(ChunkBuild::Superseded) => {}
                Err(e) => {
                    tracing::warn!("failed to build chunks for superseded {:?}: {e}", pending.path);
                    failed.push(pending.path);
                }
            }
        }
        (commits, failed)
    }

    async fn flush_chunk_batch(&self, buf: &mut Vec<PendingChunk>) -> anyhow::Result<Vec<PathBuf>> {
        if buf.is_empty() {
            return Ok(Vec::new());
        }
        let items = std::mem::take(buf);
        let mut still_pending = Vec::new();
        let mut commits = Vec::with_capacity(items.len());
        let mut superseded: Vec<PathBuf> = Vec::new();
        for item in items {
            match self.build_chunks_for_pending(&item).await {
                Ok(ChunkBuild::Built(chunks)) => commits.push(ChunkCommit {
                    path:     item.path,
                    token:    item.token,
                    checksum: item.checksum,
                    chunks:   chunks.into_iter().map(to_indexed_chunk).collect(),
                }),
                Ok(ChunkBuild::Superseded) => {
                    tracing::debug!("{:?} changed or vanished since its graph write — re-indexing it from disk", item.path);
                    superseded.push(item.path);
                }
                Err(e) => {
                    tracing::warn!("failed to build chunks for {:?}: {e}", item.path);
                    still_pending.push(item.path);
                }
            }
        }
        if !superseded.is_empty() {
            let (retried, failed) = self.reindex_superseded_batch(&superseded).await;
            commits.extend(retried);
            still_pending.extend(failed);
        }
        if commits.is_empty() {
            return Ok(still_pending);
        }
        let n = commits.len();
        let committed = tokio::task::block_in_place(|| self.coordinator.graph.commit_chunk_batch(&commits))
            .inspect_err(|e| tracing::error!("lexical FTS commit failed for batch of {n} file(s): {e}"))?;
        if committed < n {
            tracing::debug!("{} file(s) were re-indexed concurrently — dropped stale batch entries", n - committed);
        }
        Ok(still_pending)
    }
}

/// Converts a graph-aware text `Chunk` into the record shape
/// `GraphDb::upsert_fts_batch` writes. `start_line`/`language` are left at
/// their defaults — this mirrors what the pre-Plan-20 `BatchEmbedder::embed_chunks`
/// conversion did too (they were never populated on that path either); nothing
/// downstream reads them yet.
fn to_indexed_chunk(chunk: Chunk) -> IndexedChunk {
    IndexedChunk {
        symbol_id:      chunk.symbol_id,
        qualified_name: chunk.graph_meta.qualified_name,
        chunk_text:     chunk.text,
        file_path:      chunk.graph_meta.file_path.to_string_lossy().to_string(),
        start_line:     0,
        language:       String::new(),
    }
}

/// Fast pass of a full workspace index: parse every file and write graph data
/// (symbols, edges) to SQLite, but do not build/index chunks. Returns the
/// files still needing lexical indexing, which callers hand to
/// `index_chunk_backlog` — either inline (via `full_index`) or as a genuinely
/// separate background task, so the graph is queryable long before the
/// lexical index is complete.
///
/// Parsing is parallelized across a rayon thread pool (`Extractor::extract_many`)
/// since it is pure CPU work with no shared mutable state — symbol IDs are a
/// pure function of path+name, so concurrent extraction of different files
/// can't race. SQLite writes remain strictly ordered and are batched into
/// transactions of up to 200 files each (`GraphDb::upsert_batch`), preserving
/// the "SQLite upsert before chunk build" invariant: this whole pass still
/// completes, and cross-file edges are still resolved, before any chunk
/// building begins.
pub async fn full_index_graph(
    root: &Path,
    worker: &Arc<IndexWorker>,
) -> anyhow::Result<Vec<PendingChunk>> {
    // Prune ignored directories instead of walking all of target/, node_modules/
    // and .git/ only to drop each file afterwards.
    let files: Vec<std::path::PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !(e.depth() > 0 && e.file_type().is_dir() && worker.watcher.is_ignored(e.path())))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| worker.watcher.should_index(e.path()))
        .map(|e| e.path().to_owned())
        .collect();

    let total = files.len();
    tracing::info!("full index: {} files found", total);

    let worker = worker.clone();
    tokio::task::spawn_blocking(move || full_index_graph_blocking(&files, &worker)).await?
}

/// Synchronous body of `full_index_graph`. Runs entirely off the tokio
/// runtime (the caller wraps this in `spawn_blocking`): checksum-filters
/// files against one bulk `all_file_checksums()` read, parses the changed
/// ones in parallel via `extract_many` (fresh tree-sitter parser per call, so
/// concurrent threads parsing the same language never contend on a shared
/// `Mutex<Parser>`), then writes graph data to SQLite in chunked batched
/// transactions and resolves cross-file edges once at the end.
const FULL_INDEX_CHUNK_SIZE: usize = 200;

fn full_index_graph_blocking(
    files: &[std::path::PathBuf],
    worker: &Arc<IndexWorker>,
) -> anyhow::Result<Vec<PendingChunk>> {
    let existing_checksums = worker.coordinator.graph.all_file_checksums()?;

    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let pool_size = crate::memory_budget::resolve_pool_size(worker.config.memory_budget_mb, cores);
    tracing::info!(
        "full index: {} files found — parsing in {FULL_INDEX_CHUNK_SIZE}-file chunks \
         (parse pool size {pool_size} of {cores} cores)",
        files.len(),
    );
    let pool = match rayon::ThreadPoolBuilder::new().num_threads(pool_size).build() {
        Ok(pool) => Some(pool),
        Err(e) => {
            // Fall back to the global rayon pool rather than failing the
            // whole index over a scoped-pool construction error.
            tracing::warn!("failed to build a {pool_size}-thread parse pool ({e}), using the default pool");
            None
        }
    };

    // Process one FULL_INDEX_CHUNK_SIZE-file slice of the walk at a time —
    // read, parse, and write each chunk before moving to the next — rather
    // than reading every changed file's bytes and holding every extraction
    // (source bytes included, per `FileExtraction`) for the whole corpus in
    // memory simultaneously. This is what actually bounds peak memory on a
    // large repo; `memory_budget`'s pool sizing alone only bounds concurrent
    // *parse* memory, not how many chunks' worth of already-parsed data sit
    // resident while waiting to be written.
    let mut backlog = Vec::new();
    let mut indexed = 0u64;
    // Cumulative wall time per phase, logged once at the end so a slow index can be
    // attributed without a profiler.
    let (mut t_read, mut t_extract, mut t_write) =
        (std::time::Duration::ZERO, std::time::Duration::ZERO, std::time::Duration::ZERO);
    for path_chunk in files.chunks(FULL_INDEX_CHUNK_SIZE) {
        let phase = std::time::Instant::now();
        // Serial reads here dominated wall time on large repos; same bounded pool as the parse below.
        // Distinguishes "unchanged" from "errored" so an unchanged file's
        // indexed_at can be bumped (see touch_files_indexed_at's doc comment)
        // without also stamping a file that failed to read.
        enum ReadOutcome {
            Changed(std::path::PathBuf, Vec<u8>),
            Unchanged(std::path::PathBuf),
            Errored,
        }
        let read_chunk = || -> Vec<ReadOutcome> {
            path_chunk
                .par_iter()
                .map(|path| {
                    let source = match std::fs::read(path) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("skipping {:?}: {e}", path);
                            return ReadOutcome::Errored;
                        }
                    };
                    let checksum = xxhash_rust::xxh64::xxh64(&source, 0);
                    if existing_checksums.get(path.as_path()) == Some(&checksum) {
                        return ReadOutcome::Unchanged(path.clone());
                    }
                    ReadOutcome::Changed(path.clone(), source)
                })
                .collect()
        };
        let read_results = match &pool {
            Some(pool) => pool.install(read_chunk),
            None => read_chunk(),
        };
        t_read += phase.elapsed();
        let mut changed: Vec<(std::path::PathBuf, Vec<u8>)> = Vec::new();
        let mut unchanged: Vec<std::path::PathBuf> = Vec::new();
        for outcome in read_results {
            match outcome {
                ReadOutcome::Changed(path, source) => changed.push((path, source)),
                ReadOutcome::Unchanged(path)       => unchanged.push(path),
                ReadOutcome::Errored               => {}
            }
        }
        if !unchanged.is_empty() {
            let now = chrono::Utc::now().to_rfc3339();
            worker.coordinator.graph.touch_files_indexed_at(&unchanged, &now)?;
        }

        let phase = std::time::Instant::now();
        let extractions = match &pool {
            Some(pool) => pool.install(|| worker.extractor.extract_many(&changed)),
            None => worker.extractor.extract_many(&changed),
        };
        t_extract += phase.elapsed();

        let mut write_batch = Vec::with_capacity(extractions.len());
        let mut chunk_meta = Vec::with_capacity(extractions.len());
        for (path, result) in &extractions {
            match result {
                Ok(extraction) => {
                    worker.source_loader.invalidate(path);
                    let real_checksum = extraction.file_record.checksum;
                    let pending_record = graphrelay_core::types::FileRecord {
                        checksum: 0,
                        ..extraction.file_record.clone()
                    };
                    chunk_meta.push((path.clone(), extraction.file_record.id, real_checksum));
                    write_batch.push((
                        pending_record,
                        extraction.symbols.clone(),
                        extraction.edges.clone(),
                    ));
                }
                Err(e) => tracing::warn!("skipping {:?}: {e}", path),
            }
        }
        let phase = std::time::Instant::now();
        if !write_batch.is_empty() {
            let tokens = worker.coordinator.graph.upsert_batch(&write_batch)?;
            for ((path, file_id, checksum), token) in chunk_meta.into_iter().zip(tokens) {
                backlog.push(PendingChunk { path, file_id, checksum, token });
            }
        }
        t_write += phase.elapsed();
        indexed += path_chunk.len() as u64;
        // `blocking_write` is safe here (and required, not `.await`) since this
        // whole function runs inside a `spawn_blocking` closure, not the async
        // runtime — see the module doc on `full_index_graph`.
        let mut s = worker.status.blocking_write();
        s.total_files = indexed;
    }

    let t_after_loop = std::time::Instant::now();
    // Prune files that vanished while nothing was watching (git checkout to a
    // branch without a directory, daemon down during a delete) or that a config
    // change now excludes. Guarded on the file being genuinely gone or excluded,
    // not merely absent from this walk — `walkdir` silently skips unreadable
    // directories, and those files must not be pruned.
    let walked: HashSet<&PathBuf> = files.iter().collect();
    let mut pruned = 0usize;
    for path in existing_checksums.keys() {
        if walked.contains(path) {
            continue;
        }
        if !path.exists() || !worker.watcher.should_index(path) {
            worker.coordinator.delete_file(path)?;
            worker.extractor.invalidate_incremental_cache(path);
            worker.source_loader.invalidate(path);
            pruned += 1;
        }
    }
    if pruned > 0 {
        tracing::info!("full index: pruned {pruned} file(s) no longer present or indexable");
    }

    let t_prune = t_after_loop.elapsed();
    let resolve_started = std::time::Instant::now();
    let n = worker.coordinator.graph.resolve_edges()?;
    let t_resolve = resolve_started.elapsed();
    tracing::info!("resolved {n} cross-file edges");

    let synth_started = std::time::Instant::now();
    if worker.config.synthesis_enabled {
        if let Err(e) = run_dispatch_synthesis(worker) {
            tracing::warn!("dynamic-dispatch synthesis failed: {e}");
        }
    }
    tracing::info!(
        "full index graph pass phases: read {:.1}s, parse {:.1}s, sqlite write {:.1}s, prune {:.1}s, \
         resolve_edges {:.1}s, synthesis {:.1}s",
        t_read.as_secs_f64(), t_extract.as_secs_f64(), t_write.as_secs_f64(),
        t_prune.as_secs_f64(), t_resolve.as_secs_f64(), synth_started.elapsed().as_secs_f64(),
    );

    Ok(backlog)
}

/// Additive post-pass: recover dynamic-dispatch call edges (currently: C/C++
/// function-pointer dispatch tables — see `graphrelay_parser::synthesis`) the
/// 3-stage resolver structurally cannot see, since they have no literal
/// call-site expression. Operates over the corpus's full set of C/C++
/// symbols (not just this run's changed files) so it also sees dispatch
/// tables and handlers that lived in previously-indexed, unchanged files —
/// re-reads each referenced file's current on-disk text rather than trusting
/// a resolve-time snapshot. Never touches Syntax/Semantic edges; only adds
/// new ones tagged `EdgeSource::Synthesized`.
fn run_dispatch_synthesis(worker: &Arc<IndexWorker>) -> anyhow::Result<()> {
    // Always clear previously-synthesized edges before recomputing: `edges`
    // has no unique constraint beyond its own row id, so `upsert_edges`'s
    // `INSERT OR REPLACE` never actually replaces a matching edge — without
    // this, every full-index run (including every server restart, since
    // `full_index_graph` runs on normal startup) would append duplicates of
    // the same edges instead of refreshing them, permanently inflating
    // in-degree (and therefore rerank score) for repeatedly-detected
    // dispatch targets. This also correctly drops edges whose underlying
    // pattern no longer matches (e.g. a handler assignment was removed) —
    // cascade-delete on symbol removal alone wouldn't catch that case.
    let deleted = worker.coordinator.graph.delete_synthesized_edges()?;
    if deleted > 0 {
        tracing::debug!("dynamic-dispatch synthesis: cleared {deleted} stale edge(s)");
    }

    // c_fnptr needs C/C++ symbols; js_callback needs JS/TS symbols (which
    // also covers Vue/Svelte SFC-delegated code — `try_sfc_delegate` tags
    // those symbols with the embedded language, TypeScript/JavaScript, not
    // Vue/Svelte, specifically so this fetch picks them up with no changes).
    let mut symbols = worker.coordinator.graph.symbols_by_language("c")?;
    symbols.extend(worker.coordinator.graph.symbols_by_language("cpp")?);
    symbols.extend(worker.coordinator.graph.symbols_by_language("javascript")?);
    symbols.extend(worker.coordinator.graph.symbols_by_language("typescript")?);
    symbols.extend(worker.coordinator.graph.symbols_by_language("python")?);
    if symbols.is_empty() {
        return Ok(());
    }

    synthesize_dispatch_edges(&symbols, worker)
}

/// Bounded-memory driver for `graphrelay_parser::synthesis`'s two-phase API.
/// Reading every referenced file's full text into one `HashMap` up front (the
/// original shape of this pass, and still what `synthesis::synthesize_all`
/// does for small/synthetic inputs) was found — via a real `apple/swift`
/// full-index run (see `bench-reports/large-repo-indexing-2026-08-04.md`) — to
/// reach double-digit gigabytes of resident memory on a real ~5k-file C++
/// corpus, some of whose individual files run several hundred KB. Source
/// text is read in bounded `SYNTHESIS_CHUNK_SIZE`-file batches instead,
/// discarded after each phase's
/// pass over that batch — at the cost of reading every file twice (once per
/// phase) rather than once, which is cheap relative to the memory it bounds.
const SYNTHESIS_CHUNK_SIZE: usize = 200;

fn synthesize_dispatch_edges(symbols: &[Symbol], worker: &Arc<IndexWorker>) -> anyhow::Result<()> {
    let mut files_in_order: Vec<PathBuf> = Vec::new();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();
    for sym in symbols {
        if seen_files.insert(sym.file_path.clone()) {
            files_in_order.push(sym.file_path.clone());
        }
    }

    // Phase 1: build the corpus-wide handler maps. A handler registered in
    // one chunk can be the target of a call/emission site scanned from a
    // different chunk, so every chunk's phase 1 must complete before any
    // chunk's phase 2 begins.
    let mut field_handlers: HashMap<String, HashSet<graphrelay_core::types::SymbolId>> = HashMap::new();
    let mut event_handlers: HashMap<String, HashSet<graphrelay_core::types::SymbolId>> = HashMap::new();
    {
        let c_functions_by_name = c_fnptr::c_functions_by_name(symbols);
        let js_functions_by_name = js_callback::js_functions_by_name(symbols);
        for chunk in files_in_order.chunks(SYNTHESIS_CHUNK_SIZE) {
            let sources = read_chunk_sources(chunk);
            for (field, handlers) in c_fnptr::build_field_handlers(&c_functions_by_name, &sources) {
                field_handlers.entry(field).or_default().extend(handlers);
            }
            for (event, handlers) in js_callback::build_event_handlers(&js_functions_by_name, &sources) {
                event_handlers.entry(event).or_default().extend(handlers);
            }
        }
    }
    let py_targets = py_string_dispatch::py_targets(symbols);
    let py_classes = py_self_calls::class_index(symbols);
    let mut py_parents: HashMap<graphrelay_core::types::SymbolId, Vec<graphrelay_core::types::SymbolId>> = HashMap::new();
    for (child, parent) in worker.coordinator.graph.resolved_edge_pairs("Inherits", "python")? {
        py_parents.entry(child).or_default().push(parent);
    }
    let py_existing_calls: HashSet<_> =
        worker.coordinator.graph.resolved_edge_pairs("Calls", "python")?.into_iter().collect();
    if field_handlers.is_empty() && event_handlers.is_empty() && !py_targets.has_targets() && py_parents.is_empty() {
        return Ok(());
    }

    // Group symbols by file once so phase 2 can scan each chunk against just
    // that chunk's own symbols, matching the chunk's own source-text batch.
    let mut symbols_by_file: HashMap<&PathBuf, Vec<&Symbol>> = HashMap::new();
    for sym in symbols {
        symbols_by_file.entry(&sym.file_path).or_default().push(sym);
    }

    // Phase 2: call/emission sites, against the now-complete handler maps.
    let mut synthesized: Vec<Edge> = Vec::new();
    for chunk in files_in_order.chunks(SYNTHESIS_CHUNK_SIZE) {
        let sources = read_chunk_sources(chunk);
        let chunk_symbols: Vec<Symbol> = chunk
            .iter()
            .filter_map(|path| symbols_by_file.get(path))
            .flatten()
            .map(|&sym| sym.clone())
            .collect();
        synthesized.extend(c_fnptr::scan_call_sites(&chunk_symbols, &sources, &field_handlers));
        synthesized.extend(js_callback::scan_emissions(&chunk_symbols, &sources, &event_handlers));
        synthesized.extend(py_string_dispatch::scan(&chunk_symbols, &sources, &py_targets));
        synthesized.extend(py_self_calls::scan(
            &chunk_symbols,
            &sources,
            &py_classes,
            &py_parents,
            &py_existing_calls,
        ));
    }

    if synthesized.is_empty() {
        return Ok(());
    }
    // `scan_call_sites`/`scan_emissions` push one edge per regex capture per
    // handler — a function with two textual call/emit sites through the same
    // field/event name produces two identical `Edge`s. `synthesize_all` (the
    // single-shot API this chunked driver replaces) deduped on `(from_id,
    // to_id)` before returning; that dedup must happen here too, since
    // `upsert_edges`'s `INSERT OR REPLACE` is keyed on SQLite's own rowid,
    // not on edge identity, so duplicates land as distinct rows and
    // permanently inflate in-degree (and therefore both the rerank β term
    // and `resolve_edges()`'s in-degree tie-breaker) for repeatedly-called
    // targets.
    synthesized.sort_by(|a, b| a.from_id.cmp(&b.from_id).then(a.to_id.cmp(&b.to_id)));
    synthesized.dedup_by(|a, b| a.from_id == b.from_id && a.to_id == b.to_id);
    tracing::info!("dynamic-dispatch synthesis: {} edge(s) recovered", synthesized.len());
    worker.coordinator.graph.upsert_edges(&synthesized)
}

fn read_chunk_sources(paths: &[PathBuf]) -> HashMap<PathBuf, String> {
    let mut sources = HashMap::new();
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                sources.insert(path.clone(), text);
            }
            Err(e) => tracing::debug!("synthesis: skipping unreadable {:?}: {e}", path),
        }
    }
    sources
}

/// Run the SCIP auto-indexer (e.g. `rust-analyzer scip .`) and ingest its
/// output, if configured. This is a heavyweight, external-compiler-backed
/// pass — tens of seconds even on a modest repo — so it must never sit on the
/// path to marking the project `Fresh`; call it as its own background step
/// alongside (or after) `index_chunk_backlog`, the same way `full_index` does.
/// The subprocess call is dispatched via `spawn_blocking` so it can't stall
/// the async runtime's worker threads for its whole duration.
pub async fn run_scip_enrichment(root: &Path, worker: &Arc<IndexWorker>) -> anyhow::Result<()> {
    let scip_path = worker.scip_file(root);

    // Plan 21 follow-up fix (item 11/SCIP investigation): auto-run is now
    // gated on the `.scip` file's mere EXISTENCE, not on `should_ingest_scip`
    // (which answers "is there NEW work to do," not "does a usable file
    // exist at all" — a file that's already been successfully ingested and
    // hasn't changed makes `should_ingest_scip` return `false` too, and an
    // earlier version of this fix wrongly treated that as "no file, must
    // regenerate," re-triggering the expensive auto-run on every single
    // restart even when nothing needed to change — caught live, mid-session,
    // when it re-ran keycloak's ~28-minute Maven build a second time despite
    // a already-successful prior ingest).
    //
    // The old order (before either fix) always ran the external indexer
    // first, unconditionally, on every full re-index — for a large
    // Java/Maven project that means a full `clean verify` compile, tens of
    // minutes, even when a perfectly good file was already sitting there
    // needing only a (now-fast, see the `merge_index` performance fix in the
    // same investigation) ingest. This was the actual reason SCIP ingestion
    // never visibly completed in this session's earlier attempts — every
    // restart re-triggered a fresh rebuild before ever reaching the ingest
    // step, and this session's own repeated daemon restarts kept
    // interrupting it before it finished. `auto_run` now only fires when no
    // file exists yet at all — and `should_ingest_scip` is checked
    // immediately afterward (below) so a freshly-generated file still gets
    // ingested in the same pass, not just on the next restart.
    if !scip_path.exists() && worker.scip_config.auto_run {
        if let Some(lang) = detect_project_language(root) {
            if let Some(dir) = scip_path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            tracing::info!("auto_run: no ingestable SCIP file yet, running indexer for language={lang}");
            let root_owned = root.to_owned();
            let scip_out2 = scip_path.clone();
            let lang2 = lang.clone();
            let result = tokio::task::spawn_blocking(move || {
                run_scip_indexer(&root_owned, &lang2, &scip_out2)
            }).await;
            match result {
                Ok(Ok(())) => tracing::info!("SCIP auto-run complete: {}", scip_path.display()),
                Ok(Err(e)) => tracing::warn!("SCIP auto-run failed: {e} — continuing without SCIP"),
                Err(e)     => tracing::warn!("SCIP auto-run task panicked: {e}"),
            }
        }
    }

    // Plan 12 C3: ingest SCIP semantic data if a .scip file is present and newer
    // than the last ingest timestamp.
    if worker.coordinator.should_ingest_scip(&scip_path) {
        tracing::info!("SCIP index found at {} — ingesting semantic data", scip_path.display());
        let coordinator = worker.coordinator.clone();
        let scip_path2 = scip_path.clone();
        let root_owned = root.to_owned();
        match tokio::task::spawn_blocking(move || {
            graphrelay_scip::merge_scip(&scip_path2, &coordinator.graph, &root_owned)
        }).await {
            Ok(Ok(stats)) => {
                tracing::info!(
                    "SCIP ingest complete: {} files, {} symbols enriched, {} edges added",
                    stats.files_processed, stats.symbols_enriched, stats.edges_added
                );
                if let Err(e) = worker.coordinator.record_scip_ingest(
                    stats.symbols_enriched, stats.edges_added
                ) {
                    tracing::warn!("failed to record SCIP ingest stats: {e}");
                }
            }
            Ok(Err(e)) => tracing::warn!("SCIP ingest failed: {e}"),
            Err(e)     => tracing::warn!("SCIP ingest task panicked: {e}"),
        }
    }

    Ok(())
}

/// Index a backlog of files produced by `full_index_graph` into the lexical
/// (FTS5) index, in `CHUNK_BATCH_SIZE`-file groups instead of the ~one-file,
/// few-chunk batches `index_path` produces one at a time. Updates
/// `status.semantic_pending` as it drains so `watch_status` can report
/// lexical-index progress separately from graph readiness. Named
/// `semantic_pending`/"semantic index" throughout for now — this field is
/// part of the public `watch_status`/REST surface and is renamed together
/// with the rest of that surface in Plan 20's Phase 5, not here.
pub async fn index_chunk_backlog(
    worker: &Arc<IndexWorker>,
    backlog: Vec<PendingChunk>,
) -> anyhow::Result<()> {
    let total = backlog.len();
    if total == 0 {
        return Ok(());
    }
    tracing::info!("semantic index: indexing {total} file(s) in the background");
    {
        let mut s = worker.status.write().await;
        s.semantic_pending = total;
    }

    let mut remaining = total;
    let mut failed_batches = 0usize;
    let mut last_err: Option<anyhow::Error> = None;
    for group in backlog.chunks(CHUNK_BATCH_SIZE) {
        let mut buf = group.to_vec();
        match worker.flush_chunk_batch(&mut buf).await {
            Ok(unbuilt) if !unbuilt.is_empty() => {
                failed_batches += 1;
                last_err = Some(anyhow::anyhow!("{} file(s) could not be chunked", unbuilt.len()));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("background chunk indexing batch failed: {e}");
                failed_batches += 1;
                last_err = Some(e);
            }
        }
        remaining = remaining.saturating_sub(group.len());
        let mut s = worker.status.write().await;
        s.semantic_pending = remaining;
    }

    if let Some(e) = last_err {
        // Surface the failure on watch_status — the checksum sentinel means
        // affected files retry on the next full index, but agents should know
        // lexical search is incomplete in the meantime rather than seeing a
        // silent "complete" with pending == 0.
        let mut s = worker.status.write().await;
        s.last_error = Some(format!(
            "semantic indexing: {failed_batches} batch(es) failed, last error: {e}"
        ));
        anyhow::bail!("{failed_batches} chunk-indexing batch(es) failed, last error: {e}");
    }

    tracing::info!("semantic index: background chunk indexing complete ({total} file(s))");
    Ok(())
}

/// Full workspace index on startup: the fast graph-only pass followed
/// immediately by lexical indexing. Callers that want the graph queryable
/// before lexical indexing finishes (e.g. server startup) should call
/// `full_index_graph` and `index_chunk_backlog` separately instead of this
/// combined wrapper.
pub async fn full_index(
    root: &Path,
    worker: &Arc<IndexWorker>,
) -> anyhow::Result<()> {
    let Some(_guard) = worker.try_begin_full_index() else {
        tracing::warn!("full index already in progress — skipping overlapping run");
        return Ok(());
    };
    let backlog = full_index_graph(root, worker).await?;
    if let Err(e) = run_scip_enrichment(root, worker).await {
        tracing::warn!("SCIP enrichment failed: {e}");
    }
    index_chunk_backlog(worker, backlog).await
}

// ── Improvement #4: SCIP auto-run helpers ───────────────────────────────────

/// Detect the primary programming language of a project directory.
/// Returns the language name as used by the SCIP CLI commands.
fn detect_project_language(root: &Path) -> Option<String> {
    let checks: &[(&str, &str)] = &[
        ("Cargo.toml",        "rust"),
        ("package.json",      "typescript"),
        ("tsconfig.json",     "typescript"),
        ("pyproject.toml",    "python"),
        ("setup.py",          "python"),
        ("requirements.txt",  "python"),
        ("pom.xml",           "java"),
        ("build.gradle",      "java"),
        ("go.mod",            "go"),
    ];
    for (marker, lang) in checks {
        if root.join(marker).exists() {
            return Some(lang.to_string());
        }
    }
    None
}

/// Check if a binary exists in extended PATH (local SCIP install dirs + system PATH).
/// Extends PATH with well-known local install dirs for scip-python, scip-typescript, scip-java.
fn find_in_extended_path_worker(binary: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = [
        format!("{home}/.local/npm-packages/node_modules/.bin"),
        format!("{home}/.local/bin"),
        format!("{home}/.cargo/bin"),
    ];
    for dir in &dirs {
        let p = std::path::Path::new(dir).join(binary);
        if p.exists() { return Some(p); }
    }
    which::which(binary).ok()
}

/// Plan 21 follow-up fix, found while investigating item 11 (SCIP): this had
/// the exact same bug as `src/main.rs`'s `extended_scip_path()` (a separate,
/// independent copy of the same PATH-extension idea, not shared code) —
/// pushing the raw, already `:`-joined `$PATH` string as a single element
/// into `join_paths` fails on any real system (any `$PATH` with more than
/// one entry), and `.unwrap_or_default()` silently swallowed that into an
/// *empty* PATH. `run_scip_indexer`'s spawned child process would inherit
/// that empty PATH, which could break the indexer's own internal subprocess
/// calls (e.g. `scip-python`/`scip-typescript` invoking `node`, `scip-java`
/// invoking `java`/`mvn`) even though `find_in_extended_path_worker`'s own
/// *availability check* (a direct `.exists()` scan, not `which_in`) was
/// unaffected and correctly reported the indexer as present.
fn extended_path() -> std::ffi::OsString {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = [
        format!("{home}/.local/npm-packages/node_modules/.bin"),
        format!("{home}/.local/bin"),
        format!("{home}/.cargo/bin"),    // rust-analyzer via rustup
    ];
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut parts: Vec<std::ffi::OsString> = extra.iter()
        .map(std::ffi::OsString::from)
        .collect();
    parts.extend(std::env::split_paths(&existing).map(std::ffi::OsString::from));
    std::env::join_paths(parts).unwrap_or_else(|e| {
        tracing::warn!("failed to build extended SCIP PATH: {e}");
        existing
    })
}

/// Run the appropriate SCIP indexer for the given language.
/// Returns Ok(()) on success, Err if the indexer is not found or fails.
fn run_scip_indexer(root: &Path, lang: &str, output: &Path) -> anyhow::Result<()> {
    let out_str = output.to_string_lossy();
    let (binary, args): (&str, Vec<&str>) = match lang {
        "rust"                   => ("rust-analyzer", vec!["scip", "."]),
        "python"                 => ("scip-python",   vec!["index", "--output", &out_str]),
        "typescript" | "javascript" => ("scip-typescript", vec!["index", "--output", &out_str]),
        "java"                   => ("scip-java",     vec!["index", "--output", &out_str]),
        "go"                     => ("scip-go",       vec!["--output", &out_str]),
        other => anyhow::bail!("no SCIP indexer for language: {other}"),
    };

    let path_with_local = extended_path();

    // Check binary is available (search local install dirs + system PATH)
    let found = find_in_extended_path_worker(binary)
        .ok_or_else(|| anyhow::anyhow!(
            "{binary} not found — run `gr scip detect` to see install instructions",
        ))?;

    let status = std::process::Command::new(&found)
        .args(&args)
        .current_dir(root)
        .env("PATH", &path_with_local)  // ensure child also has extended PATH
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run {binary}: {e}"))?;

    anyhow::ensure!(status.success(),
        "{binary} exited with status {status}");
    Ok(())
}

#[cfg(test)]
mod superseded_tests {
    use super::*;
    use graphrelay_core::config::{ScipConfig, WatcherConfig};
    use graphrelay_core::types::{IndexState, IndexStatus};
    use graphrelay_parser::LanguageRegistry;
    use graphrelay_store::GraphDb;

    fn worker(root: &Path) -> Arc<IndexWorker> {
        let graph = Arc::new(GraphDb::open_in_memory().unwrap());
        let status = Arc::new(RwLock::new(IndexStatus {
            state: IndexState::Indexing,
            last_indexed_at: None,
            pending_count: 0,
            total_symbols: 0,
            total_files: 0,
            last_error: None,
            semantic_pending: 0,
        }));
        Arc::new(IndexWorker::new(
            Arc::new(FileWatcher::new(root.to_owned(), &[], &["rust".to_string()]).unwrap()),
            Arc::new(Extractor::new(LanguageRegistry::with_defaults())),
            Arc::new(StoreCoordinator::new(graph)),
            status,
            Arc::new(graphrelay_core::retrieval::SourceLoader::new()),
            WatcherConfig::default(),
            ScipConfig::default(),
        ))
    }

    /// Positive control: an untouched file commits its real checksum.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unchanged_file_commits_and_reports_nothing_pending() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = root.join("a.rs");
        std::fs::write(&path, "pub fn one() {}").unwrap();
        let w = worker(&root);
        let pending = w.index_path_graph_only(&path).await.unwrap().unwrap();
        let real = pending.checksum;
        let mut buf = vec![pending];
        assert!(w.flush_chunk_batch(&mut buf).await.unwrap().is_empty());
        assert_eq!(w.coordinator.graph.file_checksum(&path).unwrap(), Some(real));
    }

    /// A file rewritten after its graph write is re-indexed from disk, not
    /// reported as a failure (that flipped `watch_status` to stale and REST to
    /// 503) and not dropped (that left a stale full-index write in place when
    /// the live event for the rewrite had already been consumed).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_file_changed_after_its_graph_write_is_reindexed_not_failed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = root.join("a.rs");
        std::fs::write(&path, "pub fn one() {}").unwrap();
        let w = worker(&root);
        let pending = w.index_path_graph_only(&path).await.unwrap().unwrap();
        std::fs::write(&path, "pub fn one() {}\npub fn two() {}").unwrap();
        let mut buf = vec![pending];
        let unbuilt = w.flush_chunk_batch(&mut buf).await.unwrap();
        assert!(unbuilt.is_empty(), "a changed file must not be reported as a build failure");
        let current = xxhash_rust::xxh64::xxh64(&std::fs::read(&path).unwrap(), 0);
        assert_eq!(w.coordinator.graph.file_checksum(&path).unwrap(), Some(current));
        let file_id = w.coordinator.graph.file_by_path(&path).unwrap().unwrap().id;
        let names: Vec<String> = w.coordinator.graph.symbols_in_file(file_id).unwrap().into_iter().map(|s| s.name).collect();
        assert!(names.contains(&"two".to_string()), "the on-disk version's symbols are indexed: {names:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_file_deleted_after_its_graph_write_is_dropped_not_failed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let path = root.join("a.rs");
        std::fs::write(&path, "pub fn one() {}").unwrap();
        let w = worker(&root);
        let pending = w.index_path_graph_only(&path).await.unwrap().unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut buf = vec![pending];
        assert!(w.flush_chunk_batch(&mut buf).await.unwrap().is_empty());
        assert!(w.coordinator.graph.file_by_path(&path).unwrap().is_none(), "no ghost file row survives");
    }
}
