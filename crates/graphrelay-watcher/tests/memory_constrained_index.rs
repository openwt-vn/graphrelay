//! Integration test for the memory-budget-aware parse pool sizing
//! (`graphrelay_watcher::memory_budget`): forcing an
//! extremely tight `memory_budget_mb` override (well below any real file's
//! cost) must shrink the full-index parse pool down to sequential (size 1)
//! without breaking correctness — every file still gets indexed exactly
//! once, with the right symbols. `memory_budget::tests` unit-tests the pool
//! sizing math in isolation; this test proves the wiring through
//! `full_index_graph` doesn't regress under the smallest possible pool.

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
async fn tight_memory_budget_still_indexes_every_file_correctly() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();

    let n = 20;
    for i in 0..n {
        std::fs::write(
            root.join(format!("f{i}.rs")),
            format!("pub fn func_{i}() {{ let _ = {i}; }}"),
        )
        .unwrap();
    }

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let coordinator = Arc::new(StoreCoordinator::new(graph.clone()));

    let source_loader = Arc::new(SourceLoader::new());
    let status = Arc::new(RwLock::new(IndexStatus {
        state: IndexState::Indexing, last_indexed_at: None, pending_count: 0,
        total_symbols: 0, total_files: 0, last_error: None, semantic_pending: 0,
    }));
    let watcher = Arc::new(FileWatcher::new(root.clone(), &[], &["rust".to_string()]).unwrap());
    let extractor = Arc::new(Extractor::new(LanguageRegistry::with_defaults()));
    // 1 MiB is far below `DEFAULT_PER_FILE_ESTIMATE_BYTES` (8 MiB), forcing
    // `resolve_pool_size` down to a sequential (size 1) pool regardless of
    // how many cores this test machine has.
    let worker_config = WatcherConfig { memory_budget_mb: Some(1), ..Default::default() };
    let scip_config = ScipConfig { auto_run: false, ..Default::default() };

    let worker = Arc::new(IndexWorker::new(
        watcher, extractor, coordinator, status, source_loader,
        worker_config, scip_config,
    ));

    full_index_graph(&root, &worker).await.expect("full_index_graph failed under a size-1 pool");

    for i in 0..n {
        let file_id = generate_file_id(&root.join(format!("f{i}.rs")));
        let syms = graph.symbols_in_file(file_id).unwrap();
        assert_eq!(syms.len(), 1, "file f{i}.rs should have exactly one symbol");
        assert_eq!(syms[0].name, format!("func_{i}"), "wrong symbol extracted for f{i}.rs");
    }
    assert_eq!(graph.total_file_count().unwrap(), n as u64);
}
