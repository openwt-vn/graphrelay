pub mod routes;

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use graphrelay_core::{
    retrieval::{QueryPipeline, RetrievalQuery, SourceLoader},
    types::{IndexState, IndexStatus},
};
use graphrelay_store::GraphDb;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;

// ── Shared state ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ApiState {
    pub graph_db:      Arc<GraphDb>,
    pub source_loader: Arc<SourceLoader>,
    pub watch_status:  Arc<RwLock<IndexStatus>>,
    pub pipeline:      Arc<dyn QueryPipeline>,
}

// ── Response envelope ─────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub data: T,
    pub meta: ResponseMeta,
}

#[derive(Serialize)]
pub struct ResponseMeta {
    pub count:       Option<usize>,
    pub elapsed_ms:  u64,
    pub index_state: String,
}

impl<T: Serialize> ApiResponse<T> {
    fn new(data: T, count: Option<usize>, elapsed: Duration, state: &str) -> Self {
        Self {
            data,
            meta: ResponseMeta {
                count,
                elapsed_ms:  elapsed.as_millis() as u64,
                index_state: state.to_string(),
            },
        }
    }
}

// ── Error type ────────────────────────────────────────────────────────────────

struct ApiError {
    status:  axum::http::StatusCode,
    message: String,
}

impl ApiError {
    fn unavailable(msg: &str) -> Self {
        Self { status: axum::http::StatusCode::SERVICE_UNAVAILABLE, message: msg.to_string() }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.message });
        (self.status, Json(body)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self { status: axum::http::StatusCode::INTERNAL_SERVER_ERROR, message: e.into().to_string() }
    }
}

type ApiResult<T> = Result<(axum::http::StatusCode, Json<ApiResponse<T>>), ApiError>;

/// `Json<T>` alone always implies HTTP 200 —
/// every handler used to return 200 unconditionally even when
/// `IndexStatus.state` was `Stale`/`Error`, with only the (easy-to-miss)
/// `meta.index_state` field hinting at degraded data. Wrap the response with
/// this to surface it as the status code too, so a client that only checks
/// HTTP status (not response bodies) still finds out. `Indexing` is
/// deliberately left as 200 here — `search_handler` already has its own
/// harder-line "no data at all" 503 for `Indexing`/`Error`; this only
/// changes behavior for `Stale`, the gap `CLAUDE.md` named explicitly.
fn respond<T: Serialize>(
    data: T,
    count: Option<usize>,
    elapsed: Duration,
    state: IndexState,
) -> (axum::http::StatusCode, Json<ApiResponse<T>>) {
    let status = match state {
        IndexState::Stale | IndexState::Error => axum::http::StatusCode::SERVICE_UNAVAILABLE,
        IndexState::Fresh | IndexState::Indexing => axum::http::StatusCode::OK,
    };
    (status, Json(ApiResponse::new(data, count, elapsed, state.as_str())))
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn api_router(state: ApiState) -> Router {
    Router::new()
        .route("/api/graph",  get(graph_handler))
        .route("/api/search", get(search_handler))
        .route("/api/symbol/:id",            get(symbol_by_id_handler))
        .route("/api/symbol/by-name/:name", get(symbol_by_name_handler))
        .route("/api/file/*path",            get(file_handler))
        .route("/api/status",               get(status_handler))
        .route("/api/languages",            get(languages_handler))
        .route("/api/impact/:id",           get(impact_handler))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GraphParams {
    lang:  Option<String>,
    limit: Option<usize>,
    focus: Option<u64>,
}

#[derive(Serialize)]
struct GraphResponse {
    symbols: Vec<SymbolNode>,
    edges:   Vec<EdgeDto>,
}

#[derive(Serialize)]
struct SymbolNode {
    id:    u64,
    name:  String,
    kind:  String,
    file:  String,
    lines: String,
}

#[derive(Serialize)]
struct EdgeDto {
    from_id: u64,
    to_id:   u64,
    kind:    String,
}

async fn graph_handler(
    State(state): State<ApiState>,
    Query(params): Query<GraphParams>,
) -> ApiResult<GraphResponse> {
    let t = std::time::Instant::now();
    let limit = params.limit.unwrap_or(200).min(500);

    let symbols = if let Some(focus_id) = params.focus {
        let center = state.graph_db.symbol_by_id(focus_id)?
            .ok_or_else(|| anyhow::anyhow!("symbol not found: {focus_id}"))?;
        let mut neighbors = state.graph_db.neighbors(center.id, 1)?;
        neighbors.push(center);
        neighbors
    } else {
        state.graph_db.top_symbols_by_in_degree(limit, params.lang.as_deref())?
    };

    let ids: Vec<u64> = symbols.iter().map(|s| s.id).collect();
    let edges = state.graph_db.edges_between(&ids)?;

    let resp = GraphResponse {
        symbols: symbols.iter().map(|s| SymbolNode {
            id:    s.id,
            name:  s.name.clone(),
            kind:  format!("{:?}", s.kind),
            file:  s.file_path.display().to_string(),
            lines: format!("{}-{}", s.span.start_line, s.span.end_line),
        }).collect(),
        edges: edges.iter().filter_map(|e| {
            Some(EdgeDto {
                from_id: e.from_id,
                to_id:   e.to_id?,
                kind:    e.kind.as_str().to_string(),
            })
        }).collect(),
    };
    let count = resp.symbols.len();
    let index_state = state.watch_status.read().await.state;
    Ok(respond(resp, Some(count), t.elapsed(), index_state))
}

#[derive(Deserialize)]
struct SearchParams {
    q:    String,
    n:    Option<usize>,
    lang: Option<String>,
}

async fn search_handler(
    State(state): State<ApiState>,
    Query(params): Query<SearchParams>,
) -> ApiResult<Value> {
    let t = std::time::Instant::now();

    let index_state = {
        let status = state.watch_status.read().await;
        if matches!(status.state, IndexState::Indexing | IndexState::Error) {
            return Err(ApiError::unavailable("index not ready, retry shortly"));
        }
        status.state
    };

    let results = state.pipeline.query(RetrievalQuery {
        text:        params.q.clone(),
        top_k:       state.pipeline.candidate_top_k(),
        final_n:     params.n.unwrap_or_else(|| state.pipeline.final_top_n()).clamp(1, 50),
        expand_hops: 1,
        lang_filter: params.lang,
    }).await?;

    let data = serde_json::json!({
        "query":   params.q,
        "results": results.iter().map(|r| serde_json::json!({
            "rank":      r.rank,
            "score":     r.score,
            "symbol_id": r.symbol.id,
            "symbol":    r.symbol.qualified_name,
            "kind":      format!("{:?}", r.symbol.kind),
            "file":      r.file_path.display().to_string(),
            "lines":     format!("{}-{}", r.symbol.span.start_line, r.symbol.span.end_line),
            "snippet":   r.snippet,
            "callers":   r.callers.len(),
            "callees":   r.callees.len(),
        })).collect::<Vec<_>>(),
    });
    let count = results.len();
    Ok(respond(data, Some(count), t.elapsed(), index_state))
}

async fn symbol_by_id_handler(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
) -> ApiResult<Value> {
    let t = std::time::Instant::now();
    let sym = state.graph_db.symbol_by_id(id)?
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {id}"))?;
    symbol_detail_response(&state, sym, t).await
}

async fn symbol_by_name_handler(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> ApiResult<Value> {
    let t = std::time::Instant::now();
    let sym = state.graph_db.symbol_by_qualified(&name)?
        .or_else(|| state.graph_db.search_symbols_by_name(&name, 1).ok()?.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {name}"))?;
    symbol_detail_response(&state, sym, t).await
}

async fn symbol_detail_response(
    state: &ApiState,
    sym: graphrelay_core::types::Symbol,
    t: std::time::Instant,
) -> ApiResult<Value> {
    let index_state = state.watch_status.read().await.state;
    let callers = state.graph_db.callers_of(sym.id)?;
    let callees = state.graph_db.callees_of(sym.id)?;
    let snippet = state.source_loader.snippet(&sym).unwrap_or_default();

    let data = serde_json::json!({
        "id":             sym.id,
        "name":           sym.name,
        "qualified_name": sym.qualified_name,
        "kind":           format!("{:?}", sym.kind),
        "file":           sym.file_path.display().to_string(),
        "start_line":     sym.span.start_line,
        "end_line":       sym.span.end_line,
        "docstring":      sym.docstring,
        "definition":     snippet,
        "callers": callers.iter().map(|s| serde_json::json!({
            "id": s.id, "name": s.name, "qualified_name": s.qualified_name,
            "kind": format!("{:?}", s.kind),
        })).collect::<Vec<_>>(),
        "callees": callees.iter().map(|s| serde_json::json!({
            "id": s.id, "name": s.name, "qualified_name": s.qualified_name,
            "kind": format!("{:?}", s.kind),
        })).collect::<Vec<_>>(),
    });
    Ok(respond(data, None, t.elapsed(), index_state))
}

async fn file_handler(
    State(state): State<ApiState>,
    Path(path): Path<String>,
) -> ApiResult<Value> {
    let t = std::time::Instant::now();
    let index_state = state.watch_status.read().await.state;
    // Exact match first, then a suffix-match fallback — a URL path segment is
    // naturally repo-relative, and an exact-match-only lookup silently fails
    // for every caller that doesn't already know the stored absolute path
    // (see crates/graphrelay-mcp/src/tools/get_file_context.rs's resolve_file,
    // same underlying GraphDb::files_by_suffix fallback).
    let file = match state.graph_db.file_by_path(std::path::Path::new(&path))? {
        Some(f) => f,
        None => {
            let mut matches = state.graph_db.files_by_suffix(std::path::Path::new(&path))?;
            match matches.len() {
                1 => matches.remove(0),
                0 => return Err(anyhow::anyhow!("file not indexed: {path}").into()),
                n => return Err(anyhow::anyhow!(
                    "ambiguous path: {path} matches {n} indexed files — pass a longer suffix or the absolute path"
                ).into()),
            }
        }
    };
    let syms = state.graph_db.symbols_in_file(file.id)?;

    let data = serde_json::json!({
        "path":     file.path.to_string_lossy(),
        "language": file.language.as_str(),
        "symbols":  syms.iter().map(|s| serde_json::json!({
            "id":         s.id,
            "name":       s.name,
            "kind":       format!("{:?}", s.kind),
            "start_line": s.span.start_line,
            "end_line":   s.span.end_line,
        })).collect::<Vec<_>>(),
    });
    Ok(respond(data, Some(syms.len()), t.elapsed(), index_state))
}

async fn status_handler(
    State(state): State<ApiState>,
) -> Json<Value> {
    let status = state.watch_status.read().await;
    // Plan 12 C4: include SCIP status
    let scip_ingested_at    = state.graph_db.get_meta("scip_ingested_at").ok().flatten();
    let scip_symbols        = state.graph_db.get_meta("scip_symbols_enriched").ok().flatten()
        .and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    Json(serde_json::json!({
        "state":                  status.state.as_str(),
        "last_indexed":           status.last_indexed_at,
        "pending_files":          status.pending_count,
        "total_symbols":          status.total_symbols,
        "total_files":            status.total_files,
        "index_age_secs":         status.age_seconds(),
        "last_error":             status.last_error,
        "semantic_pending":       status.semantic_pending,
        "scip_present":           scip_ingested_at.is_some(),
        "scip_ingested_at":       scip_ingested_at,
        "scip_symbols_enriched":  scip_symbols,
        "jev_active":             state.pipeline.jev_active(),
        "judge_provider":         state.pipeline.judge_provider(),
    }))
}

async fn languages_handler(
    State(state): State<ApiState>,
) -> ApiResult<Value> {
    let t = std::time::Instant::now();
    let index_state = state.watch_status.read().await.state;
    let langs = state.graph_db.language_counts()?;
    let data = serde_json::json!({
        "languages": langs.iter().map(|(lang, count)| serde_json::json!({
            "language":   lang,
            "file_count": count,
        })).collect::<Vec<_>>(),
    });
    let count = langs.len();
    Ok(respond(data, Some(count), t.elapsed(), index_state))
}

#[derive(Deserialize)]
struct ImpactParams {
    depth: Option<u8>,
}

async fn impact_handler(
    State(state): State<ApiState>,
    Path(id): Path<u64>,
    Query(params): Query<ImpactParams>,
) -> ApiResult<Value> {
    let t = std::time::Instant::now();
    let depth = params.depth.unwrap_or(3);
    let impact = state.graph_db.impact_graph(id, depth)?;

    let data = serde_json::json!({
        "root_symbol_id": id,
        "depth":          depth,
        "affected_count": impact.affected.len(),
        "affected": impact.affected.iter().map(|n| serde_json::json!({
            "id":          n.symbol.id,
            "name":        n.symbol.qualified_name,
            "file":        n.path.display().to_string(),
            "kind":        format!("{:?}", n.symbol.kind),
            "edge_source": n.edge_source.as_str(),
            "hop_depth":   n.hop_depth,
        })).collect::<Vec<_>>(),
    });
    let count = impact.affected.len();
    let index_state = state.watch_status.read().await.state;
    Ok(respond(data, Some(count), t.elapsed(), index_state))
}
