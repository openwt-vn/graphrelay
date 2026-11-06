-- Plan 24 Phase 2.
-- pending_token: which graph write a file's pending (checksum 0) state belongs to.
-- The shared checksum-0 sentinel could not tell writers apart, so a stale chunk
-- build could commit over a newer graph write. Each graph write now stamps a
-- fresh token; chunk commits are accepted only for the token they were built from.
ALTER TABLE files ADD COLUMN pending_token INTEGER NOT NULL DEFAULT 0;

-- Scoped edge resolution looks up unresolved edges by target name.
CREATE INDEX IF NOT EXISTS idx_edges_unresolved_name
    ON edges(to_name) WHERE to_id IS NULL;
