-- Plan 24 Phase 1: heal databases that carry the pre-fix SCIP layer.
--
-- The old merger resolved SCIP `local N` symbols globally, matched enrichment by
-- line only, and let `resolve_edges` name-resolve semantic edges, so existing
-- semantic edges / scip_symbol values can be wrong and chunk text built from them
-- is stale. Only projects that actually ingested SCIP are touched: their files are
-- reset to the pending sentinel (checksum 0) so the next index rebuilds symbols,
-- edges and chunk text through the normal path, and the ingest marker is cleared
-- so the fixed merger re-runs against the existing .scip file.
UPDATE files SET checksum = 0
 WHERE EXISTS (SELECT 1 FROM index_meta WHERE key = 'scip_ingested_at');
DELETE FROM edges WHERE source = 'semantic';
UPDATE symbols SET scip_symbol = NULL WHERE scip_symbol LIKE 'local %';
DELETE FROM index_meta WHERE key IN ('scip_ingested_at', 'scip_symbols_enriched', 'scip_edges_added');

-- Semantic-edge identity. Scoped to source = 'semantic' on purpose: syntax edges
-- legitimately repeat per call site and changing that would shift in-degree/PPR.
-- Expression index because a plain UNIQUE treats NULL to_id/to_name as distinct.
CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_semantic_unique
    ON edges(from_id, COALESCE(to_id, -1), COALESCE(to_name, ''), kind)
    WHERE source = 'semantic';
