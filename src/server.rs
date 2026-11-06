use anyhow::Context;
use axum::{extract::State, routing::get, Json, Router};
use graphrelay_api::ApiState;
use graphrelay_core::{
    relevance_judge::RelevanceJudge,
    retrieval::{RetrievalPipeline, SourceLoader},
    types::{IndexState, IndexStatus},
    Config,
};
use graphrelay_decision_engine::{JevJudge, LayaJudge};
use graphrelay_mcp::McpAppState;
use graphrelay_parser::{Extractor, LanguageRegistry};
use graphrelay_store::{GraphDb, StoreCoordinator};
use graphrelay_watcher::{FileWatcher, IndexWorker};
use std::{path::Path, sync::Arc};
use tokio::sync::RwLock;

use crate::registry::{project_data_dir, ProjectEntry};

/// Boot a single project: opens stores, spawns indexer + watcher, returns a
/// Router that can be nested under `/{name}/` by the caller.
pub async fn build_project_router(
    entry: &ProjectEntry,
    shutdown: Arc<tokio::sync::Notify>,
) -> anyhow::Result<Router> {
    let root = entry
        .root
        .canonicalize()
        .unwrap_or_else(|_| entry.root.clone());

    let config = load_project_config(&root)?;
    tracing::info!(
        project = %entry.name,
        beta = config.retrieval.beta,
        gamma = config.retrieval.gamma,
        path_pin_mode = ?config.retrieval.path_pin.mode,
        path_pin_bonus = config.retrieval.path_pin.bonus,
        ppr_iterations = config.retrieval.ppr.iterations,
        ppr_early_stop_epsilon = ?config.retrieval.ppr.early_stop_epsilon,
        direct_hit_graph_credit_damping = config.retrieval.direct_hit_graph_credit_damping,
        jev_enabled = config.retrieval.jev.enabled,
        laya_enabled = config.retrieval.laya.enabled,
        "effective retrieval config"
    );

    let data_dir = project_data_dir(&root);
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating data dir {:?}", data_dir))?;

    let db_path = data_dir.join("graph.db");

    tracing::info!("[{}] SQLite → {:?}", entry.name, db_path);
    let graph_db = Arc::new(GraphDb::open(&db_path)?);

    let coordinator = Arc::new(StoreCoordinator::new(graph_db.clone()));
    let source_loader = Arc::new(SourceLoader::new());
    let jev = build_jev_judge(&entry.name, &config)?;

    let pipeline: Arc<dyn graphrelay_core::retrieval::QueryPipeline> =
        Arc::new(RetrievalPipeline {
            graph_db:      graph_db.clone(),
            source_loader: source_loader.clone(),
            config:        config.retrieval.clone(),
            jev,
        });

    let watch_status = Arc::new(RwLock::new(IndexStatus {
        state:            IndexState::Indexing,
        last_indexed_at:  None,
        pending_count:    0,
        total_symbols:    0,
        total_files:      0,
        last_error:       None,
        semantic_pending: 0,
    }));

    // Merge index.ignore + watcher.ignore so full_index and the live watcher
    // both apply the same exclusion set.
    let combined_ignores: Vec<String> = config
        .watcher
        .ignore
        .iter()
        .chain(config.index.ignore.iter())
        .cloned()
        .collect();
    let watcher = Arc::new(FileWatcher::new(root.clone(), &combined_ignores, &config.index.languages)?);
    let extractor = Arc::new(Extractor::new(LanguageRegistry::with_defaults()).with_root(root.clone()));
    let worker = Arc::new(IndexWorker::new(
        watcher.clone(),
        extractor,
        coordinator,
        watch_status.clone(),
        source_loader.clone(),
        config.watcher.clone(),
        config.scip.clone(),
    ));

    // Startup hygiene: `chunk_fts` has no foreign-key cascade, so drop any rows
    // whose symbol is gone (they would keep winning `top_k` slots). An FTS-format
    // migration needs no separate backfill: it resets files to pending and the
    // full index below re-chunks them through the normal path.
    match graph_db.gc_orphan_fts() {
        Ok(0) => {}
        Ok(n) => tracing::info!("[{}] removed {n} orphaned lexical-index row(s)", entry.name),
        Err(e) => tracing::warn!("[{}] lexical-index orphan cleanup failed: {e}", entry.name),
    }

    // Initial full index in background: the graph pass (parse + SQLite) runs
    // first and is fast, so the project is marked Fresh — and every graph-based
    // MCP/REST tool becomes usable — as soon as it completes. Lexical (FTS5)
    // chunk indexing and SCIP enrichment (which shells out to an external
    // compiler-backed indexer and can take tens of seconds even on a modest
    // repo) then run concurrently in a second, fully independent background
    // task; `watch_status.semantic_pending` reports lexical-indexing progress
    // until it drains to 0.
    {
        let worker = worker.clone();
        let root = root.clone();
        let ws = watch_status.clone();
        let gdb = graph_db.clone();
        let name = entry.name.clone();
        tokio::spawn(async move {
            let Some(guard) = worker.try_begin_full_index() else {
                // Shouldn't happen this early, but guards against a duplicate
                // startup call racing e.g. watcher-overflow recovery.
                tracing::warn!("[{name}] full index already in progress — skipping startup run");
                return;
            };
            let backlog = match graphrelay_watcher::full_index_graph(&root, &worker).await {
                Ok(backlog) => backlog,
                Err(e) => {
                    tracing::error!("[{name}] full index failed: {e}");
                    let mut s = ws.write().await;
                    s.state = IndexState::Stale;
                    s.last_error = Some(e.to_string());
                    return;
                }
            };

            let total_files = gdb.total_file_count().unwrap_or(0);
            let total_symbols = gdb.total_symbol_count().unwrap_or(0);
            {
                let mut s = ws.write().await;
                s.state = IndexState::Fresh;
                s.last_indexed_at = Some(chrono::Utc::now());
                s.total_files = total_files;
                s.total_symbols = total_symbols;
                s.semantic_pending = backlog.len();
            }
            tracing::info!(
                "[{name}] graph ready: {total_files} files, {total_symbols} symbols — \
                 lexical-indexing {} file(s) and running SCIP enrichment in the background",
                backlog.len(),
            );

            // `index_chunk_backlog` already records details on
            // `watch_status.last_error` before returning `Err` — just log
            // here. `guard` is held until this task finishes, so it covers
            // the whole full_index_graph → (SCIP + chunk indexing) sequence,
            // not just the fast graph pass above.
            tokio::spawn(async move {
                let _guard = guard;
                // Chunk text embeds each symbol's callers/callees, so an ingest
                // that lands mid-backlog leaves some chunks built with semantic
                // edges and some without — different FTS content on identical
                // runs (Plan 24 item 1.7). When a `.scip` file already exists the
                // ingest is fast, so run it first. When it doesn't, the external
                // indexer can take minutes to hours; lexical search must not wait
                // on that, so the two stay concurrent and those chunks are built
                // from the syntax graph only.
                let (scip_result, index_result) = if worker.scip_file(&root).exists() {
                    let scip = graphrelay_watcher::run_scip_enrichment(&root, &worker).await;
                    (scip, graphrelay_watcher::index_chunk_backlog(&worker, backlog).await)
                } else {
                    tokio::join!(
                        graphrelay_watcher::run_scip_enrichment(&root, &worker),
                        graphrelay_watcher::index_chunk_backlog(&worker, backlog),
                    )
                };
                if let Err(e) = scip_result {
                    tracing::warn!("[{name}] SCIP enrichment failed: {e}");
                }
                if let Err(e) = index_result {
                    tracing::error!("[{name}] background semantic indexing failed: {e}");
                }
            });
        });
    }
    // Continuous watcher
    {
        let worker = worker.clone();
        tokio::spawn(async move { worker.run(shutdown).await });
    }

    let mcp_state = McpAppState {
        graph_db:      graph_db.clone(),
        source_loader: source_loader.clone(),
        watch_status:  watch_status.clone(),
        pipeline:      pipeline.clone(),
    };
    let api_state = ApiState {
        graph_db:      graph_db.clone(),
        source_loader: source_loader.clone(),
        watch_status:  watch_status.clone(),
        pipeline:      pipeline.clone(),
    };
    let health_router = Router::new()
        .route("/health", get(health_handler))
        .with_state(watch_status);

    // Register the UI index directly at "/" — Axum 0.7 nest + merge has a quirk
    // where a route("/", ...) inside a merged sub-router is not reachable after nesting.
    // Pass the project name so the served HTML has the correct __BASE__ substitution.
    let base = format!("/{}", entry.name);
    let router = Router::new()
        .route("/", get(move || graphrelay_ui::serve_index_for(base.clone())))
        .merge(graphrelay_mcp::mcp_router(mcp_state).layer(axum::middleware::from_fn(crate::offload::blocking_handler)))
        .merge(graphrelay_api::api_router(api_state).layer(axum::middleware::from_fn(crate::offload::blocking_handler)))
        .merge(health_router)
        .merge(graphrelay_ui::router());

    Ok(router)
}

/// Checks project-local `.graphrelay/config.toml` first, then `graphrelay.toml`
/// at the project root, then falls back to defaults. Logs which one it used
/// at INFO — found the hard way (Plan 22 item 3's own validation sweep) that
/// a pre-existing `.graphrelay/config.toml` silently shadows a top-level
/// `graphrelay.toml` a later experiment writes, with no error or warning:
/// without this log, a whole day's worth of "arm" measurements could (and
/// once did) silently compare a config against itself. Anyone changing a
/// project's `graphrelay.toml` and not seeing the expected effect should
/// check this line before suspecting the code.
pub(crate) fn load_project_config(root: &Path) -> anyhow::Result<Config> {
    let local = root.join(".graphrelay").join("config.toml");
    if local.exists() {
        tracing::info!(config_path = ?local, "loaded project config");
        return Config::load(&local);
    }
    let top = root.join("graphrelay.toml");
    if top.exists() {
        tracing::info!(config_path = ?top, "loaded project config");
        return Config::load(&top);
    }
    tracing::info!("no project config file found, using defaults");
    Ok(Config::default())
}

/// Plan 20 Phase 8 / Plan 25: build the optional relevance judge for a project, or `None`
/// when none is enabled (the default) or the enabled one is misconfigured. Returning
/// `None` here rather than constructing a judge that would fail on every single query is
/// deliberate — `Config::validate()` already warned at load time; this is where that state
/// actually turns into "don't call it." Jev wins if both providers are enabled.
fn build_jev_judge(project_name: &str, config: &Config) -> anyhow::Result<Option<Arc<dyn RelevanceJudge>>> {
    use graphrelay_core::config::JudgeProvider;
    match config.retrieval.judge_provider() {
        None => Ok(None),
        Some(JudgeProvider::Jev) => {
            let jev = &config.retrieval.jev;
            let api_key = jev
                .api_key
                .clone()
                .or_else(|| std::env::var("GRAPHRELAY_JEV_API_KEY").ok());
            let Some(api_key) = api_key.filter(|k| !k.is_empty()) else {
                tracing::warn!(
                    "[{project_name}] retrieval.jev.enabled is true but no api_key is set — \
                     Jev reranking disabled for this project, falling back to lexical+graph ranking only",
                );
                return Ok(None);
            };
            let judge = JevJudge::new(
                api_key,
                jev.base_url.clone(),
                jev.model.clone(),
                std::time::Duration::from_millis(jev.timeout_ms),
            )?;
            tracing::info!("[{project_name}] Jev reranking enabled (model={})", jev.model);
            Ok(Some(Arc::new(judge)))
        }
        Some(JudgeProvider::Laya) => {
            let laya = &config.retrieval.laya;
            // No key is the normal case: laya-serve only checks a bearer token when it was
            // started with LAYA_API_KEY.
            let api_key = laya
                .api_key
                .clone()
                .or_else(|| std::env::var("GRAPHRELAY_LAYA_API_KEY").ok());
            let judge = LayaJudge::new(
                api_key,
                laya.base_url.clone(),
                laya.model.clone(),
                laya.max_len,
                laya.concurrency,
                std::time::Duration::from_millis(laya.timeout_ms),
            )?;
            tracing::info!(
                "[{project_name}] Laya reranking enabled (url={}, top_n={}, weight={}, concurrency={})",
                laya.base_url, laya.top_n, laya.weight, laya.concurrency,
            );
            Ok(Some(Arc::new(judge)))
        }
    }
}

async fn health_handler(
    State(watch_status): State<Arc<RwLock<IndexStatus>>>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let status = watch_status.read().await;
    let ready = matches!(status.state, IndexState::Fresh);
    let code = if ready {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(serde_json::json!({
            "ready":            ready,
            "state":            status.state.as_str(),
            "total_symbols":    status.total_symbols,
            "total_files":      status.total_files,
            "semantic_pending": status.semantic_pending,
            "version":          env!("GRAPHRELAY_FULL_VERSION"),
        })),
    )
}
