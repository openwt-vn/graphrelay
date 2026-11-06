//! Regression test for the parallel full-index path's checksum-match skip
//! branch (`full_index_graph_blocking` in `graphrelay-watcher/src/worker.rs`):
//! re-running a full index over files whose content hasn't changed must
//! still bump `files.indexed_at`, mirroring the live single-file watcher
//! path's `touch_files_indexed_at` call. Without this, a disk mtime change
//! with no content change (a fresh clone, `git checkout`/`rebase`/`stash`, a
//! CI checkout) followed by a re-index would leave every unchanged file
//! permanently `mtime > indexed_at` — the `is_possibly_stale` condition in
//! `graphrelay-mcp` — even though a real re-index just ran over it.

use graphrelay_core::{
    config::{ScipConfig, WatcherConfig},
    retrieval::SourceLoader,
    types::{IndexState, IndexStatus},
};
use graphrelay_parser::{extractor::generate_file_id, Extractor, LanguageRegistry};
use graphrelay_store::{GraphDb, StoreCoordinator};
use graphrelay_watcher::{full_index_graph, FileWatcher, IndexWorker};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_index_rerun_bumps_indexed_at_for_unchanged_files() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let path = root.join("unchanged.rs");
    std::fs::write(&path, "pub fn unchanged_fn() {}").unwrap();
    let file_id = generate_file_id(&path);

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let coordinator = Arc::new(StoreCoordinator::new(graph.clone()));
    let source_loader = Arc::new(SourceLoader::new());
    let status = Arc::new(RwLock::new(IndexStatus {
        state: IndexState::Indexing, last_indexed_at: None, pending_count: 0,
        total_symbols: 0, total_files: 0, last_error: None, semantic_pending: 0,
    }));
    let watcher = Arc::new(FileWatcher::new(root.clone(), &[], &["rust".to_string()]).unwrap());
    let extractor = Arc::new(Extractor::new(LanguageRegistry::with_defaults()));
    let worker_config = WatcherConfig::default();
    let scip_config = ScipConfig { auto_run: false, ..Default::default() };

    let worker = Arc::new(IndexWorker::new(
        watcher, extractor, coordinator, status, source_loader,
        worker_config, scip_config,
    ));

    full_index_graph(&root, &worker).await.expect("first full index");
    let before = graph.file_by_path(&path).unwrap().expect("file row exists after first index");

    // Move mtime forward without changing content — exactly what a `touch`,
    // `git checkout`, or fresh clone does — then re-run the full index. This
    // is the checksum-match skip branch under test.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    std::fs::write(&path, "pub fn unchanged_fn() {}").unwrap();

    full_index_graph(&root, &worker).await.expect("second full index (unchanged content)");
    let after = graph.file_by_path(&path).unwrap().expect("file row exists after second index");

    assert_eq!(after.checksum, before.checksum, "content unchanged, checksum must match");
    assert_ne!(
        after.indexed_at, before.indexed_at,
        "indexed_at must be bumped on a checksum-match re-index, or the file stays \
         permanently possibly_stale after any mtime-only change"
    );

    let syms = graph.symbols_in_file(file_id).unwrap();
    assert_eq!(syms.len(), 1, "checksum-match skip path must not duplicate or drop symbols");
}
