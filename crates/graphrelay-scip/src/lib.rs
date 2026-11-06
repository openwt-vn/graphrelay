//! graphrelay-scip — Plan 12 SCIP semantic layer.
//!
//! Enabled with `--features scip`. Without the feature this crate compiles
//! to an empty stub so the main binary stays lean.

pub mod mapper;
pub mod merger;

pub use merger::ScipIngestStats;

#[cfg(feature = "scip")]
use std::path::Path;

/// Merge a `.scip` file into the graphrelay store, enriching symbol kinds and
/// adding cross-file call/import edges with `source = 'semantic'`.
///
/// Idempotent: all previous semantic edges are deleted before re-inserting.
/// Returns statistics about what was enriched.
/// `root` is the project root `scip_path`'s document paths are relative to
/// — see `merger::merge_index`'s doc comment for why this is now required
/// (it used to be resolved implicitly per-occurrence via an unindexable
/// `LIKE` scan; fixed to resolve per-document via an indexed exact match).
#[cfg(feature = "scip")]
pub fn merge_scip(
    scip_path: &Path,
    graph_db: &graphrelay_store::GraphDb,
    root: &Path,
) -> anyhow::Result<ScipIngestStats> {
    use protobuf::Message;

    tracing::info!("ingesting SCIP index from {}", scip_path.display());
    let bytes = std::fs::read(scip_path)
        .map_err(|e| anyhow::anyhow!("failed to read SCIP file {}: {e}", scip_path.display()))?;

    let index = scip::types::Index::parse_from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("failed to parse SCIP protobuf: {e}"))?;

    let stats = merger::merge_index(&index, graph_db, root)?;
    tracing::info!(
        "SCIP ingest complete: {} files, {} symbols enriched, {} edges added",
        stats.files_processed, stats.symbols_enriched, stats.edges_added
    );
    Ok(stats)
}

/// Stub when compiled without the `scip` feature — returns an error with a
/// helpful message rather than silently doing nothing.
#[cfg(not(feature = "scip"))]
pub fn merge_scip(
    _scip_path: &std::path::Path,
    _graph_db: &graphrelay_store::GraphDb,
    _root: &std::path::Path,
) -> anyhow::Result<ScipIngestStats> {
    anyhow::bail!(
        "SCIP support is not compiled in. Rebuild with `--features scip` to enable it."
    )
}
