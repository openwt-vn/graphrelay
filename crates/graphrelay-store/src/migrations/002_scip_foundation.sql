-- Plan 12 Phase A1/A4: add edge provenance and SCIP symbol identifier columns.
-- Existing rows get defaults so the migration is backwards-compatible.

-- A1: edge source provenance ('syntax' | 'semantic' | 'synthesized') — no CHECK
-- constraint on this column, values are validated Rust-side by EdgeSource::from_str.
ALTER TABLE edges ADD COLUMN source TEXT NOT NULL DEFAULT 'syntax';
CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source);

-- A4: SCIP symbol identifier for semantic enrichment
ALTER TABLE symbols ADD COLUMN scip_symbol TEXT;
CREATE INDEX IF NOT EXISTS idx_symbols_scip ON symbols(scip_symbol)
    WHERE scip_symbol IS NOT NULL;
