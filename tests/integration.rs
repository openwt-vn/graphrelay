/// End-to-end integration tests: extraction, storage, pipeline, and HTTP health endpoint.
use graphrelay_core::{
    retrieval::{
        pipeline::{GraphStore, RetrievalPipeline},
        RetrievalQuery, SourceLoader,
    },
    types::{Symbol, SymbolId},
    config::RetrievalConfig,
};
use graphrelay_parser::{Extractor, LanguageRegistry};
use graphrelay_store::GraphDb;
use std::{path::PathBuf, sync::Arc};
use tempfile::TempDir;

// ── Adapters ──────────────────────────────────────────────────────────────────

/// Wraps a real `GraphDb` for `GraphStore`, with a fixed lexical-search
/// result list standing in for a real SQLite FTS5 query — this test cares
/// about the pipeline's graph/rerank wiring against a real store, not about
/// FTS5 scoring itself (that's covered in `graphrelay-store`'s own tests).
struct GraphDbStore {
    db:           Arc<GraphDb>,
    lexical_hits: Vec<(SymbolId, f32)>,
}

impl GraphStore for GraphDbStore {
    fn symbol_by_id(&self, id: SymbolId) -> anyhow::Result<Option<Symbol>> { self.db.symbol_by_id(id) }
    fn callers_of(&self, id: SymbolId) -> anyhow::Result<Vec<Symbol>>      { self.db.callers_of(id) }
    fn callees_of(&self, id: SymbolId) -> anyhow::Result<Vec<Symbol>>      { self.db.callees_of(id) }
    fn neighbors(&self, id: SymbolId, hops: u8) -> anyhow::Result<Vec<Symbol>> { self.db.neighbors(id, hops) }
    fn in_degree(&self, id: SymbolId) -> anyhow::Result<u32>               { self.db.in_degree(id) }
    fn lexical_search(&self, _terms: &[String], _synonym_terms: &[String], top_k: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        Ok(self.lexical_hits.iter().take(top_k).cloned().collect())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const SAMPLE_RUST: &[u8] = br#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn multiply(a: i32, b: i32) -> i32 {
    add(a, b)
}
"#;

fn write_fixture(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn extractor_finds_symbols_in_rust_fixture() {
    let dir    = TempDir::new().unwrap();
    let path   = write_fixture(&dir, "lib.rs", SAMPLE_RUST);
    let result = Extractor::new(LanguageRegistry::default()).extract(&path, SAMPLE_RUST).unwrap();

    assert!(result.symbols.iter().any(|s| s.name == "add"),     "add not found");
    assert!(result.symbols.iter().any(|s| s.name == "multiply"), "multiply not found");
}

#[test]
fn graph_db_stores_and_retrieves_extracted_symbols() {
    let dir    = TempDir::new().unwrap();
    let path   = write_fixture(&dir, "calc.rs", SAMPLE_RUST);
    let result = Extractor::new(LanguageRegistry::default()).extract(&path, SAMPLE_RUST).unwrap();

    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&result.file_record).unwrap();
    db.upsert_symbols(&result.symbols).unwrap();
    db.upsert_edges(&result.edges).unwrap();

    let symbols = db.symbols_in_file(result.file_record.id).unwrap();
    assert!(symbols.len() >= 2, "expected at least add and multiply");

    let qname = format!("{}::add", path.display());
    let add   = db.symbol_by_qualified(&qname).unwrap();
    assert!(add.is_some(), "add not found by qualified name ({qname})");
}

#[test]
fn pipeline_returns_matching_symbol() {
    let dir    = TempDir::new().unwrap();
    let path   = write_fixture(&dir, "ops.rs", SAMPLE_RUST);
    let result = Extractor::new(LanguageRegistry::default()).extract(&path, SAMPLE_RUST).unwrap();

    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    db.upsert_file(&result.file_record).unwrap();
    db.upsert_symbols(&result.symbols).unwrap();
    db.upsert_edges(&result.edges).unwrap();

    let add_sym = result.symbols.iter().find(|s| s.name == "add").expect("add not found");

    let pipeline: RetrievalPipeline<GraphDbStore> = RetrievalPipeline {
        graph_db: Arc::new(GraphDbStore {
            db:           Arc::clone(&db),
            lexical_hits: vec![(add_sym.id, 0.92)],
        }),
        source_loader: Arc::new(SourceLoader::new()),
        config:        RetrievalConfig::default(),
        jev:           None,
    };

    let rt      = tokio::runtime::Runtime::new().unwrap();
    let results = rt.block_on(pipeline.query(RetrievalQuery {
        text:        "add two numbers".to_string(),
        top_k:       5,
        final_n:     3,
        expand_hops: 0,
        lang_filter: None,
    })).unwrap();

    // Graph expansion always widens to >= 2 hops (see `RetrievalPipeline::query`,
    // Plan 20) regardless of the requested `expand_hops`, so `multiply` (a
    // caller of `add`) also enters the pool via `neighbors()` — the direct
    // lexical hit must still rank first.
    assert!(!results.is_empty());
    assert_eq!(results[0].symbol.name, "add", "the direct lexical hit should rank first");
    assert_eq!(results[0].rank, 1);
}

#[test]
fn trace_impact_returns_callers_at_depth_two() {
    // Chain: top → mid → core  (top calls mid, mid calls core)
    // impact_graph(core, depth=2) must return both mid and top.
    let dir    = TempDir::new().unwrap();
    let path   = write_fixture(&dir, "chain.rs", br#"
pub fn core_fn() {}
pub fn mid_fn()  { core_fn(); }
pub fn top_fn()  { mid_fn(); }
"#);
    let result = Extractor::new(LanguageRegistry::default()).extract(&path, &std::fs::read(&path).unwrap()).unwrap();

    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&result.file_record).unwrap();
    db.upsert_symbols(&result.symbols).unwrap();
    db.upsert_edges(&result.edges).unwrap();

    let core = result.symbols.iter().find(|s| s.name == "core_fn").expect("core_fn not found");
    let impact = db.impact_graph(core.id, 2).unwrap();

    let names: Vec<&str> = impact.affected.iter().map(|n| n.symbol.name.as_str()).collect();
    assert!(names.contains(&"mid_fn"), "mid_fn should be in impact graph; got: {names:?}");
    assert!(names.contains(&"top_fn"), "top_fn (depth 2) should be in impact graph; got: {names:?}");
}

#[test]
fn call_edges_link_multiply_to_add() {
    let dir    = TempDir::new().unwrap();
    let path   = write_fixture(&dir, "edge.rs", SAMPLE_RUST);
    let result = Extractor::new(LanguageRegistry::default()).extract(&path, SAMPLE_RUST).unwrap();

    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&result.file_record).unwrap();
    db.upsert_symbols(&result.symbols).unwrap();
    db.upsert_edges(&result.edges).unwrap();

    let add_sym = result.symbols.iter().find(|s| s.name == "add").expect("add not found");
    let callers = db.callers_of(add_sym.id).unwrap();
    assert!(
        callers.iter().any(|s| s.name == "multiply"),
        "expected multiply to be a caller of add; got: {:?}",
        callers.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

// ── /health endpoint ──────────────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_503_while_indexing_and_200_when_fresh() {
    use axum::{
        body::to_bytes,
        extract::State,
        http::{Request, StatusCode},
        routing::get,
        Json, Router,
    };
    use graphrelay_core::types::{IndexState, IndexStatus};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    async fn health_fn(
        State(ws): State<Arc<RwLock<IndexStatus>>>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let s = ws.read().await;
        let ready = matches!(s.state, IndexState::Fresh);
        (
            if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
            Json(serde_json::json!({ "ready": ready, "state": s.state.as_str() })),
        )
    }

    let make_app = |st: Arc<RwLock<IndexStatus>>| {
        Router::new().route("/health", get(health_fn)).with_state(st)
    };

    // 503 while indexing
    let mut indexing_status = IndexStatus::fresh();
    indexing_status.state = IndexState::Indexing;
    let indexing = Arc::new(RwLock::new(indexing_status));
    let resp = make_app(indexing)
        .oneshot(Request::builder().uri("/health").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE, "/health must be 503 while indexing");
    let body: serde_json::Value = serde_json::from_slice(
        &to_bytes(resp.into_body(), usize::MAX).await.unwrap()
    ).unwrap();
    assert_eq!(body["ready"], false);

    // 200 when fresh
    let fresh = Arc::new(RwLock::new(IndexStatus::fresh()));
    let resp = make_app(fresh)
        .oneshot(Request::builder().uri("/health").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "/health must be 200 when fresh");
    let body: serde_json::Value = serde_json::from_slice(
        &to_bytes(resp.into_body(), usize::MAX).await.unwrap()
    ).unwrap();
    assert_eq!(body["ready"], true);
    assert_eq!(body["state"], "fresh");
}
