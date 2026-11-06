use crate::server::McpAppState;
use serde_json::Value;

pub async fn handle(state: &McpAppState) -> anyhow::Result<Value> {
    // Clone and release the lock before the synchronous DB reads below.
    let status = state.watch_status.read().await.clone();
    let scip_ingested_at = match state.graph_db.get_meta("scip_ingested_at") {
        Ok(v)  => v,
        Err(e) => { tracing::warn!("get_meta(scip_ingested_at) failed: {e}"); None }
    };
    let scip_symbols = match state.graph_db.get_meta("scip_symbols_enriched") {
        Ok(v)  => v.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0),
        Err(e) => { tracing::warn!("get_meta(scip_symbols_enriched) failed: {e}"); 0 }
    };
    Ok(serde_json::json!({
        "state":                  status.state.as_str(),
        "last_error":             status.last_error,
        "last_indexed":           status.last_indexed_at,
        "pending_files":          status.pending_count,
        "total_symbols":          status.total_symbols,
        "total_files":            status.total_files,
        "index_age_secs":         status.age_seconds(),
        "semantic_pending":       status.semantic_pending,
        "scip_present":           scip_ingested_at.is_some(),
        "scip_ingested_at":       scip_ingested_at,
        "scip_symbols_enriched":  scip_symbols,
        "jev_active":             state.pipeline.jev_active(),
        "judge_provider":         state.pipeline.judge_provider(),
    }))
}
