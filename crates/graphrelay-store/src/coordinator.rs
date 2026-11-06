use crate::GraphDb;
use graphrelay_core::types::{FileExtraction, IndexedChunk};
use std::path::Path;
use std::sync::Arc;

pub struct StoreCoordinator {
    pub graph: Arc<GraphDb>,
}

impl StoreCoordinator {
    pub fn new(graph: Arc<GraphDb>) -> Self {
        Self { graph }
    }

    /// SQLite write (symbols/edges) followed by the lexical (FTS5) index
    /// write for the same file's chunks. Plan 20 removed the vector-store
    /// phase this used to run second — `graphrelay-watcher`'s
    /// `flush_chunk_batch` builds `IndexedChunk`s and calls
    /// `graph.upsert_fts_batch` directly rather than going through here for
    /// the live/full-index paths; this method remains for any single-file
    /// caller that wants both phases done together.
    pub fn index_file(
        &self,
        extraction: FileExtraction,
        chunks: &[IndexedChunk],
    ) -> anyhow::Result<()> {
        self.graph.replace_file_graph(&extraction.file_record, &extraction.symbols, &extraction.edges)?;
        self.graph.upsert_fts_batch(chunks)?;
        Ok(())
    }

    /// Returns whether an indexed file existed at exactly `path`.
    pub fn delete_file(&self, path: &Path) -> anyhow::Result<bool> {
        if let Some(file) = self.graph.file_by_path(path)? {
            self.graph.delete_file(file.id)?;
            return Ok(true);
        }
        Ok(false)
    }

    // ── Plan 12 Phase C: SCIP semantic layer ─────────────────────────────────

    /// C1: Record SCIP ingest completion stats in index_meta.
    /// Call this after `graphrelay_scip::merge_scip()` succeeds.
    /// Kept separate from the actual ingest to avoid a cyclic crate dependency
    /// (graphrelay-scip depends on graphrelay-store, so store cannot depend on scip).
    pub fn record_scip_ingest(&self, symbols_enriched: usize, edges_added: usize)
        -> anyhow::Result<()>
    {
        self.graph.set_meta("scip_ingested_at",
            &chrono::Utc::now().to_rfc3339())?;
        self.graph.set_meta("scip_symbols_enriched",
            &symbols_enriched.to_string())?;
        self.graph.set_meta("scip_edges_added",
            &edges_added.to_string())?;
        Ok(())
    }

    /// C3: Check whether a `.scip` file is present and newer than the last ingest.
    /// Returns `true` if `graphrelay_scip::merge_scip()` should be called.
    pub fn should_ingest_scip(&self, scip_path: &Path) -> bool {
        if !scip_path.exists() { return false; }
        let Ok(meta) = std::fs::metadata(scip_path) else { return false; };
        let Ok(modified) = meta.modified() else { return true; };
        // Compare against stored timestamp
        match self.graph.get_meta("scip_ingested_at") {
            Ok(Some(ts)) => {
                if let Ok(ingested) = ts.parse::<chrono::DateTime<chrono::Utc>>() {
                    let modified_dt: chrono::DateTime<chrono::Utc> = modified.into();
                    return modified_dt > ingested;
                }
                true
            }
            _ => true, // no previous ingest — always run
        }
    }

    /// C4/C5: Return SCIP status information for the status API and watch_status tool.
    pub fn scip_status(&self) -> ScipStatus {
        let scip_present = self.graph.get_meta("scip_ingested_at")
            .ok().flatten().is_some();
        ScipStatus {
            scip_present,
            scip_ingested_at: self.graph.get_meta("scip_ingested_at")
                .ok().flatten(),
            scip_symbols_enriched: self.graph.get_meta("scip_symbols_enriched")
                .ok().flatten()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        }
    }
}

/// C4/C5: SCIP status returned by the status API and watch_status tool.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ScipStatus {
    pub scip_present:          bool,
    pub scip_ingested_at:      Option<String>,
    pub scip_symbols_enriched: u64,
}
