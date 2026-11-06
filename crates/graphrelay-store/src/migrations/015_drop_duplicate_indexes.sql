-- Both duplicate the automatic index SQLite already builds for the column's
-- UNIQUE constraint, so every symbol/file upsert maintained two identical B-trees.
DROP INDEX IF EXISTS idx_symbols_qualified;
DROP INDEX IF EXISTS idx_files_path;
