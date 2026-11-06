-- Plan 24 item 3.4: per-term document frequency for the lexical index, so query
-- terms that appear in a large fraction of all symbols (they contribute almost
-- no BM25 signal but have the biggest posting lists) can be skipped before the
-- FTS5 MATCH. A read-only view over the existing index — no extra storage, no
-- backfill, and nothing to keep in sync.
CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts_vocab USING fts5vocab(chunk_fts, 'row');
