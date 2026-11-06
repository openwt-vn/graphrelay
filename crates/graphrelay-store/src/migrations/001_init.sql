CREATE TABLE IF NOT EXISTS files (
    id          INTEGER PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    checksum    INTEGER NOT NULL,
    language    TEXT NOT NULL,
    indexed_at  TEXT NOT NULL,
    byte_size   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS symbols (
    id              INTEGER PRIMARY KEY,
    file_id         INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    qualified_name  TEXT NOT NULL UNIQUE,
    kind            TEXT NOT NULL,
    start_byte      INTEGER NOT NULL,
    end_byte        INTEGER NOT NULL,
    start_line      INTEGER NOT NULL,
    end_line        INTEGER NOT NULL,
    docstring       TEXT,
    language        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS edges (
    id       INTEGER PRIMARY KEY,
    from_id  INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    to_id    INTEGER REFERENCES symbols(id) ON DELETE SET NULL,
    to_name  TEXT,
    kind     TEXT NOT NULL,
    weight   REAL NOT NULL DEFAULT 1.0
);

CREATE TABLE IF NOT EXISTS index_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_symbols_file_id   ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_name      ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbols(qualified_name);
CREATE INDEX IF NOT EXISTS idx_edges_from        ON edges(from_id);
CREATE INDEX IF NOT EXISTS idx_edges_to          ON edges(to_id);
CREATE INDEX IF NOT EXISTS idx_edges_kind        ON edges(kind);
CREATE INDEX IF NOT EXISTS idx_files_path        ON files(path);
