-- Content-format bump, not a schema change: `tokenize_name` (used by
-- `GraphDb::upsert_fts_batch` to decompose the bare identifier) now stems
-- each token (Porter/Snowball via `rust-stemmers`) so a query using a
-- different inflection of the same word ("resets" vs. `resetEditor`'s
-- "reset") still matches. Existing rows only have the old, un-stemmed
-- decomposed tokens — clear them and let the startup backfill
-- (`fts_backfill_needed`, re-triggered for this migration too) repopulate
-- with stemmed text.
DELETE FROM chunk_fts;
