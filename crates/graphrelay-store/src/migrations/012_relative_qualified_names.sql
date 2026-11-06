-- Plan 24 Phase 4: qualified names (and therefore symbol ids, xxhash64 of the
-- qualified name) are now relative to the project root instead of embedding the
-- absolute checkout path. Every existing symbol carries an absolute-path id, so
-- reset all files to the pending sentinel: the next index re-parses them, the
-- old-id symbols are removed as stale (with their chunk_fts rows) and the new-id
-- symbols inserted. The SCIP ingest marker is cleared because semantic edges hang
-- off the old ids; the existing .scip file is re-ingested against the new ones.
-- No-op on a brand-new project.
UPDATE files SET checksum = 0;
DELETE FROM index_meta WHERE key IN ('scip_ingested_at', 'scip_symbols_enriched', 'scip_edges_added');
