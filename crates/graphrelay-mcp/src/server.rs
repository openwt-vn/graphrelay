use crate::schema::{JsonRpcRequest, JsonRpcResponse};
use crate::tools;
use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use graphrelay_core::{retrieval::{QueryPipeline, SourceLoader}, types::IndexStatus};
use graphrelay_store::GraphDb;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Application state visible to the MCP server.
#[derive(Clone)]
pub struct McpAppState {
    pub graph_db:      Arc<GraphDb>,
    pub source_loader: Arc<SourceLoader>,
    pub watch_status:  Arc<RwLock<IndexStatus>>,
    pub pipeline:      Arc<dyn QueryPipeline>,
}

pub fn mcp_router(state: McpAppState) -> Router {
    Router::new()
        .route("/mcp", post(handle_mcp))
        .with_state(state)
}

/// Protocol revisions this server speaks, newest first. The transport is
/// POST-only Streamable HTTP, which first appeared in 2025-03-26.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

async fn handle_mcp(State(state): State<McpAppState>, body: Bytes) -> Response {
    // Parsed by hand rather than with axum's `Json` extractor, whose rejections
    // are plain-text 4xx responses instead of JSON-RPC errors.
    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return Json(JsonRpcResponse::err(Some(Value::Null), -32700, &format!("parse error: {e}"))).into_response(),
    };
    let Some(obj) = value.as_object() else {
        return Json(JsonRpcResponse::err(Some(Value::Null), -32600, "invalid request: expected a single JSON-RPC object (batches are not supported)")).into_response();
    };
    // MCP spec: notifications have no `id` and must not receive a JSON-RPC
    // response. An explicit `"id": null` is still a request and gets one.
    let id = obj.get("id").cloned();
    let req: JsonRpcRequest = match serde_json::from_value(value.clone()) {
        Ok(r) => r,
        Err(e) => return Json(JsonRpcResponse::err(id.or(Some(Value::Null)), -32600, &format!("invalid request: {e}"))).into_response(),
    };
    if req.jsonrpc != "2.0" {
        return Json(JsonRpcResponse::err(id.or(Some(Value::Null)), -32600, "invalid request: jsonrpc must be \"2.0\"")).into_response();
    }
    let Some(id) = id else {
        return StatusCode::ACCEPTED.into_response();
    };
    Json(dispatch(&state, JsonRpcRequest { id: Some(id), ..req }).await).into_response()
}

pub async fn dispatch(state: &McpAppState, req: JsonRpcRequest) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize"  => handle_initialize(req.id, req.params.as_ref()),
        "tools/list"  => handle_tools_list(req.id),
        "tools/call"  => handle_tools_call(state, req).await,
        "ping"        => JsonRpcResponse::ok(req.id, serde_json::json!({})),
        other         => JsonRpcResponse::method_not_found(req.id, other),
    }
}

fn handle_initialize(id: Option<Value>, params: Option<&Value>) -> JsonRpcResponse {
    // Echo the client's revision when we support it, else offer our newest.
    let requested = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
    let version = requested
        .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0]);
    JsonRpcResponse::ok(
        id,
        serde_json::json!({
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name":    "graphrelay",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

/// Every description follows one shape: what it does / USE WHEN / DON'T USE /
/// FLOW / OUTPUT / LIMITS. Numeric caps come from the constants the handlers
/// enforce, so a description cannot drift from the code when a cap changes.
fn search_code_description() -> String {
    [
        "Find code you cannot name yet. Pass exactly one of `intent` or `implements`.".to_string(),
        "USE WHEN: you need to find where a behavior, rule or flow is implemented (`intent`), or every subclass or implementor of a named interface, trait, protocol or base class (`implements`).".to_string(),
        "DON'T USE: to list every literal occurrence of a string or regex (use Grep); for a symbol you can already name (use inspect with `symbol`); for who calls or depends on X (use trace_impact); for non-code files such as JSON fixtures, configs, templates and docs, which are not indexed (find them with Glob, search them with Grep).".to_string(),
        "FLOW: search_code, then inspect a symbol or file it returned, then trace_impact if callers matter. If the question has separate parts, make one call per part with different keywords; the same keywords return the same files.".to_string(),
        "QUERY: `intent` is keyword search (BM25 with stemming and camelCase/snake_case splitting) plus 1-hop call-graph expansion, not semantic matching. Use identifiers and domain words likely to appear in the code.".to_string(),
        format!(
            "OUTPUT (intent): `results` has one entry per file, best first. `source` holds only the matched symbols' line ranges, numbered like Read and separated by `...`; it is not the whole file. `source_truncated: true` means some matched ranges were cut or dropped. Read other ranges only for code outside these. `symbols[]` gives rank, `lines` (1-based), and callers/callees (first {} names; `*_count` is the number found). A `note` may say results cluster in one area. `possibly_stale: true` means the file changed since indexing: Read it before relying on it.",
            tools::CALLER_CALLEE_DISPLAY_CAP
        ),
        format!(
            "OUTPUT (implements): transitive Inherits/Implements edges only, up to {} with short snippets; check `truncated`, `total_found` and `resolution_note`. `top_n` and `language` are ignored.",
            tools::find_implementations::MAX_SHOWN
        ),
        "LIMITS: misses code that shares no words with your query; results are a ranked sample, not a complete list. A result shows what exists, never what is absent: for \"is there X\", \"what is the default\" or \"is Y configured\", also Grep the settings, hooks.py and sibling files before concluding, and for questions that list every place a pattern occurs, Grep to enumerate the sites.".to_string(),
    ]
    .join("\n")
}

fn inspect_description() -> String {
    let (small, large) = (tools::response_budget(0), tools::response_budget(u64::MAX));
    [
        "Look up a symbol or path you can already name. Pass exactly one of `symbol` or `path`.".to_string(),
        "USE WHEN: a tool result, error message or the question gives you a function, class or method name (`symbol`), a file (`path`), or a directory you want an overview of (`path`).".to_string(),
        "DON'T USE: to discover where something lives (use search_code); for transitive dependents (use trace_impact; `callers` here are direct calls only); for files that are not indexed, such as config, docs or lockfiles (use Read).".to_string(),
        "FLOW: usually after search_code or trace_impact. For a file search_code already showed, pass `focus_line` taken from its `lines` to see the code around that symbol.".to_string(),
        format!(
            "`symbol`: returns `definition` (capped at about {}-{} characters by repo size; see `definition_truncated`), `docstring`, `file`, `lines` and direct `callers`/`callees` (first {} names; `*_count` is the total, counting stops at about 200). Accepts a plain name, a path::name as other tools print it, or a dotted name (last segment used). If several symbols share the name, the one with the most callers is picked and `resolution_note` says so; check `file`.",
            large.per_item_chars, small.per_item_chars, tools::CALLER_CALLEE_DISPLAY_CAP
        ),
        format!(
            "`path` naming one indexed file: line-numbered text capped at about {}-{} characters by repo size, centered on `focus_line` if given. If `source_truncated` is true, call again with `focus_line` or Read the exact range; if false, you have the whole file, so do not Read it again. `include_source: false` returns only a per-symbol signature list. `path` naming a directory: file count, symbol kinds and the most-referenced symbols. An ambiguous path is an error that lists the candidates.",
            large.single_file_chars, small.single_file_chars
        ),
        "`possibly_stale: true` means the file changed since indexing: Read it.".to_string(),
    ]
    .join("\n")
}

fn trace_impact_description() -> String {
    [
        "List the symbols that depend on a given symbol: its callers, their callers, and so on (reverse direction only).".to_string(),
        "USE WHEN: what calls X, what breaks if I change X, which entry points reach X; before changing a signature.".to_string(),
        "DON'T USE: for what X itself calls (inspect `symbol` gives `callees`); to find code by topic (use search_code); for string mentions such as config keys, templates or reflection (use Grep).".to_string(),
        "FLOW: one call with `depth` set to the number of hops the question asks for (1 = direct callers). Results carry no source and no call-site lines: to see how a dependent uses X, Grep X's name in that `file`, or inspect that symbol.".to_string(),
        format!(
            "OUTPUT: `affected[]` sorted by `hop_depth`; each item has `symbol`, `file`, `lines` (the dependent's own range, 1-based), `kind`, `reason` and `edge_source` (syntax = tree-sitter, semantic = compiler-backed SCIP). Beyond hop 1, `via` names one hop-closer symbol on a shortest path (others may exist); it is null at hop 1. Functions and methods follow Calls edges only; types also follow Inherits, Implements and References (see `edge_kinds`). At most {} items are listed: if `truncated` is true, see `note`; `affected_count_is_lower_bound: true` means the walk stopped early. If `resolution_note` is set, the name matched several symbols and the most-called one was traced; check `root_symbol`.",
            tools::trace_impact::MAX_AFFECTED
        ),
        "LIMITS: calls the indexer could not resolve (dynamic dispatch, interface or callback calls, functions passed as values, a name defined several times) have no edge and are missing. For such names a small or empty result is not proof of no callers; confirm with Grep.".to_string(),
    ]
    .join("\n")
}

fn watch_status_description() -> String {
    [
        "Report index health. No arguments.",
        "USE WHEN: a result is empty, misses a file you know exists, or is marked `possibly_stale`, or after you edit files you are about to query. Not needed at the start of a task.",
        "DON'T USE: as a routine first call (a fresh index is the normal case), or to look for code (use search_code).",
        "OUTPUT: `state` is `fresh`; `indexing` (wait and retry; `pending_files` and `semantic_pending` count files not yet searchable); `stale` (some index writes failed and part of the index is behind disk; see `last_error`); or `error` (indexing failed; see `last_error`; retrying will not help). Also `total_files`, `total_symbols`, `index_age_secs`, `jev_active`/`judge_provider` (an optional rerank judge, usually off) and `scip_*` (compiler-backed edges merged).",
        "If `state` is `fresh`, an empty result means the indexed code has no match: try other keywords or Grep.",
    ]
    .join("\n")
}

fn handle_tools_list(id: Option<Value>) -> JsonRpcResponse {
    let tools = vec![
        serde_json::json!({
            "name": "search_code",
            "description": search_code_description(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "intent":     { "type": "string",  "description": "Keywords and identifiers describing the code to find (keyword-matched, not semantic). Exactly one of intent/implements." },
                    "implements": { "type": "string",  "description": "Name of an interface, trait, base class or protocol (plain name or path::name). Exactly one of intent/implements." },
                    "top_n":      { "type": "integer", "description": format!("Max matched symbols to return, grouped by file (default 8, max {}). intent only.", tools::find_relevant_code::MAX_TOP_N), "default": 8, "minimum": 1, "maximum": tools::find_relevant_code::MAX_TOP_N },
                    "language":   { "type": "string",  "description": "intent only: keep one language, lowercase as indexed, e.g. python, go, rust, typescript." }
                },
                "required": []
            }
        }),
        serde_json::json!({
            "name": "inspect",
            "description": inspect_description(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol":         { "type": "string",  "description": "Plain name, path::name as other tools print it, or a dotted name (last segment used). Exactly one of symbol/path required." },
                    "path":           { "type": "string",  "description": "A file's path (absolute, or a repo-relative suffix that uniquely identifies one indexed file) or a directory prefix (e.g. 'src/auth'). Exactly one of symbol/path required." },
                    "focus_line":     { "type": "integer", "description": "1-based line to center on (same numbering as `source` and every `lines` field; use the start of a `lines` range from search_code). path mode, single file only.", "minimum": 1 },
                    "include_source": { "type": "boolean", "description": "path mode, single file only: return the file's text (default) instead of just the per-symbol signature list. Pass false for the cheaper signature list.", "default": true },
                    "max_symbols":    { "type": "integer", "description": "path mode, directory only: max key symbols to return (default 10, max 50).", "default": 10, "minimum": 1, "maximum": 50 }
                },
                "required": []
            }
        }),
        serde_json::json!({
            "name": "trace_impact",
            "description": trace_impact_description(),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Plain name, path::name as other tools print it, or a dotted name (last segment used)." },
                    "depth":  { "type": "integer", "description": format!("Hops to follow (1 = direct dependents; default 3, clamped to 1-{}). Use the smallest depth the question needs; hub symbols truncate.", tools::trace_impact::MAX_DEPTH), "default": 3, "minimum": 1, "maximum": tools::trace_impact::MAX_DEPTH }
                },
                "required": ["symbol"]
            }
        }),
        serde_json::json!({
            "name": "watch_status",
            "description": watch_status_description(),
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    ];
    JsonRpcResponse::ok(id, serde_json::json!({ "tools": tools }))
}

/// Opt-in (`GRAPHRELAY_INTENT_LOG=<path>`): intents can contain private text, so
/// nothing is persisted unless the operator asks. Needed to collect real agent
/// queries for the fuzzy-search revisit trigger in `plans/24-*.md`.
fn log_search_intent(arguments: &Value) {
    use std::io::Write;
    let Some(path) = std::env::var_os("GRAPHRELAY_INTENT_LOG") else { return };
    let line = serde_json::json!({
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "intent": arguments.get("intent"),
        "implements": arguments.get("implements"),
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

async fn handle_tools_call(state: &McpAppState, req: JsonRpcRequest) -> JsonRpcResponse {
    let params = match req.params {
        Some(p) => p,
        None    => return JsonRpcResponse::invalid_params(req.id, "missing params"),
    };

    let name: &str = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None    => return JsonRpcResponse::invalid_params(req.id, "missing tool name"),
    };

    let arguments = params.get("arguments").cloned().unwrap_or(Value::Object(Default::default()));

    // Instrumentation for the tool-surface mis-pick eval that motivated
    // Plan 23's consolidation (7 tools → 4: search_code, inspect,
    // trace_impact, watch_status) — logs which tool name was called per
    // incoming query so real usage can be measured going forward.
    tracing::info!(target: "graphrelay_mcp::tool_call", tool = name, "mcp tool called");

    if name == "search_code" {
        log_search_intent(&arguments);
    }

    let result = match name {
        "search_code" => {
            let args: tools::search_code::Args = match serde_json::from_value(arguments) {
                Ok(a)  => a,
                Err(e) => return JsonRpcResponse::invalid_params(req.id, &e.to_string()),
            };
            tools::search_code::handle(state, args).await
        }
        "inspect" => {
            let args: tools::inspect::Args = match serde_json::from_value(arguments) {
                Ok(a)  => a,
                Err(e) => return JsonRpcResponse::invalid_params(req.id, &e.to_string()),
            };
            tools::inspect::handle(state, args).await
        }
        "trace_impact" => {
            let args: tools::trace_impact::Args = match serde_json::from_value(arguments) {
                Ok(a)  => a,
                Err(e) => return JsonRpcResponse::invalid_params(req.id, &e.to_string()),
            };
            tools::trace_impact::handle(state, args).await
        }
        "watch_status" => tools::watch_status::handle(state).await,
        other => {
            return JsonRpcResponse::invalid_params(req.id, &format!("unknown tool: {other}"))
        }
    };

    match result {
        Ok(value) => JsonRpcResponse::ok(req.id, serde_json::json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "isError": false
        })),
        Err(e) => JsonRpcResponse::ok(req.id, serde_json::json!({
            "content": [{ "type": "text", "text": e.to_string() }],
            "isError": true
        })),
    }
}
