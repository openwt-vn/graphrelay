-- Plan 29: Go methods are now qualified by their receiver type
-- (`path::Receiver::Method`, was `path::Method[#N]`), so their symbol ids
-- (xxhash64 of the qualified name) change. Reset Go files to the pending
-- sentinel so the next index re-parses them; the old-id symbols are removed as
-- stale (with their chunk_fts rows) and the new-id symbols inserted. Semantic
-- (SCIP) edges hang off the old ids, so the ingest marker is cleared for
-- projects that have Go files and the existing .scip file is re-ingested.
-- No-op on a brand-new project.
UPDATE files SET checksum = 0 WHERE language = 'go';
DELETE FROM index_meta
 WHERE key IN ('scip_ingested_at', 'scip_symbols_enriched', 'scip_edges_added')
   AND EXISTS (SELECT 1 FROM files WHERE language = 'go');
