# Plan 06 — MCP Server (HTTP Transport)

## Goal
Implement an MCP-over-HTTP server at `/mcp` that exposes 5 agent-shaped tools. Each tool is designed to match what a coding agent actually asks — one call, one complete answer — not generic search primitives.

## Success Criteria
- All 5 tools respond correctly to JSON-RPC 2.0 requests
- Tool schemas are auto-discoverable via `tools/list`
- `find_relevant_code` returns code + graph context in a single response
- Response time < 200 ms for all tools (excluding initial index build)
- Compatible with Claude Code, Codex, and Cursor MCP clients

---

## 1. MCP Protocol Basics (HTTP Transport)

The MCP HTTP transport uses JSON-RPC 2.0 over plain HTTP POST:

```
POST /mcp
Content-Type: application/json

{
  "jsonrpc": "2.0",
  "id": "1",
  "method": "tools/call",
  "params": {
    "name": "find_relevant_code",
    "arguments": { "intent": "how does search indexing work?" }
  }
}
```

Session state is managed by the client via a `mcp-session-id` header. Since codegraph is stateless between calls (all state in SQLite + LanceDB), we keep sessions minimal.

---

## 2. MCP Router (axum)

```rust
// crates/codegraph-mcp/src/server.rs

use axum::{Router, routing::post, extract::State, Json};
use serde_json::Value;

pub fn mcp_router(app_state: Arc<AppState>) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(app_state)
}

async fn handle_mcp(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> impl axum::response::IntoResponse {
    let result = dispatch(&state, req).await;
    Json(result)
}

async fn dispatch(state: &AppState, req: JsonRpcRequest) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize"   => handle_initialize(&req),
        "tools/list"   => handle_tools_list(&req),
        "tools/call"   => handle_tools_call(state, &req).await,
        "ping"         => JsonRpcResponse::ok(req.id, serde_json::json!({})),
        other          => JsonRpcResponse::method_not_found(req.id, other),
    }
}
```

---

## 3. JSON-RPC Types

```rust
// crates/codegraph-mcp/src/schema.rs

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id:      Option<Value>,
    pub method:  String,
    pub params:  Option<Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id:      Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result:  Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error:   Option<JsonRpcError>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code:    i32,
    pub message: String,
    pub data:    Option<Value>,
}

impl JsonRpcResponse {
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }
    pub fn err(id: Option<Value>, code: i32, msg: &str) -> Self {
        Self { jsonrpc: "2.0".into(), id, result: None,
               error: Some(JsonRpcError { code, message: msg.into(), data: None }) }
    }
    pub fn method_not_found(id: Option<Value>, method: &str) -> Self {
        Self::err(id, -32601, &format!("method not found: {method}"))
    }
}
```

---

## 4. `initialize` Response

```rust
fn handle_initialize(req: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse::ok(req.id.clone(), serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name":    "codegraph",
            "version": env!("CARGO_PKG_VERSION")
        }
    }))
}
```

---

## 5. Tool Registry and `tools/list`

```rust
fn handle_tools_list(req: &JsonRpcRequest) -> JsonRpcResponse {
    let tools = vec![
        tool_schema_find_relevant_code(),
        tool_schema_trace_impact(),
        tool_schema_explain_symbol(),
        tool_schema_get_file_context(),
        tool_schema_watch_status(),
    ];
    JsonRpcResponse::ok(req.id.clone(), serde_json::json!({ "tools": tools }))
}

fn tool_schema_find_relevant_code() -> Value {
    serde_json::json!({
        "name": "find_relevant_code",
        "description": "Find code relevant to a natural language intent. Returns ranked snippets with graph context (callers, callees) in a single response.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "intent": {
                    "type": "string",
                    "description": "Natural language description of what you're looking for."
                },
                "top_n": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default 5).",
                    "default": 5
                },
                "language": {
                    "type": "string",
                    "description": "Filter by programming language (optional)."
                }
            },
            "required": ["intent"]
        }
    })
}

fn tool_schema_trace_impact() -> Value {
    serde_json::json!({
        "name": "trace_impact",
        "description": "Find all symbols that depend on (call or import) a given symbol. Use this to understand blast radius before changing a function or type.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Qualified symbol name, e.g. 'src/query/engine.rs::QueryEngine::execute', or a plain function name."
                },
                "depth": {
                    "type": "integer",
                    "description": "Traversal depth (default 3).",
                    "default": 3
                }
            },
            "required": ["symbol"]
        }
    })
}

fn tool_schema_explain_symbol() -> Value {
    serde_json::json!({
        "name": "explain_symbol",
        "description": "Return definition, docstring, callers, and callees for a symbol in one payload.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Qualified or plain symbol name."
                }
            },
            "required": ["symbol"]
        }
    })
}

fn tool_schema_get_file_context() -> Value {
    serde_json::json!({
        "name": "get_file_context",
        "description": "Return the symbols defined in a file and their relationships, optionally focused around a specific line.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative file path."
                },
                "focus_line": {
                    "type": "integer",
                    "description": "Line number to focus on (optional). Returns the symbol that contains this line."
                }
            },
            "required": ["path"]
        }
    })
}

fn tool_schema_watch_status() -> Value {
    serde_json::json!({
        "name": "watch_status",
        "description": "Return the current index freshness. Use this to check if the index is stale before running other tools.",
        "inputSchema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    })
}
```

---

## 6. Tool Handler: `find_relevant_code`

```rust
// crates/codegraph-mcp/src/tools/find_relevant_code.rs

#[derive(Deserialize)]
pub struct FindRelevantCodeArgs {
    pub intent:   String,
    pub top_n:    Option<usize>,
    pub language: Option<String>,
}

pub async fn handle(state: &AppState, args: FindRelevantCodeArgs) -> anyhow::Result<Value> {
    let results = state.pipeline.query(RetrievalQuery {
        text:        args.intent.clone(),
        top_k:       20,
        final_n:     args.top_n.unwrap_or(5),
        expand_hops: 1,
        lang_filter: args.language,
    }).await?;

    let output = results.iter().map(|r| serde_json::json!({
        "rank":           r.rank,
        "score":          r.score,
        "symbol":         r.symbol.qualified_name,
        "kind":           format!("{:?}", r.symbol.kind),
        "file":           r.file_path.display().to_string(),
        "lines":          format!("{}-{}", r.symbol.span.start_line, r.symbol.span.end_line),
        "snippet":        r.snippet,
        "callers":        r.callers.iter().map(|s| &s.qualified_name).collect::<Vec<_>>(),
        "callees":        r.callees.iter().map(|s| &s.qualified_name).collect::<Vec<_>>(),
        "docstring":      r.symbol.docstring,
    })).collect::<Vec<_>>();

    Ok(serde_json::json!({
        "query":   args.intent,
        "count":   output.len(),
        "results": output,
    }))
}
```

---

## 7. Tool Handler: `trace_impact`

```rust
#[derive(Deserialize)]
pub struct TraceImpactArgs {
    pub symbol: String,
    pub depth:  Option<u8>,
}

pub async fn handle(state: &AppState, args: TraceImpactArgs) -> anyhow::Result<Value> {
    let sym = resolve_symbol(&state.graph_db, &args.symbol)?;
    let impact = trace_impact(&state.graph_db, sym.id, args.depth.unwrap_or(3))?;

    Ok(serde_json::json!({
        "root_symbol": sym.qualified_name,
        "affected_count": impact.affected.len(),
        "affected": impact.affected.iter().map(|n| serde_json::json!({
            "symbol": n.symbol.qualified_name,
            "file":   n.path.display().to_string(),
            "kind":   format!("{:?}", n.symbol.kind),
        })).collect::<Vec<_>>(),
    }))
}
```

---

## 8. Tool Handler: `explain_symbol`

```rust
pub async fn handle(state: &AppState, args: ExplainSymbolArgs) -> anyhow::Result<Value> {
    let sym      = resolve_symbol(&state.graph_db, &args.symbol)?;
    let callers  = state.graph_db.callers_of(sym.id)?;
    let callees  = state.graph_db.callees_of(sym.id)?;
    let snippet  = state.source_loader.snippet(&sym)?;

    Ok(serde_json::json!({
        "symbol":     sym.qualified_name,
        "kind":       format!("{:?}", sym.kind),
        "file":       sym.file_path.display().to_string(),
        "lines":      format!("{}-{}", sym.span.start_line, sym.span.end_line),
        "docstring":  sym.docstring,
        "definition": snippet,
        "callers":    callers.iter().map(|s| s.qualified_name.as_str()).collect::<Vec<_>>(),
        "callees":    callees.iter().map(|s| s.qualified_name.as_str()).collect::<Vec<_>>(),
    }))
}
```

---

## 9. Tool Handler: `get_file_context`

```rust
pub async fn handle(state: &AppState, args: GetFileContextArgs) -> anyhow::Result<Value> {
    let file   = state.graph_db.file_by_path(Path::new(&args.path))?
        .ok_or_else(|| anyhow!("file not indexed: {}", args.path))?;
    let syms   = state.graph_db.symbols_in_file(file.id)?;

    let focused = if let Some(line) = args.focus_line {
        syms.iter().find(|s| s.span.start_line <= line && line <= s.span.end_line)
    } else {
        None
    };

    Ok(serde_json::json!({
        "path":     args.path,
        "language": file.language,
        "symbol_count": syms.len(),
        "focused_symbol": focused.map(|s| serde_json::json!({
            "name":  s.qualified_name,
            "kind":  format!("{:?}", s.kind),
            "lines": format!("{}-{}", s.span.start_line, s.span.end_line),
        })),
        "symbols": syms.iter().map(|s| serde_json::json!({
            "name":  s.qualified_name,
            "kind":  format!("{:?}", s.kind),
            "lines": format!("{}-{}", s.span.start_line, s.span.end_line),
        })).collect::<Vec<_>>(),
    }))
}
```

---

## 10. Tool Handler: `watch_status`

```rust
pub async fn handle(state: &AppState) -> anyhow::Result<Value> {
    let status = state.watch_status.read().await;
    Ok(serde_json::json!({
        "state":          status.state.as_str(),   // "fresh" | "indexing" | "stale" | "error"
        "last_indexed":   status.last_indexed_at,
        "pending_files":  status.pending_count,
        "total_symbols":  status.total_symbols,
        "total_files":    status.total_files,
        "index_age_secs": status.age_seconds(),
    }))
}
```

---

## 11. Symbol Resolution Helper

Many tools accept either a qualified name or a plain function name:

```rust
fn resolve_symbol(graph_db: &GraphDb, input: &str) -> anyhow::Result<Symbol> {
    // Try qualified name first
    if let Some(sym) = graph_db.symbol_by_qualified(input)? {
        return Ok(sym);
    }
    // Fall back to name search, take the highest-in-degree match
    let candidates = graph_db.search_symbols_by_name(input, 10)?;
    candidates.into_iter()
        .max_by_key(|s| graph_db.in_degree(s.id).unwrap_or(0))
        .ok_or_else(|| anyhow!("symbol not found: {input}"))
}
```

---

## 12. Error Handling

MCP errors map to JSON-RPC error codes:

| Situation | JSON-RPC code | Message |
|-----------|--------------|---------|
| Symbol not found | -32602 (invalid params) | "symbol not found: {name}" |
| Index not ready | -32000 (server error) | "index is still building" |
| Store error | -32000 | "internal error: {detail}" |
| Unknown method | -32601 | "method not found: {method}" |

---

## 13. Claude Code Integration

Add to `.claude/config.json` (or user-level MCP config):
```json
{
  "mcpServers": {
    "codegraph": {
      "type": "http",
      "url": "http://localhost:3030/mcp"
    }
  }
}
```

---

## 14. Implementation Steps

1. Define `JsonRpcRequest` / `JsonRpcResponse` types
2. Implement `handle_initialize()` and `handle_tools_list()` with all 5 schemas
3. Implement `handle_tools_call()` dispatcher
4. Implement each tool handler (5 handlers)
5. Implement `resolve_symbol()` helper
6. Wire mcp_router into `main.rs`
7. Manual test with `curl` / MCP inspector
8. Integration test: `tools/list` returns all 5 tools; `find_relevant_code` on fixture repo returns rank-1 = expected symbol
9. Test error cases: unknown symbol, empty index

---

## 15. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| MCP protocol version | 2024-11-05 vs latest | Use `2024-11-05` — most widely supported by Claude Code today |
| Session management | Stateless vs session tokens | Stateless — codegraph has no per-session state |
| CORS | Allow all origins | Yes, since it runs on localhost only |
| Auth | None | None — localhost only; add opt-in token in v2 |
| Streaming responses | SSE / chunked | Not needed for v1; add SSE for `trace_impact` on large graphs in v2 |
