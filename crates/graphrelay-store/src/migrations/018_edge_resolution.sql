-- Plan 37: record how a cross-file edge was resolved (unique / import /
-- tiebreak). NULL = resolved before this column existed. Additive; no reset.
ALTER TABLE edges ADD COLUMN resolution TEXT;
