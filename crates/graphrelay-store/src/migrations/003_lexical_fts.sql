-- Lexical candidate-generation index: promotes keyword search from a rerank-only
-- signal to a real retrieval stage. Without this, `find_relevant_code` can only
-- rerank whatever `vector_search` happened to return in top_k — when the embedder
-- is the zero-vector stub (no ONNX model present), that seed set is effectively
-- arbitrary and no amount of downstream BM25/graph reranking can recover a
-- relevant symbol that never made it into the pool.
--
-- rowid is set explicitly to the symbol's `SymbolId` (xxhash64(qualified_name),
-- cast bit-for-bit to i64, same convention as `symbols.id` in 001_init.sql) so
-- re-indexing a symbol is an O(1) `INSERT OR REPLACE` by rowid instead of an
-- O(n) FTS5 column scan to find the old row to delete.
--
-- bm25() (SQLite's built-in ranking function) gives real corpus-level IDF —
-- unlike the hand-rolled `bm25_score_terms()` reranker in graphrelay-core, which
-- approximates IDF as 1.0 because it only ever sees one already-retrieved
-- document at a time, not the whole corpus.
CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
    qualified_name,
    file_path,
    chunk_text,
    tokenize = 'unicode61 remove_diacritics 2'
);
