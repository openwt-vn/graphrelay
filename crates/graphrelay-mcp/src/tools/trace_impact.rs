use crate::server::McpAppState;
use crate::tools::resolve_symbol_verbose;
use graphrelay_core::types::{EdgeKind, SymbolKind};
use graphrelay_store::graph_db::IMPACT_NODE_BUDGET;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub symbol: String,
    pub depth:  Option<u8>,
}

/// Cap on the number of nodes returned, closest hops first. A handful of highly-connected
/// "everything eventually calls this" symbols (e.g. a framework's shared validate()/save()
/// hook) can produce an impact set of tens of thousands of nodes at the default depth,
/// which blows past the MCP client's per-result token limit and aborts the whole call —
/// forcing the agent to abandon graph-based verification and fall back to a plain-text
/// Grep that only sees the literal call site, not the traced blast radius. Truncating here
/// (with an honest total count and a note) keeps the call usable instead of unusable.
pub(crate) const MAX_AFFECTED: usize = 200;

/// Deeper than this, nearly every hub's impact set is "the whole program"; the
/// node budget would stop it anyway, but a clamp keeps the cost predictable.
pub(crate) const MAX_DEPTH: u8 = 6;

/// A type is depended on through more than calls: subclasses, implementors and
/// type mentions all break when it changes, and a trait or interface has almost
/// no `Calls` in-edges at all, so a Calls-only walk reported it as unused.
fn dependency_kinds(kind: SymbolKind) -> &'static [EdgeKind] {
    match kind {
        SymbolKind::Function | SymbolKind::Method => &[EdgeKind::Calls],
        _ => &[EdgeKind::Calls, EdgeKind::Inherits, EdgeKind::Implements, EdgeKind::References],
    }
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    tracing::info!(symbol = %args.symbol, depth = ?args.depth, "trace_impact");
    let (sym, resolution_note) = resolve_symbol_verbose(&state.graph_db, &args.symbol)?;
    let depth = args.depth.unwrap_or(3).clamp(1, MAX_DEPTH);
    let kinds = dependency_kinds(sym.kind);
    let impact = state
        .graph_db
        .impact_graph_with(sym.id, depth, kinds, IMPACT_NODE_BUDGET)?;

    let total = impact.affected.len();
    let mut affected = impact.affected;
    affected.sort_by(|a, b| {
        a.hop_depth
            .cmp(&b.hop_depth)
            .then_with(|| a.symbol.qualified_name.cmp(&b.symbol.qualified_name))
    });
    let truncated = total > MAX_AFFECTED || impact.budget_hit;
    affected.truncate(MAX_AFFECTED);
    let count_note = if impact.budget_hit { format!("at least {total}") } else { total.to_string() };

    Ok(serde_json::json!({
        "root_symbol":     sym.qualified_name,
        "resolution_note": resolution_note,
        "depth":          depth,
        "edge_kinds":     kinds.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
        "affected_count": total,
        "affected_count_is_lower_bound": impact.budget_hit,
        "truncated":      truncated,
        "note":           truncated.then(|| format!(
            "Showing the {} closest of {count_note} affected symbols, nearest hop first. \
             Narrow with a smaller `depth` or a more specific symbol to see the rest.",
            affected.len()
        )),
        "affected": affected.iter().map(|n| {
            serde_json::json!({
                "symbol":      n.symbol.qualified_name,
                "file":        n.path.display().to_string(),
                "lines":       crate::tools::line_range(&n.symbol.span),
                "kind":        format!("{:?}", n.symbol.kind),
                "edge_source": n.edge_source.as_str(),
                "reason":      n.reason,
                "hop_depth":   n.hop_depth,
                "via":         n.via,
            })
        }).collect::<Vec<_>>(),
    }))
}
