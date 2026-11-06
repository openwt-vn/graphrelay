/// find_implementations — MCP tool to find all classes/structs that implement or
/// inherit from a given interface, trait, protocol, or base class.
///
/// Graph-based only: Inherits/Implements edges pointing at the target, walked
/// transitively (see `GraphDb::implementors_of`; Plan 21 item 4 dropped
/// References, which were mostly type-annotation mentions). Plan 20 removed
/// vector search, which this tool used to run as an unconditional second
/// pass ("Strategy 2: semantic", tagging results `"source": "semantic"`) to
/// catch implementations with no direct edge. That pass is gone rather than
/// left running degraded: without an embedder, `pipeline.query()`'s only
/// remaining channel is lexical (FTS5) keyword search, and the synthetic
/// queries this used ("class implementing {type_name}", etc.) are mostly
/// generic English words that would have diluted a keyword search rather
/// than approximating anything semantic — a misleading `"semantic"` tag on
/// what would really be near-noise keyword hits.
use crate::server::McpAppState;
use crate::tools::{is_possibly_stale, truncate_snippet_on_line_boundary, SymbolNotFound};
use serde::Deserialize;
use serde_json::Value;

/// Implementations listed per response.
pub(crate) const MAX_SHOWN: usize = 15;
/// Per-implementation snippet budget, cut on a line boundary.
const SNIPPET_CHARS: usize = 300;

#[derive(Debug, Deserialize)]
pub struct Args {
    /// Name of the interface, trait, base class, or protocol to look up.
    pub type_name: String,
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    let type_name = args.type_name.trim().to_string();
    tracing::info!(type_name = %type_name, "find_implementations");

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut total_found = 0usize;

    // Only "no such symbol" means "no implementations"; a database error used to
    // be reported to the agent as a spelling problem.
    let resolved = match crate::tools::resolve_symbol_verbose(&state.graph_db, &type_name) {
        Ok(r) => Some(r),
        Err(e) if e.downcast_ref::<SymbolNotFound>().is_some() => None,
        Err(e) => return Err(e),
    };
    let resolution_note = resolved.as_ref().and_then(|(_, note)| note.clone());

    if let Some((target, _)) = &resolved {
        let implementors = state.graph_db.implementors_of(target.id)?;
        total_found = implementors.len();
        for sym in implementors {
            if seen_ids.insert(sym.id) {
                let snippet = state.source_loader.snippet(&sym).unwrap_or_else(|e| {
                    tracing::warn!(symbol = %sym.qualified_name, "snippet load failed: {e}");
                    String::new()
                });
                let mut item = serde_json::json!({
                    "symbol":  sym.qualified_name,
                    "name":    sym.name,
                    "kind":    format!("{:?}", sym.kind),
                    "file":    sym.file_path.display().to_string(),
                    "lines":   crate::tools::line_range(&sym.span),
                    "snippet": truncate_snippet_on_line_boundary(&snippet, SNIPPET_CHARS).0,
                    "source":  "graph",
                });
                // Plan 22 item 2: per-item, emitted only when true.
                if is_possibly_stale(&state.graph_db, &sym.file_path) {
                    item.as_object_mut().expect("json! built an object")
                        .insert("possibly_stale".to_string(), serde_json::json!(true));
                }
                results.push(item);
            }
            if results.len() >= MAX_SHOWN { break; }
        }
    }
    let truncated = total_found > results.len();
    // `implementors_of` caps its own query; at the cap the true total is unknown.
    let total_note = if total_found >= graphrelay_store::graph_db::IMPLEMENTORS_LIMIT {
        format!("at least {total_found}")
    } else {
        total_found.to_string()
    };

    let any_stale = results.iter().any(|r| r.get("possibly_stale").is_some());
    let note = if resolved.is_none() {
        format!("No indexed symbol named '{type_name}'. Check the spelling, or use search_code with `intent` to find the type first.")
    } else if results.is_empty() {
        format!("'{type_name}' is indexed but nothing inherits from or implements it via an indexed edge.")
    } else if truncated {
        format!(
            "Showing {} of {total_note} implementation(s) of '{type_name}' (alphabetical by qualified name). \
             Pass a more specific base type to narrow the set.",
            results.len()
        )
    } else if any_stale {
        format!(
            "Found {} implementation(s) of '{type_name}'. One or more marked \
             `possibly_stale` may have changed since indexing — read those files directly.",
            results.len()
        )
    } else {
        format!("Found {} implementation(s) of '{type_name}'", results.len())
    };

    Ok(serde_json::json!({
        "type_name":       type_name,
        "resolution_note": resolution_note,
        "count":           results.len(),
        "total_found":     total_found,
        "truncated":       truncated,
        "implementations": results,
        "note":            note,
    }))
}
