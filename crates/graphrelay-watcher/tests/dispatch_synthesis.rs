//! End-to-end test that the C function-pointer dispatch synthesis pass
//! (`graphrelay_parser::synthesis::c_fnptr`) is actually wired into
//! `full_index_graph`, not just unit-tested in isolation.

use graphrelay_core::{
    config::{ScipConfig, WatcherConfig},
    retrieval::SourceLoader,
    types::{EdgeSource, IndexState, IndexStatus},
};
use graphrelay_parser::{extractor::generate_file_id, Extractor, LanguageRegistry};
use graphrelay_store::{GraphDb, StoreCoordinator};
use graphrelay_watcher::{full_index_graph, FileWatcher, IndexWorker};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::RwLock;

const DISPATCH_C: &str = r#"
struct cmd {
    const char *name;
    void (*fn)(int argc, char **argv);
};

void cmd_add(int argc, char **argv) { }
void cmd_remove(int argc, char **argv) { }

static struct cmd cmds[] = {
    {"add", cmd_add},
    {"remove", cmd_remove},
};

void dispatch(struct cmd *p, int argc, char **argv) {
    p->fn(argv);
}
"#;

async fn build_worker(root: &std::path::Path, graph: Arc<GraphDb>, synthesis_enabled: bool) -> Arc<IndexWorker> {
    build_worker_for_languages(root, graph, synthesis_enabled, &["c"]).await
}

async fn build_worker_for_languages(
    root: &std::path::Path,
    graph: Arc<GraphDb>,
    synthesis_enabled: bool,
    languages: &[&str],
) -> Arc<IndexWorker> {
    let coordinator = Arc::new(StoreCoordinator::new(graph));

    let source_loader = Arc::new(SourceLoader::new());
    let status = Arc::new(RwLock::new(IndexStatus {
        state: IndexState::Indexing, last_indexed_at: None, pending_count: 0,
        total_symbols: 0, total_files: 0, last_error: None, semantic_pending: 0,
    }));
    let langs: Vec<String> = languages.iter().map(|s| s.to_string()).collect();
    let watcher = Arc::new(FileWatcher::new(root.to_owned(), &[], &langs).unwrap());
    let extractor = Arc::new(Extractor::new(LanguageRegistry::with_defaults()));
    let worker_config = WatcherConfig { synthesis_enabled, ..Default::default() };
    let scip_config = ScipConfig { auto_run: false, ..Default::default() };

    Arc::new(IndexWorker::new(
        watcher, extractor, coordinator, status, source_loader,
        worker_config, scip_config,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_index_synthesizes_dispatch_edges() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("dispatch.c"), DISPATCH_C).unwrap();

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let worker = build_worker(&root, graph.clone(), true).await;

    full_index_graph(&root, &worker).await.expect("full_index_graph failed");

    let dispatch_id = generate_file_id(&root.join("dispatch.c"));
    let symbols = graph.symbols_in_file(dispatch_id).unwrap();
    let dispatch_sym = symbols.iter().find(|s| s.name == "dispatch").expect("dispatch() not extracted");
    let cmd_add_sym = symbols.iter().find(|s| s.name == "cmd_add").expect("cmd_add() not extracted");

    let edges = graph.edges_between(&[dispatch_sym.id, cmd_add_sym.id]).unwrap();
    let synthesized = edges
        .iter()
        .find(|e| e.from_id == dispatch_sym.id && e.to_id == Some(cmd_add_sym.id));
    let edge = synthesized.expect("expected a synthesized dispatch->cmd_add edge after full_index_graph");
    assert_eq!(edge.source, EdgeSource::Synthesized);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_index_skips_synthesis_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("dispatch.c"), DISPATCH_C).unwrap();

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let worker = build_worker(&root, graph.clone(), false).await;

    full_index_graph(&root, &worker).await.expect("full_index_graph failed");

    let dispatch_id = generate_file_id(&root.join("dispatch.c"));
    let symbols = graph.symbols_in_file(dispatch_id).unwrap();
    let dispatch_sym = symbols.iter().find(|s| s.name == "dispatch").unwrap();
    let cmd_add_sym = symbols.iter().find(|s| s.name == "cmd_add").unwrap();

    let edges = graph.edges_between(&[dispatch_sym.id, cmd_add_sym.id]).unwrap();
    assert!(
        !edges.iter().any(|e| e.from_id == dispatch_sym.id && e.to_id == Some(cmd_add_sym.id)),
        "synthesis_enabled=false must not produce a dispatch->cmd_add edge",
    );
}

/// Regression test for review finding: `run_dispatch_synthesis` used to call
/// `upsert_edges` without first clearing previously-synthesized edges.
/// `edges` has no unique constraint beyond its row id, so `INSERT OR
/// REPLACE` never actually replaced a matching edge — every full-index run
/// (including every server restart, since `full_index_graph` runs on normal
/// startup) appended duplicates instead of refreshing. This runs
/// `full_index_graph` on the same unchanged repo three times and asserts the
/// synthesized edge count stays at exactly 1, not 3.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_full_index_does_not_duplicate_synthesized_edges() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("dispatch.c"), DISPATCH_C).unwrap();

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let worker = build_worker(&root, graph.clone(), true).await;

    for _ in 0..3 {
        full_index_graph(&root, &worker).await.expect("full_index_graph failed");
    }

    let dispatch_id = generate_file_id(&root.join("dispatch.c"));
    let symbols = graph.symbols_in_file(dispatch_id).unwrap();
    let dispatch_sym = symbols.iter().find(|s| s.name == "dispatch").unwrap();
    let cmd_add_sym = symbols.iter().find(|s| s.name == "cmd_add").unwrap();

    let edges = graph.edges_between(&[dispatch_sym.id, cmd_add_sym.id]).unwrap();
    let matching: Vec<_> = edges
        .iter()
        .filter(|e| e.from_id == dispatch_sym.id && e.to_id == Some(cmd_add_sym.id))
        .collect();
    assert_eq!(
        matching.len(), 1,
        "expected exactly 1 dispatch->cmd_add synthesized edge after 3 full-index runs, found {}: {:?}",
        matching.len(), matching,
    );
}

const BUS_JS: &str = r#"
class Bus {
    constructor() {
        this.on('save', this.handleSave);
    }

    handleSave() {
        console.log('saved');
    }

    trigger() {
        this.emit('save');
    }
}
"#;

/// End-to-end proof that the JS/TS callback synthesis pass
/// (`graphrelay_parser::synthesis::js_callback`)
/// is wired into `full_index_graph` the same way the C fn-ptr pass is —
/// `run_dispatch_synthesis` must fetch JavaScript/TypeScript symbols, not
/// just C/C++, for this to produce anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_index_synthesizes_js_callback_edges() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("bus.js"), BUS_JS).unwrap();

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let worker = build_worker_for_languages(&root, graph.clone(), true, &["javascript"]).await;

    full_index_graph(&root, &worker).await.expect("full_index_graph failed");

    let bus_id = generate_file_id(&root.join("bus.js"));
    let symbols = graph.symbols_in_file(bus_id).unwrap();
    let trigger_sym = symbols.iter().find(|s| s.name == "trigger").expect("trigger() not extracted");
    let handle_save_sym = symbols.iter().find(|s| s.name == "handleSave").expect("handleSave() not extracted");

    let edges = graph.edges_between(&[trigger_sym.id, handle_save_sym.id]).unwrap();
    let synthesized = edges
        .iter()
        .find(|e| e.from_id == trigger_sym.id && e.to_id == Some(handle_save_sym.id));
    let edge = synthesized.expect("expected a synthesized trigger->handleSave edge after full_index_graph");
    assert_eq!(edge.source, EdgeSource::Synthesized);
}

const DISPATCH_TWO_CALL_SITES_C: &str = r#"
struct cmd {
    const char *name;
    void (*fn)(int argc, char **argv);
};

void cmd_add(int argc, char **argv) { }

static struct cmd cmds[] = {
    {"add", cmd_add},
};

void dispatch(struct cmd *p, int argc, char **argv) {
    p->fn(argv);
    p->fn(argv);
}
"#;

/// Regression test for a code-review finding on the large-repo memory fix
/// (see `bench-reports/large-repo-indexing-2026-08-04.md`):
/// the chunked driver (`synthesize_dispatch_edges`, added to bound memory on
/// real large-repo runs) replaced `synthesize_all`'s single-shot call —
/// which deduped edges on `(from_id, to_id)` before returning — with direct
/// accumulation from `scan_call_sites`/`scan_emissions` and no dedup step.
/// `dispatch()` here calls `p->fn(argv)` twice, so `scan_call_sites`
/// produces two identical `dispatch -> cmd_add` edges from two regex
/// captures; without a dedup step both land as distinct rows (`upsert_edges`
/// is keyed on SQLite's own rowid, not edge identity), permanently
/// inflating `cmd_add`'s in-degree. Asserts exactly 1 edge, not 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_call_sites_through_the_same_field_do_not_duplicate_the_edge() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("dispatch.c"), DISPATCH_TWO_CALL_SITES_C).unwrap();

    let graph = Arc::new(GraphDb::open_in_memory().unwrap());
    let worker = build_worker(&root, graph.clone(), true).await;

    full_index_graph(&root, &worker).await.expect("full_index_graph failed");

    let dispatch_id = generate_file_id(&root.join("dispatch.c"));
    let symbols = graph.symbols_in_file(dispatch_id).unwrap();
    let dispatch_sym = symbols.iter().find(|s| s.name == "dispatch").unwrap();
    let cmd_add_sym = symbols.iter().find(|s| s.name == "cmd_add").unwrap();

    let edges = graph.edges_between(&[dispatch_sym.id, cmd_add_sym.id]).unwrap();
    let matching: Vec<_> = edges
        .iter()
        .filter(|e| e.from_id == dispatch_sym.id && e.to_id == Some(cmd_add_sym.id))
        .collect();
    assert_eq!(
        matching.len(), 1,
        "expected exactly 1 dispatch->cmd_add edge despite 2 textual call sites, found {}: {:?}",
        matching.len(), matching,
    );
}
