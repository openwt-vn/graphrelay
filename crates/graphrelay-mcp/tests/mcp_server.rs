use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use graphrelay_core::{
    retrieval::{QueryPipeline, RetrievalQuery, RetrievalResult, SourceLoader},
    types::{IndexState, IndexStatus, Language, Span, Symbol, SymbolId, SymbolKind},
};
use graphrelay_mcp::{
    mcp_router, McpAppState,
    schema::JsonRpcRequest,
    server::dispatch,
    tools,
};
use graphrelay_store::GraphDb;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tower::util::ServiceExt; // oneshot

// ── Stub pipeline ─────────────────────────────────────────────────────────────

struct StubPipeline(Vec<RetrievalResult>);

#[async_trait]
impl QueryPipeline for StubPipeline {
    async fn query(&self, _req: RetrievalQuery) -> anyhow::Result<Vec<RetrievalResult>> {
        Ok(self.0.clone())
    }
}

// ── State helpers ─────────────────────────────────────────────────────────────

fn empty_state() -> McpAppState {
    McpAppState {
        graph_db:      Arc::new(GraphDb::open_in_memory().unwrap()),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    }
}

fn sym(id: u64, file_id: u64, name: &str, file_path: &str) -> Symbol {
    Symbol {
        id,
        file_id,
        name:           name.to_string(),
        qualified_name: format!("{file_path}::{name}"),
        kind:           SymbolKind::Function,
        span:           Span { start_byte: 0, end_byte: 50, start_line: 1, end_line: 5 },
        docstring:      None,
        language:       Language::Rust,
        file_path:      PathBuf::from(file_path),
    }
}

fn req(method: &str, params: Option<Value>) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id:      Some(json!(1)),
        method:  method.to_string(),
        params,
    }
}

// ── JSON-RPC protocol tests ───────────────────────────────────────────────────

#[tokio::test]
async fn initialize_returns_protocol_version() {
    let state = empty_state();
    let resp  = dispatch(&state, req("initialize", None)).await;
    assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2025-06-18", "no requested version: offer the newest");
    assert!(result["capabilities"].is_object());
    assert_eq!(result["serverInfo"]["name"], "graphrelay");
}

#[tokio::test]
async fn initialize_echoes_a_supported_requested_version_and_replaces_an_unknown_one() {
    let state = empty_state();
    let resp = dispatch(&state, req("initialize", Some(json!({"protocolVersion": "2025-03-26"})))).await;
    assert_eq!(resp.result.unwrap()["protocolVersion"], "2025-03-26");
    let resp = dispatch(&state, req("initialize", Some(json!({"protocolVersion": "1999-01-01"})))).await;
    assert_eq!(resp.result.unwrap()["protocolVersion"], "2025-06-18");
}

#[tokio::test]
async fn ping_returns_empty_object() {
    let state = empty_state();
    let resp  = dispatch(&state, req("ping", None)).await;
    assert!(resp.error.is_none());
    assert_eq!(resp.result.unwrap(), json!({}));
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let state = empty_state();
    let resp  = dispatch(&state, req("no_such_method", None)).await;
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32601);
    assert!(err.message.contains("no_such_method"));
}

#[tokio::test]
async fn id_is_echoed_back() {
    let mut r = req("ping", None);
    r.id = Some(json!(42));
    let state = empty_state();
    let resp  = dispatch(&state, r).await;
    assert_eq!(resp.id, Some(json!(42)));
}

// ── tools/list ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tools_list_returns_exactly_4_tools() {
    let state = empty_state();
    let resp  = dispatch(&state, req("tools/list", None)).await;
    assert!(resp.error.is_none());
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 4, "MCP invariant (Plan 23): exactly 4 tools must be listed");
}

/// Every description follows one shape so an agent can tell when to call a
/// tool and when not to, and what to do next. Budget guards the length: the
/// four descriptions are re-read on every session.
#[tokio::test]
async fn every_tool_description_says_when_to_use_and_when_not_to() {
    let state = empty_state();
    let resp = dispatch(&state, req("tools/list", None)).await;
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 4);
    let mut total = 0;
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        let d = t["description"].as_str().unwrap();
        total += d.len();
        eprintln!("description chars: {name} = {}", d.len());
        assert!(d.contains("USE WHEN:"), "{name} lacks a USE WHEN section");
        assert!(d.contains("DON'T USE:"), "{name} lacks a DON'T USE section");
        assert!(d.len() <= 2_600, "{name} description is {} chars; keep each under 2,600", d.len());
        assert!(!d.contains("embedding"), "{name} must not mention embeddings (there are none)");
    }
    for name in ["search_code", "inspect", "trace_impact"] {
        let d = tools.iter().find(|t| t["name"] == name).unwrap()["description"].as_str().unwrap().to_string();
        assert!(d.contains("FLOW:"), "{name} lacks a FLOW section");
        assert!(d.contains("search_code") || d.contains("inspect") || d.contains("trace_impact"));
    }
    assert!(total <= 8_000, "all four descriptions total {total} chars; the rewrite targeted about 5,000");
}

#[tokio::test]
async fn tools_list_names_are_correct() {
    let state = empty_state();
    let resp  = dispatch(&state, req("tools/list", None)).await;
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

    for expected in &[
        "search_code",
        "inspect",
        "trace_impact",
        "watch_status",
    ] {
        assert!(names.contains(expected), "missing tool: {expected}");
    }
}

#[tokio::test]
async fn each_tool_has_required_input_schema_fields() {
    let state = empty_state();
    let resp  = dispatch(&state, req("tools/list", None)).await;
    let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
    for tool in &tools {
        let name = tool["name"].as_str().unwrap();
        assert!(tool["description"].is_string(), "{name}: missing description");
        assert!(tool["inputSchema"].is_object(), "{name}: missing inputSchema");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}: inputSchema type must be object");
    }
}

// ── tools/call — missing/bad params ──────────────────────────────────────────

#[tokio::test]
async fn tools_call_without_params_returns_invalid_params() {
    let state = empty_state();
    let resp  = dispatch(&state, req("tools/call", None)).await;
    let err   = resp.error.unwrap();
    assert_eq!(err.code, -32602);
}

#[tokio::test]
async fn tools_call_unknown_tool_returns_error() {
    let state = empty_state();
    let resp  = dispatch(&state, req("tools/call", Some(json!({
        "name": "does_not_exist",
        "arguments": {}
    })))).await;
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("does_not_exist"));
}

#[tokio::test]
async fn tools_call_missing_required_arg_returns_error() {
    let state = empty_state();
    // inspect requires exactly one of "symbol"/"path" — call with neither.
    // Args deserializes fine (both are Option), so this must surface as a
    // tool-level (isError: true) response, not a JSON-RPC invalid_params.
    let resp = dispatch(&state, req("tools/call", Some(json!({
        "name": "inspect",
        "arguments": {}
    })))).await;
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("symbol") && text.contains("path"), "error should name the missing args: {text}");
}

// ── watch_status tool ─────────────────────────────────────────────────────────

#[tokio::test]
async fn watch_status_returns_state_and_counts() {
    let mut status         = IndexStatus::fresh();
    status.total_files     = 7;
    status.total_symbols   = 42;
    status.state           = IndexState::Fresh;

    let state = McpAppState {
        graph_db:      Arc::new(GraphDb::open_in_memory().unwrap()),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(status)),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::watch_status::handle(&state).await.unwrap();
    assert_eq!(result["state"], "fresh");
    assert_eq!(result["total_files"], 7);
    assert_eq!(result["total_symbols"], 42);
}

// ── explain_symbol tool ───────────────────────────────────────────────────────

#[tokio::test]
async fn explain_symbol_returns_symbol_fields() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 1, path: PathBuf::from("lib.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    let s = sym(100, 1, "my_fn", "lib.rs");
    db.upsert_symbols(&[s]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::explain_symbol::handle(
        &state,
        tools::explain_symbol::Args { symbol: "my_fn".into() },
    ).await.unwrap();

    assert!(result["symbol"].as_str().unwrap().contains("my_fn"));
    assert_eq!(result["kind"], "Function");
    assert!(result["callers"].is_array());
    assert!(result["callees"].is_array());
}

#[tokio::test]
async fn explain_symbol_unknown_returns_server_error() {
    let state = empty_state();
    let resp  = tools::explain_symbol::handle(
        &state,
        tools::explain_symbol::Args { symbol: "ghost_fn".into() },
    ).await;
    assert!(resp.is_err(), "unknown symbol must return Err");
}

#[tokio::test]
async fn explain_symbol_resolves_dotted_class_method_guess() {
    // Agents commonly guess `Class.method` or a fully-qualified Python import path
    // (`pkg.module.Class.method`) — neither is how this index's qualified_name or bare
    // `name` field is shaped, so this used to fail outright ("symbol not found") even
    // though the target is indexed and reachable by its own bare name.
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 10, path: PathBuf::from("/repo/erpnext/controllers/stock_controller.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(1000, 10, "make_gl_entries", "/repo/erpnext/controllers/stock_controller.py")]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::explain_symbol::handle(
        &state,
        tools::explain_symbol::Args { symbol: "StockController.make_gl_entries".into() },
    ).await.unwrap();

    assert!(result["symbol"].as_str().unwrap().contains("make_gl_entries"));
    // Unique name in this index — no other candidate to be ambiguous against.
    assert!(result["resolution_note"].is_null());
}

#[tokio::test]
async fn explain_symbol_flags_ambiguous_dotted_guess_with_a_note() {
    // Same dotted-guess recovery, but with 2 same-named symbols in the index — silently
    // picking one by in-degree without saying so risks a confident, wrong-context answer,
    // exactly the failure mode Group F ("trap / hallucination resistance") targets.
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    db.upsert_file(&graphrelay_core::types::FileRecord {
        id: 11, path: PathBuf::from("/repo/a/mod.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 50,
    }).unwrap();
    db.upsert_file(&graphrelay_core::types::FileRecord {
        id: 12, path: PathBuf::from("/repo/b/mod.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 50,
    }).unwrap();
    db.upsert_symbols(&[
        sym(1100, 11, "validate", "/repo/a/mod.py"),
        sym(1101, 12, "validate", "/repo/b/mod.py"),
    ]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::explain_symbol::handle(
        &state,
        tools::explain_symbol::Args { symbol: "SomeClass.validate".into() },
    ).await.unwrap();

    assert!(!result["resolution_note"].is_null(), "ambiguous name-only match must carry a note");
}

// ── get_file_context tool ─────────────────────────────────────────────────────

#[tokio::test]
async fn get_file_context_returns_symbols_for_indexed_file() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 2, path: PathBuf::from("ops.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(200, 2, "alpha", "ops.rs"), sym(201, 2, "beta", "ops.rs")]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::get_file_context::handle(
        &state,
        tools::get_file_context::Args { path: "ops.rs".into(), focus_line: None, include_source: None },
    ).await.unwrap();

    assert_eq!(result["path"], "ops.rs");
    assert_eq!(result["symbol_count"], 2);
    assert_eq!(result["language"], "rust");
    assert_eq!(result["symbols"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn get_file_context_focus_line_returns_focused_symbol() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 3, path: PathBuf::from("focused.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 200,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(300, 3, "at_line_3", "focused.rs")]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    // sym spans lines 1-5; focus on line 3 should match
    let result = tools::get_file_context::handle(
        &state,
        tools::get_file_context::Args { path: "focused.rs".into(), focus_line: Some(3), include_source: None },
    ).await.unwrap();

    assert!(!result["focused_symbol"].is_null(), "focus_line 3 should match the symbol at lines 1-5");
}

/// `focus_line` and `lines` are 1-based like `Read` and `source`. The symbol's
/// span starts on 0-based line 1, i.e. the file's second line: focusing line 2
/// must find it and line 1 must not (the old 0-vs-1 mixup was off by one).
#[tokio::test]
async fn get_file_context_lines_and_focus_line_are_one_based() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 5, path: PathBuf::from("lines.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 200,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(500, 5, "second_line", "lines.rs")]).unwrap();
    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };
    let at = |line: u32| tools::get_file_context::handle(
        &state,
        tools::get_file_context::Args { path: "lines.rs".into(), focus_line: Some(line), include_source: None },
    );

    let hit = at(2).await.unwrap();
    assert_eq!(hit["focused_symbol"]["lines"], "2-6");
    assert_eq!(hit["symbols"][0]["lines"], "2-6");
    assert!(at(1).await.unwrap()["focused_symbol"].is_null());
    assert!(at(7).await.unwrap()["focused_symbol"].is_null());
}

#[tokio::test]
async fn get_file_context_unknown_file_returns_error() {
    let state = empty_state();
    let resp  = tools::get_file_context::handle(
        &state,
        tools::get_file_context::Args { path: "no_such_file.rs".into(), focus_line: None, include_source: None },
    ).await;
    assert!(resp.is_err());
}

#[tokio::test]
async fn get_file_context_resolves_repo_relative_path_against_absolute_index() {
    // Regression test: the index stores absolute paths, but Read/Grep/Glob
    // all accept repo-relative paths — an exact-match-only lookup silently
    // failed with "file not indexed" for any caller that (reasonably)
    // followed that convention with get_file_context too.
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 5, path: PathBuf::from("/repo/erpnext/stock/doctype/item/item.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(500, 5, "validate", "/repo/erpnext/stock/doctype/item/item.py")]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::get_file_context::handle(
        &state,
        tools::get_file_context::Args { path: "erpnext/stock/doctype/item/item.py".into(), focus_line: None, include_source: None },
    ).await.unwrap();

    assert_eq!(result["path"], "/repo/erpnext/stock/doctype/item/item.py");
    assert_eq!(result["symbol_count"], 1);
}

#[tokio::test]
async fn get_file_context_ambiguous_suffix_returns_error_not_a_silent_pick() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    db.upsert_file(&graphrelay_core::types::FileRecord {
        id: 6, path: PathBuf::from("/repo/erpnext/erpnext/utils.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 50,
    }).unwrap();
    db.upsert_file(&graphrelay_core::types::FileRecord {
        id: 7, path: PathBuf::from("/repo/erpnext/hr/utils.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 50,
    }).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let resp = tools::get_file_context::handle(
        &state,
        tools::get_file_context::Args { path: "utils.py".into(), focus_line: None, include_source: None },
    ).await;
    assert!(resp.is_err(), "an ambiguous suffix must error, not silently pick one match");
}

// ── trace_impact tool ─────────────────────────────────────────────────────────

#[tokio::test]
async fn trace_impact_returns_affected_symbols() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 4, path: PathBuf::from("impact.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();

    let root      = sym(400, 4, "core_fn", "impact.rs");
    let dependent = sym(401, 4, "user_fn", "impact.rs");
    db.upsert_symbols(&[root, dependent]).unwrap();
    db.upsert_edges(&[graphrelay_core::types::Edge {
        from_id: 401,
        to_id:   Some(400),
        to_name: None,
        kind:    graphrelay_core::types::EdgeKind::Calls,
        source:  Default::default(),
    }]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::trace_impact::handle(
        &state,
        tools::trace_impact::Args { symbol: "core_fn".into(), depth: Some(3) },
    ).await.unwrap();

    assert!(result["root_symbol"].as_str().unwrap().contains("core_fn"));
    assert!(result["affected_count"].as_u64().unwrap() >= 1);
}

/// A trait has implementors, not callers: a Calls-only walk reported it unused.
#[tokio::test]
async fn trace_impact_of_a_type_follows_implements_edges_and_clamps_depth() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 6, path: PathBuf::from("repo.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    let mut tr = sym(600, 6, "UserRepository", "repo.rs");
    tr.kind = SymbolKind::Trait;
    let mut imp = sym(601, 6, "SqlUserRepository", "repo.rs");
    imp.kind = SymbolKind::Struct;
    db.upsert_symbols(&[tr, imp]).unwrap();
    db.upsert_edges(&[graphrelay_core::types::Edge {
        from_id: 601,
        to_id:   Some(600),
        to_name: None,
        kind:    graphrelay_core::types::EdgeKind::Implements,
        source:  Default::default(),
    }]).unwrap();
    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::trace_impact::handle(
        &state,
        tools::trace_impact::Args { symbol: "UserRepository".into(), depth: Some(200) },
    ).await.unwrap();

    assert_eq!(result["affected_count"], 1);
    assert_eq!(result["affected"][0]["symbol"], "repo.rs::SqlUserRepository");
    assert_eq!(result["depth"], 6);
    assert_eq!(result["affected_count_is_lower_bound"], false);
}

/// A second-hop caller names the direct caller it goes through, so an agent
/// does not have to open files to say which one (Gitea S10 repeat 3).
#[tokio::test]
async fn trace_impact_names_the_hop_closer_symbol_for_deeper_nodes() {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 7, path: PathBuf::from("via.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(700, 7, "leaf", "via.rs"), sym(701, 7, "mid", "via.rs"), sym(702, 7, "top", "via.rs")]).unwrap();
    let call = |from, to| graphrelay_core::types::Edge {
        from_id: from, to_id: Some(to), to_name: None,
        kind: graphrelay_core::types::EdgeKind::Calls, source: Default::default(),
    };
    db.upsert_edges(&[call(701, 700), call(702, 701)]).unwrap();
    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::trace_impact::handle(
        &state,
        tools::trace_impact::Args { symbol: "leaf".into(), depth: Some(2) },
    ).await.unwrap();

    let item = |name: &str| result["affected"].as_array().unwrap().iter()
        .find(|a| a["symbol"] == name).unwrap().clone();
    assert!(item("via.rs::mid")["via"].is_null());
    assert_eq!(item("via.rs::top")["via"], "via.rs::mid");
    // The dependent's own range, 1-based like every other `lines` field (the
    // fixture symbol's 0-based span is 1..5).
    assert_eq!(item("via.rs::top")["lines"], "2-6");
}

#[tokio::test]
async fn trace_impact_unknown_symbol_returns_error() {
    let state = empty_state();
    let resp  = tools::trace_impact::handle(
        &state,
        tools::trace_impact::Args { symbol: "phantom".into(), depth: None },
    ).await;
    assert!(resp.is_err());
}

#[tokio::test]
async fn trace_impact_resolves_repo_relative_qualified_symbol() {
    // Regression test: the index stores `<absolute-path>::<name>` qualified names, but
    // an agent following find_relevant_code's echoed repo-relative paths (reasonably)
    // guesses `<repo-relative-path>::<name>` instead — an exact-match-only resolve_symbol
    // silently failed with "symbol not found" for this shape, forcing a Grep fallback
    // that can't see call/caller graph data at all.
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 8, path: PathBuf::from("/repo/erpnext/stock/doctype/item/item.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(800, 8, "validate", "/repo/erpnext/stock/doctype/item/item.py")]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::trace_impact::handle(
        &state,
        tools::trace_impact::Args {
            symbol: "erpnext/stock/doctype/item/item.py::validate".into(),
            depth:  Some(1),
        },
    ).await.unwrap();

    assert_eq!(result["root_symbol"], "/repo/erpnext/stock/doctype/item/item.py::validate");
}

#[tokio::test]
async fn trace_impact_truncates_large_result_sets() {
    // A highly-connected symbol (e.g. a framework's shared validate() hook) can have an
    // impact set of thousands of nodes — returning all of them can exceed an MCP client's
    // per-result token limit and abort the whole call. The tool must cap and say so rather
    // than either erroring out or silently returning an unusable multi-hundred-KB result.
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 9, path: PathBuf::from("hub.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();

    let root = sym(900, 9, "core_fn", "hub.rs");
    db.upsert_symbols(&[root]).unwrap();

    let mut callers = Vec::new();
    let mut edges    = Vec::new();
    for i in 0..250u64 {
        let id = 1000 + i;
        callers.push(sym(id, 9, &format!("caller_{i}"), "hub.rs"));
        edges.push(graphrelay_core::types::Edge {
            from_id: id,
            to_id:   Some(900),
            to_name: None,
            kind:    graphrelay_core::types::EdgeKind::Calls,
            source:  Default::default(),
        });
    }
    db.upsert_symbols(&callers).unwrap();
    db.upsert_edges(&edges).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::trace_impact::handle(
        &state,
        tools::trace_impact::Args { symbol: "core_fn".into(), depth: Some(1) },
    ).await.unwrap();

    assert_eq!(result["affected_count"], 250);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["affected"].as_array().unwrap().len(), 200);
    assert!(result["note"].as_str().unwrap().contains("200"));
}

// ── find_relevant_code tool ───────────────────────────────────────────────────

#[tokio::test]
async fn find_relevant_code_returns_empty_when_no_hits() {
    // Unaffected by Plan 23's file-grouping follow-up — an empty flat list
    // groups into an empty list of groups either way.
    let state  = empty_state(); // StubPipeline returns []
    let result = tools::find_relevant_code::handle(
        &state,
        tools::find_relevant_code::Args {
            intent:   "find something".into(),
            top_n:    Some(5),
            language: None,
        },
    ).await.unwrap();

    assert_eq!(result["count"], 0);
    assert_eq!(result["results"].as_array().unwrap().len(), 0);
    assert_eq!(result["query"], "find something");
}

#[tokio::test]
async fn find_relevant_code_respects_top_n() {
    // Unaffected by Plan 23's file-grouping follow-up — `count` keeps its
    // old meaning (total matched symbols) regardless of how many file-groups
    // they land in; all 3 stub results here share one file anyway.
    let results = make_stub_results(10);
    let state   = McpAppState {
        graph_db:      Arc::new(GraphDb::open_in_memory().unwrap()),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(results[..3].to_vec())),
    };

    let result = tools::find_relevant_code::handle(
        &state,
        tools::find_relevant_code::Args {
            intent:   "query".into(),
            top_n:    Some(3),
            language: None,
        },
    ).await.unwrap();

    assert_eq!(result["count"], 3);
}

// ── summarize_module tool ─────────────────────────────────────────────────────

#[tokio::test]
async fn summarize_module_falls_back_to_parent_dir_of_a_single_file_path() {
    // `path` is documented as a directory/module prefix, but an agent naturally passes a
    // specific file when it means "summarize the module this file lives in" (the same
    // path shape it would hand get_file_context). files_in_prefix's `%/{prefix}/%`
    // pattern requires something *after* the prefix, which a leaf file never has, so this
    // used to error ("no indexed files found") on an entirely foreseeable input shape.
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 20, path: PathBuf::from("/repo/erpnext/controllers/stock_controller.py"), checksum: 0,
        language: Language::Python, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(2000, 20, "make_gl_entries", "/repo/erpnext/controllers/stock_controller.py")]).unwrap();

    let state = McpAppState {
        graph_db:      Arc::clone(&db),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    };

    let result = tools::summarize_module::handle(
        &state,
        tools::summarize_module::Args {
            path: "erpnext/controllers/stock_controller.py".into(),
            max_symbols: None,
        },
    ).await.unwrap();

    assert_eq!(result["file_count"], 1);
    assert_eq!(result["symbol_count"], 1);
}

#[tokio::test]
async fn summarize_module_unknown_path_still_returns_error() {
    let state = empty_state();
    let resp  = tools::summarize_module::handle(
        &state,
        tools::summarize_module::Args { path: "no/such/module".into(), max_symbols: None },
    ).await;
    assert!(resp.is_err());
}

// ── Notification handling ─────────────────────────────────────────────────────

#[tokio::test]
async fn notification_initialized_returns_http_202_no_body() {
    // MCP spec: notifications (no id) must not get a JSON-RPC response.
    // The server must return HTTP 202 Accepted with an empty body.
    let app  = mcp_router(empty_state());
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            ))
            .unwrap(),
    ).await.unwrap();

    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn notification_cancelled_returns_http_202() {
    let app  = mcp_router(empty_state());
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
            ))
            .unwrap(),
    ).await.unwrap();

    assert_eq!(resp.status(), StatusCode::ACCEPTED);
}

// ── HTTP layer ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn http_tools_list_returns_200() {
    let app  = mcp_router(empty_state());
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#))
            .unwrap(),
    ).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn http_bad_json_returns_a_jsonrpc_parse_error() {
    let app  = mcp_router(empty_state());
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from("not json at all"))
            .unwrap(),
    ).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap(),
    ).unwrap();
    assert_eq!(body["error"]["code"], -32700);
    assert!(body["id"].is_null());
}

#[tokio::test]
async fn http_null_id_is_a_request_not_a_notification() {
    let app  = mcp_router(empty_state());
    let resp = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap(),
    ).unwrap();
    assert!(body["result"].is_object());
}

#[tokio::test]
async fn http_batch_or_wrong_version_is_an_invalid_request() {
    for payload in [r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#, r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#] {
        let app  = mcp_router(empty_state());
        let resp = app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .unwrap(),
        ).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap(),
        ).unwrap();
        assert_eq!(body["error"]["code"], -32600, "{payload}");
    }
}

// ── HTTP tools/call round-trip tests ─────────────────────────────────────────

/// Helper: POST /mcp and parse the JSON-RPC response body.
///
/// The MCP server wraps tool results in `{"content":[{"type":"text","text":"..."}],"isError":bool}`.
/// This helper unwraps that layer so callers can assert against the logical result:
///   - success  → `resp["result"]` is the parsed tool output JSON
///   - tool err → `resp["error"]` = `{"code": -32000, "message": "..."}`
async fn call_tool(state: McpAppState, name: &str, arguments: Value) -> Value {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    });

    let resp = mcp_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "HTTP status should be 200 for all tools/call");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let mut body: Value = serde_json::from_slice(&bytes).expect("response must be valid JSON");

    // Unwrap the MCP content envelope present on successful dispatch.
    if let Some(text) = body["result"]["content"][0]["text"].as_str().map(str::to_owned) {
        let is_error = body["result"]["isError"].as_bool().unwrap_or(false);
        if is_error {
            body["error"] = json!({ "code": -32000, "message": text });
            body["result"] = Value::Null;
        } else if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
            body["result"] = parsed;
        }
    }

    body
}

/// Helper: build a state with two pre-indexed symbols in a given file.
fn state_with_symbols(file_id: u64, file_path: &str) -> McpAppState {
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id:         file_id,
        path:       PathBuf::from(file_path),
        checksum:   0,
        language:   Language::Rust,
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        byte_size:  100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[
        sym(file_id * 10,     file_id, "alpha", file_path),
        sym(file_id * 10 + 1, file_id, "beta",  file_path),
    ]).unwrap();
    McpAppState {
        graph_db:      db,
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(vec![])),
    }
}

#[tokio::test]
async fn http_watch_status_tool_returns_state() {
    let resp = call_tool(empty_state(), "watch_status", json!({})).await;
    assert!(resp["result"].is_object(), "result must be an object");
    assert!(resp["result"]["state"].is_string(), "state field must be present");
    assert!(resp["error"].is_null(), "must have no error");
}

#[tokio::test]
async fn http_tools_list_body_contains_4_tools() {
    let payload = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let resp = mcp_router(empty_state())
        .oneshot(
            Request::builder()
                .method("POST").uri("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(payload)).unwrap(),
        ).await.unwrap();

    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let tools = body["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
}

#[tokio::test]
async fn http_inspect_path_mode_returns_symbols() {
    let state  = state_with_symbols(1, "src/lib.rs");
    let resp   = call_tool(state, "inspect", json!({ "path": "src/lib.rs" })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert_eq!(resp["result"]["symbol_count"], 2);
    assert_eq!(resp["result"]["language"], "rust");
    let syms = resp["result"]["symbols"].as_array().unwrap();
    assert_eq!(syms.len(), 2);
    let names: Vec<&str> = syms.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(names.iter().any(|n| n.contains("alpha")));
}

/// Real file on disk (unlike `state_with_symbols`'s made-up path) so
/// `SourceLoader::full_text` can actually succeed — needed to verify the
/// `include_source` default. A real benchmark transcript investigation
/// (2026-09-28) found the agent essentially never passed `include_source:
/// true` explicitly when it defaulted to `false`, then `Read` the same file
/// anyway every time — making the capability a dead letter in practice.
/// Defaulted to `true` for exactly this reason; these two tests pin both
/// directions so a future change can't silently flip the default back.
fn state_with_real_file(dir: &std::path::Path) -> (McpAppState, PathBuf) {
    let path = dir.join("inspect_default_test.rs");
    std::fs::write(&path, "fn alpha() {}\nfn beta() {}\n").unwrap();
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    db.upsert_file(&graphrelay_core::types::FileRecord {
        id: 1, path: path.clone(), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 30,
    }).unwrap();
    db.upsert_symbols(&[sym(1, 1, "alpha", path.to_str().unwrap())]).unwrap();
    let state = McpAppState {
        graph_db: db, source_loader: Arc::new(SourceLoader::new()),
        watch_status: Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline: Arc::new(StubPipeline(vec![])),
    };
    (state, path)
}

#[tokio::test]
async fn http_inspect_path_mode_includes_source_by_default() {
    let dir = std::env::temp_dir().join(format!("cg-inspect-default-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (state, path) = state_with_real_file(&dir);
    let resp = call_tool(state, "inspect", json!({ "path": path.to_str().unwrap() })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert!(resp["result"]["source"].as_str().unwrap().contains("fn alpha"),
        "include_source should default to true: {}", resp["result"]);
}

#[tokio::test]
async fn http_inspect_path_mode_include_source_false_omits_source() {
    let dir = std::env::temp_dir().join(format!("cg-inspect-default-false-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (state, path) = state_with_real_file(&dir);
    let resp = call_tool(state, "inspect", json!({ "path": path.to_str().unwrap(), "include_source": false })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert!(resp["result"]["source"].is_null(), "explicit include_source:false must omit source: {}", resp["result"]);
}

#[tokio::test]
async fn http_inspect_unresolvable_path_returns_server_error() {
    // Not a file, and not a directory prefix with any indexed files under
    // it either — inspect's path mode falls through file-context ->
    // module-summary and both must fail for a genuinely unindexed path.
    let resp = call_tool(empty_state(), "inspect", json!({ "path": "no_such.rs" })).await;
    assert!(!resp["error"].is_null(), "unindexed path must return an error");
    assert_eq!(resp["error"]["code"], -32000);
}

#[tokio::test]
async fn http_inspect_symbol_mode_returns_definition() {
    let state = state_with_symbols(2, "util.rs");
    let resp  = call_tool(state, "inspect", json!({ "symbol": "alpha" })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert!(resp["result"]["symbol"].as_str().unwrap().contains("alpha"));
    assert_eq!(resp["result"]["kind"], "Function");
    assert!(resp["result"]["callers"].is_array());
    assert!(resp["result"]["callees"].is_array());
}

#[tokio::test]
async fn http_inspect_unknown_symbol_returns_error() {
    let resp = call_tool(empty_state(), "inspect", json!({ "symbol": "ghost" })).await;
    assert!(!resp["error"].is_null(), "unknown symbol must return an error");
}

#[tokio::test]
async fn http_inspect_both_symbol_and_path_returns_error() {
    let resp = call_tool(empty_state(), "inspect", json!({ "symbol": "alpha", "path": "src/lib.rs" })).await;
    assert!(!resp["error"].is_null(), "passing both symbol and path must return an error");
}

#[tokio::test]
async fn http_trace_impact_tool_returns_affected() {
    // Build a state where beta calls alpha
    let db = Arc::new(GraphDb::open_in_memory().unwrap());
    let fr = graphrelay_core::types::FileRecord {
        id: 3, path: PathBuf::from("impact.rs"), checksum: 0,
        language: Language::Rust, indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 100,
    };
    db.upsert_file(&fr).unwrap();
    db.upsert_symbols(&[sym(30, 3, "alpha", "impact.rs"), sym(31, 3, "beta", "impact.rs")]).unwrap();
    db.upsert_edges(&[graphrelay_core::types::Edge {
        from_id: 31, to_id: Some(30), to_name: None,
        kind: graphrelay_core::types::EdgeKind::Calls,
        source: Default::default(),
    }]).unwrap();
    let state = McpAppState {
        graph_db: db, source_loader: Arc::new(SourceLoader::new()),
        watch_status: Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline: Arc::new(StubPipeline(vec![])),
    };

    let resp = call_tool(state, "trace_impact", json!({ "symbol": "alpha" })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert!(resp["result"]["root_symbol"].as_str().unwrap().contains("alpha"));
    assert!(resp["result"]["affected_count"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn http_trace_impact_unknown_symbol_returns_error() {
    let resp = call_tool(empty_state(), "trace_impact", json!({ "symbol": "phantom" })).await;
    assert!(!resp["error"].is_null());
}

#[tokio::test]
async fn http_search_code_intent_mode_returns_results() {
    let results = make_stub_results(3);
    let state   = McpAppState {
        graph_db:      Arc::new(GraphDb::open_in_memory().unwrap()),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(results)),
    };

    let resp = call_tool(state, "search_code", json!({ "intent": "something useful" })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert_eq!(resp["result"]["count"], 3);
    assert_eq!(resp["result"]["query"], "something useful");
    // make_stub_results puts all 3 symbols in the same file ("stub.rs"), so
    // Plan 23's follow-up (grouping by file) collapses them into a single
    // file-group here — see the multi-file test below for the >1-group case.
    assert_eq!(resp["result"]["file_count"], 1);
    let items = resp["result"]["results"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["source"].is_string());
    let symbols = items[0]["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 3);
    assert!(symbols[0]["rank"].as_u64().unwrap() >= 1);
    assert!(symbols[0]["symbol"].is_string());
}

#[tokio::test]
async fn http_search_code_intent_mode_groups_multi_file_results_by_file() {
    let dir = std::env::temp_dir().join(format!("cg-search-code-group-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path_a = dir.join("a.rs");
    let path_b = dir.join("b.rs");
    std::fs::write(&path_a, "fn alpha() {}\nfn beta() {}\n").unwrap();
    std::fs::write(&path_b, "fn gamma() {}\n").unwrap();

    let mk = |id: u64, name: &str, qualified: &str, line: u32, path: &PathBuf, rank: usize, score: f32| RetrievalResult {
        rank, score,
        symbol: Symbol {
            id: id as SymbolId, file_id: 0, name: name.to_string(), qualified_name: qualified.to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 10, start_line: line, end_line: line },
            docstring: None, language: Language::Rust, file_path: path.clone(),
        },
        file_path: path.clone(), snippet: format!("fn {name}() {{}}"), callers: vec![], callees: vec![],
    };
    let results = vec![
        mk(1, "alpha", "a.rs::alpha", 0, &path_a, 1, 1.0),
        mk(2, "beta",  "a.rs::beta",  1, &path_a, 2, 0.9),
        mk(3, "gamma", "b.rs::gamma", 0, &path_b, 3, 0.8),
    ];

    let state = McpAppState {
        graph_db:      Arc::new(GraphDb::open_in_memory().unwrap()),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus::fresh())),
        pipeline:      Arc::new(StubPipeline(results)),
    };

    let resp = call_tool(state, "search_code", json!({ "intent": "test" })).await;
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);
    assert_eq!(resp["result"]["count"], 3);
    assert_eq!(resp["result"]["file_count"], 2);
    let items = resp["result"]["results"].as_array().unwrap();
    assert_eq!(items.len(), 2);

    // Group order is min-rank order: the file containing rank 1 comes first.
    assert_eq!(items[0]["file"], path_a.to_string_lossy().to_string());
    let symbols_a = items[0]["symbols"].as_array().unwrap();
    assert_eq!(symbols_a.len(), 2);
    assert_eq!(symbols_a[0]["rank"], 1);
    assert_eq!(symbols_a[1]["rank"], 2);
    let source_a = items[0]["source"].as_str().unwrap();
    assert!(source_a.contains("fn alpha"), "source must contain rank-1 symbol's own span: {source_a}");
    assert!(source_a.contains("fn beta"), "source must contain rank-2 symbol's own span: {source_a}");

    assert_eq!(items[1]["file"], path_b.to_string_lossy().to_string());
    let symbols_b = items[1]["symbols"].as_array().unwrap();
    assert_eq!(symbols_b.len(), 1);
    assert_eq!(symbols_b[0]["rank"], 3);
    let source_b = items[1]["source"].as_str().unwrap();
    assert!(source_b.contains("fn gamma"), "source must contain the matched symbol's own span: {source_b}");
}

#[tokio::test]
async fn http_search_code_intent_mode_empty_returns_count_zero() {
    let resp = call_tool(empty_state(), "search_code", json!({ "intent": "nothing" })).await;
    assert!(resp["error"].is_null());
    assert_eq!(resp["result"]["count"], 0);
}

#[tokio::test]
async fn http_search_code_neither_intent_nor_implements_returns_error() {
    // search_code's Args are both Option — this deserializes fine, so the
    // "exactly one required" check is a tool-level error (isError: true,
    // normalized by call_tool to code -32000), not a JSON-RPC invalid_params.
    let resp = call_tool(empty_state(), "search_code", json!({})).await;
    assert!(!resp["error"].is_null(), "missing intent/implements must return error");
    assert_eq!(resp["error"]["code"], -32000);
}

#[tokio::test]
async fn http_search_code_both_intent_and_implements_returns_error() {
    let resp = call_tool(empty_state(), "search_code", json!({ "intent": "x", "implements": "Y" })).await;
    assert!(!resp["error"].is_null(), "passing both intent and implements must return an error");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_stub_results(n: usize) -> Vec<RetrievalResult> {
    (1..=n).map(|i| RetrievalResult {
        rank:      i,
        score:     1.0 - (i as f32 * 0.05),
        symbol:    Symbol {
            id:             i as SymbolId,
            file_id:        0,
            name:           format!("fn_{i}"),
            qualified_name: format!("stub.rs::fn_{i}"),
            kind:           SymbolKind::Function,
            span:           Span { start_byte: 0, end_byte: 10, start_line: i as u32, end_line: i as u32 + 1 },
            docstring:      None,
            language:       Language::Rust,
            file_path:      PathBuf::from("stub.rs"),
        },
        file_path: PathBuf::from("stub.rs"),
        snippet:   format!("fn fn_{i}() {{}}"),
        callers:   vec![],
        callees:   vec![],
    }).collect()
}
