-- Plan 32: a decorated Python function was indexed twice (`name#1` bare def,
-- `name#2` decorated wrapper). It is now one symbol with the wrapper's span,
-- so ids change. Reset Python files to the pending sentinel so the next index
-- re-parses them; the old-id symbols are removed as stale (with their
-- chunk_fts rows). Cleared SCIP markers re-ingest semantic edges that hung off
-- the old ids. No-op on a brand-new project.
UPDATE files SET checksum = 0 WHERE language = 'python';
DELETE FROM index_meta
 WHERE key IN ('scip_ingested_at', 'scip_symbols_enriched', 'scip_edges_added')
   AND EXISTS (SELECT 1 FROM files WHERE language = 'python');
