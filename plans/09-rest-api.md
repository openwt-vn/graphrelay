# Plan 09 — REST API

## Goal
Implement the REST API at `/api/*` that backs the Web UI and exposes raw graph + search data for external tooling. The API must be fast, have consistent JSON shapes, and support the graph size management strategies defined in Plan 08.

## Success Criteria
- All endpoints respond in < 50 ms for typical queries (cached source, warm DB)
- Consistent JSON envelope: `{ data: ..., meta: { ... } }` for all list responses
- CORS enabled (so the UI at the same origin, and external local tools, can call it)
- No auth required (localhost-only service)
- Endpoints are self-describing via OpenAPI-compatible shape

---

## 1. Route Table

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/graph` | Full or filtered symbol graph (nodes + edges) |
| GET | `/api/search` | Hybrid semantic search (delegates to retrieval pipeline) |
| GET | `/api/symbol/:id` | Symbol detail: definition, callers, callees, snippet |
| GET | `/api/symbol/by-name/:name` | Symbol lookup by name or qualified name |
| GET | `/api/file/:path` | All symbols in a file |
| GET | `/api/status` | Index freshness + counts |
| GET | `/api/languages` | List of indexed languages + file counts |
| GET | `/api/impact/:id` | Blast-radius graph for a symbol (alias for trace_impact) |

---

## 2. axum Router

```rust
// crates/codegraph-api/src/routes/mod.rs

use axum::{Router, routing::get, extract::{Query, Path, State}};

pub fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/graph",              get(graph::handler))
        .route("/api/search",             get(search::handler))
        .route("/api/symbol/:id",         get(symbol::handler_by_id))
        .route("/api/symbol/by-name/:name", get(symbol::handler_by_name))
        .route("/api/file/*path",         get(file::handler))
        .route("/api/status",             get(status::handler))
        .route("/api/languages",          get(languages::handler))
        .route("/api/impact/:id",         get(impact::handler))
        .with_state(state)
        .layer(tower_http::cors::CorsLayer::permissive())
}
```

---

## 3. Shared Response Envelope

```rust
// crates/codegraph-api/src/models.rs

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub data: T,
    pub meta: ResponseMeta,
}

#[derive(Serialize)]
pub struct ResponseMeta {
    pub count:         Option<usize>,
    pub elapsed_ms:    u64,
    pub index_state:   String,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn new(data: T, count: Option<usize>, elapsed: std::time::Duration, state: &str) -> Self {
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

pub type ApiResult<T> = Result<axum::Json<ApiResponse<T>>, ApiError>;

#[derive(Debug)]
pub struct ApiError(anyhow::Error);

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({ "error": self.0.to_string() });
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(body)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self { Self(e.into()) }
}
```

---

## 4. `GET /api/graph`

```rust
// crates/codegraph-api/src/routes/graph.rs

#[derive(Deserialize)]
pub struct GraphQuery {
    pub lang:    Option<String>,
    pub limit:   Option<usize>,    // default 200, max 500
    pub focus:   Option<u64>,      // symbol_id — return 1-hop neighborhood only
}

#[derive(Serialize)]
pub struct GraphResponse {
    pub symbols: Vec<SymbolNode>,
    pub edges:   Vec<EdgeDto>,
}

#[derive(Serialize)]
pub struct SymbolNode {
    pub id:    u64,
    pub name:  String,
    pub kind:  String,
    pub file:  String,
    pub lines: String,
}

#[derive(Serialize)]
pub struct EdgeDto {
    pub from_id: u64,
    pub to_id:   u64,
    pub kind:    String,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GraphQuery>,
) -> ApiResult<GraphResponse> {
    let t = std::time::Instant::now();

    let limit = params.limit.unwrap_or(200).min(500);

    let symbols = if let Some(focus_id) = params.focus {
        // 1-hop neighborhood
        let center = state.graph_db.symbol_by_id(focus_id)?
            .ok_or_else(|| anyhow::anyhow!("symbol not found"))?;
        let mut neighbors = state.graph_db.neighbors(focus_id, 1)?;
        neighbors.push(center);
        neighbors
    } else {
        // Top-N by in-degree, optionally filtered by language
        state.graph_db.top_symbols_by_in_degree(limit, params.lang.as_deref())?
    };

    let symbol_ids: Vec<u64> = symbols.iter().map(|s| s.id).collect();
    let edges = state.graph_db.edges_between(&symbol_ids)?;

    let response = GraphResponse {
        symbols: symbols.iter().map(|s| SymbolNode {
            id:    s.id,
            name:  s.name.clone(),
            kind:  format!("{:?}", s.kind),
            file:  s.file_path.display().to_string(),
            lines: format!("{}-{}", s.span.start_line, s.span.end_line),
        }).collect(),
        edges: edges.iter().map(|e| EdgeDto {
            from_id: e.from_id,
            to_id:   e.to_id,
            kind:    format!("{:?}", e.kind),
        }).collect(),
    };

    let count = response.symbols.len();
    Ok(axum::Json(ApiResponse::new(
        response, Some(count), t.elapsed(),
        &state.watch_status.read().await.as_str()
    )))
}
```

---

## 5. `GET /api/search`

```rust
// crates/codegraph-api/src/routes/search.rs

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q:    String,
    pub n:    Option<usize>,       // default 5
    pub lang: Option<String>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> ApiResult<serde_json::Value> {
    let t = std::time::Instant::now();

    let results = state.pipeline.query(RetrievalQuery {
        text:        params.q.clone(),
        top_k:       20,
        final_n:     params.n.unwrap_or(5),
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
    Ok(axum::Json(ApiResponse::new(data, Some(count), t.elapsed(), "fresh")))
}
```

---

## 6. `GET /api/symbol/:id`

```rust
// crates/codegraph-api/src/routes/symbol.rs

pub async fn handler_by_id(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
) -> ApiResult<serde_json::Value> {
    let t   = std::time::Instant::now();
    let sym = state.graph_db.symbol_by_id(id)?
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {id}"))?;

    symbol_response(&state, sym, t).await
}

pub async fn handler_by_name(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> ApiResult<serde_json::Value> {
    let t   = std::time::Instant::now();
    let sym = state.graph_db.symbol_by_qualified(&name)?
        .or_else(|| state.graph_db.search_symbols_by_name(&name, 1).ok()?.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("symbol not found: {name}"))?;

    symbol_response(&state, sym, t).await
}

async fn symbol_response(
    state: &AppState,
    sym: Symbol,
    t: std::time::Instant,
) -> ApiResult<serde_json::Value> {
    let callers = state.graph_db.callers_of(sym.id)?;
    let callees = state.graph_db.callees_of(sym.id)?;
    let snippet = state.source_loader.snippet(&sym)?;

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
            "id": s.id, "name": s.name, "qualified_name": s.qualified_name, "kind": format!("{:?}", s.kind)
        })).collect::<Vec<_>>(),
        "callees": callees.iter().map(|s| serde_json::json!({
            "id": s.id, "name": s.name, "qualified_name": s.qualified_name, "kind": format!("{:?}", s.kind)
        })).collect::<Vec<_>>(),
    });

    Ok(axum::Json(ApiResponse::new(data, None, t.elapsed(), "fresh")))
}
```

---

## 7. `GET /api/file/*path`

```rust
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> ApiResult<serde_json::Value> {
    let t    = std::time::Instant::now();
    let file = state.graph_db.file_by_path(Path::new(&path))?
        .ok_or_else(|| anyhow::anyhow!("file not indexed: {path}"))?;
    let syms = state.graph_db.symbols_in_file(file.id)?;

    let data = serde_json::json!({
        "path":     path,
        "language": file.language,
        "symbols":  syms.iter().map(|s| serde_json::json!({
            "id":   s.id,
            "name": s.name,
            "kind": format!("{:?}", s.kind),
            "start_line": s.span.start_line,
            "end_line":   s.span.end_line,
        })).collect::<Vec<_>>(),
    });

    Ok(axum::Json(ApiResponse::new(data, Some(syms.len()), t.elapsed(), "fresh")))
}
```

---

## 8. `GET /api/status`

```rust
pub async fn handler(State(state): State<Arc<AppState>>) -> axum::Json<serde_json::Value> {
    let status = state.watch_status.read().await;
    axum::Json(serde_json::json!({
        "state":          status.as_str(),
        "last_indexed":   status.last_indexed_at,
        "pending_files":  status.pending_count,
        "total_symbols":  status.total_symbols,
        "total_files":    status.total_files,
        "index_age_secs": status.age_seconds(),
        "last_error":     status.last_error,
    }))
}
```

---

## 9. `GET /api/languages`

```rust
pub async fn handler(State(state): State<Arc<AppState>>) -> ApiResult<serde_json::Value> {
    let langs = state.graph_db.language_counts()?;
    let data  = serde_json::json!({
        "languages": langs.iter().map(|(lang, count)| serde_json::json!({
            "language": lang,
            "file_count": count,
        })).collect::<Vec<_>>(),
    });
    Ok(axum::Json(ApiResponse::new(data, Some(langs.len()), Duration::ZERO, "fresh")))
}
```

Requires one new SQL query:
```sql
SELECT language, COUNT(*) as cnt FROM files GROUP BY language ORDER BY cnt DESC;
```

---

## 10. `GET /api/impact/:id`

```rust
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<u64>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<serde_json::Value> {
    let depth  = params.get("depth").and_then(|d| d.parse::<u8>().ok()).unwrap_or(3);
    let impact = trace_impact(&state.graph_db, id, depth)?;

    let data = serde_json::json!({
        "root_symbol_id": id,
        "depth":          depth,
        "affected_count": impact.affected.len(),
        "affected": impact.affected.iter().map(|n| serde_json::json!({
            "id":   n.symbol.id,
            "name": n.symbol.qualified_name,
            "file": n.path.display().to_string(),
            "kind": format!("{:?}", n.symbol.kind),
        })).collect::<Vec<_>>(),
    });

    Ok(axum::Json(ApiResponse::new(data, Some(impact.affected.len()), Duration::ZERO, "fresh")))
}
```

---

## 11. Error Codes

| HTTP Status | When |
|-------------|------|
| 200 | Success |
| 400 | Missing required query param (e.g., `q`) |
| 404 | Symbol / file not found |
| 503 | Index not yet ready (state = "indexing") |
| 500 | Internal error |

---

## 12. Implementation Steps

1. Define `ApiResponse<T>`, `ResponseMeta`, `ApiError` in `models.rs`
2. Implement `api_router()` with all routes
3. Implement each route handler (8 handlers)
4. Add `CorsLayer::permissive()` middleware
5. Wire `api_router` into `main.rs` alongside MCP router and UI router
6. Integration test each endpoint against a fixture index
7. Test CORS: verify `Access-Control-Allow-Origin: *` header is present
8. Benchmark: `/api/search` < 50 ms on warm cache

---

## 13. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Response envelope | Flat / `{ data, meta }` | `{ data, meta }` — easier for UI to read timing/state |
| CORS policy | Permissive / origin-restricted | Permissive — localhost service, no sensitive data |
| Pagination | Cursor / offset / none | None for v1 — graph endpoints have `limit` param |
| Error format | Plain string / `{ error, code }` | `{ "error": "message" }` — minimal, readable |
| `/api/graph` default limit | 100 / 200 / 500 | 200 — renders fine in Cytoscape on most machines |
