use crate::server::McpAppState;
use crate::tools::{capped_names, is_possibly_stale, resolve_symbol_verbose, response_budget, truncate_snippet_on_line_boundary};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub symbol: String,
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    tracing::info!(symbol = %args.symbol, "explain_symbol");
    let (sym, resolution_note) = resolve_symbol_verbose(&state.graph_db, &args.symbol)?;
    let callers = state.graph_db.callers_of(sym.id).unwrap_or_else(|e| {
        tracing::warn!(symbol = %sym.qualified_name, "callers_of failed: {e}");
        vec![]
    });
    let callees = state.graph_db.callees_of(sym.id).unwrap_or_else(|e| {
        tracing::warn!(symbol = %sym.qualified_name, "callees_of failed: {e}");
        vec![]
    });
    let snippet = state.source_loader.snippet(&sym).unwrap_or_else(|e| {
        tracing::warn!(symbol = %sym.qualified_name, "snippet load failed: {e}");
        String::new()
    });

    // Plan 12 E3: include SCIP enrichment status in response.
    let scip_symbol = match state.graph_db.get_meta_for_symbol(sym.id) {
        Ok(v)  => v,
        Err(e) => {
            tracing::warn!(symbol = %sym.qualified_name, "get_meta_for_symbol failed: {e}");
            None
        }
    };

    let (caller_names, callers_count, callers_truncated) = capped_names(&callers);
    let (callee_names, callees_count, callees_truncated) = capped_names(&callees);

    // Plan 23: repo-size-adaptive cap instead of a flat 800-char literal —
    // once find_relevant_code's top hits can carry up to 4000 chars, this
    // "I already know the symbol, give me the deep dive" tool must not
    // return *less* source than a fuzzy search hit for the same symbol.
    let total_files = state.watch_status.read().await.total_files;
    let budget = response_budget(total_files);
    let (definition, definition_truncated) =
        truncate_snippet_on_line_boundary(&snippet, budget.per_item_chars);

    let mut out = serde_json::json!({
        "symbol":        sym.qualified_name,
        "kind":          format!("{:?}", sym.kind),
        "file":          sym.file_path.display().to_string(),
        "lines":         crate::tools::line_range(&sym.span),
        "docstring":     sym.docstring,
        "definition":    definition,
        "definition_truncated": definition_truncated,
        "callers":           caller_names,
        "callers_count":     callers_count,
        "callers_truncated": callers_truncated,
        "callees":           callee_names,
        "callees_count":     callees_count,
        "callees_truncated": callees_truncated,
        "scip_enriched": scip_symbol.is_some(),
        "resolution_note": resolution_note,
    });
    // Plan 22 item 2: emitted only when true — zero token cost in the common case.
    if is_possibly_stale(&state.graph_db, &sym.file_path) {
        let obj = out.as_object_mut().expect("json! built an object");
        obj.insert("possibly_stale".to_string(), serde_json::json!(true));
        obj.insert("note".to_string(), serde_json::json!(
            "This file may have changed since it was indexed (recent edit still processing). \
             Read it directly before relying on `definition`/`lines`."
        ));
    }
    Ok(out)
}
