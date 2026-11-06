# Plan 03 — Storage Backend (SQLite + LanceDB)

## Goal
Design and implement the dual-store backend: SQLite for the relational code graph (symbols, edges, files) and LanceDB for vector embeddings. Both stores must support atomic coordinated writes and fast incremental updates.

## Success Criteria
- Full index of a 100k LOC repo fits in < 500 MB on disk
- Symbol lookup by ID or name: < 1 ms
- Neighbor query (callers + callees of a symbol): < 5 ms
- LanceDB ANN search (top-20 from 1M vectors): < 50 ms
- Coordinated upsert (SQLite + LanceDB together) for a single file: < 100 ms

---

## 1. SQLite Schema

```sql
-- migrations/001_init.sql

CREATE TABLE IF NOT EXISTS files (
    id          INTEGER PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    checksum    INTEGER NOT NULL,          -- xxhash64 of file content
    language    TEXT NOT NULL,
    indexed_at  TEXT NOT NULL,             -- ISO 8601
    byte_size   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS symbols (
    id              INTEGER PRIMARY KEY,
    file_id         INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    qualified_name  TEXT NOT NULL UNIQUE,  -- "src/foo.rs::Bar::baz"
    kind            TEXT NOT NULL,         -- Function | Class | Struct | Enum | Trait | Const | ...
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
    to_id    INTEGER REFERENCES symbols(id) ON DELETE SET NULL,  -- NULL if unresolved
    to_name  TEXT,                                               -- raw name if unresolved
    kind     TEXT NOT NULL,          -- Calls | Imports | References | Inherits | Implements
    weight   REAL NOT NULL DEFAULT 1.0
    -- weight: reserved for v2 weighted graph analytics (call frequency, confidence score).
    -- In v1 all edges have weight=1.0 and the retrieval pipeline uses in-degree count, not weight.
    -- Do NOT use weight in v1 scoring — it will always be 1.0.
);

CREATE TABLE IF NOT EXISTS index_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Indexes for hot query paths
CREATE INDEX IF NOT EXISTS idx_symbols_file_id       ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_name          ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_qualified     ON symbols(qualified_name);
CREATE INDEX IF NOT EXISTS idx_edges_from            ON edges(from_id);
CREATE INDEX IF NOT EXISTS idx_edges_to              ON edges(to_id);
CREATE INDEX IF NOT EXISTS idx_edges_kind            ON edges(kind);
CREATE INDEX IF NOT EXISTS idx_files_path            ON files(path);
```

**Symbol ID strategy**: `symbols.id` is a **content-addressed `u64`** computed as `xxhash64(qualified_name_bytes)` — not SQLite autoincrement. This ensures ID stability across incremental re-index runs, enabling `ON CONFLICT(qualified_name) DO UPDATE` upserts and reliable LanceDB delete-by-symbol_id. See Plan 02 §7b for details and collision handling.

---

## 2. SQLite Access Layer

```rust
// crates/codegraph-store/src/graph_db.rs

pub struct GraphDb {
    conn: Mutex<rusqlite::Connection>,
}

impl GraphDb {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
        run_migrations(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    // ── File operations ──────────────────────────────────────────────────────

    pub fn upsert_file(&self, file: &FileRecord) -> anyhow::Result<FileId> { .. }
    pub fn file_by_path(&self, path: &Path) -> anyhow::Result<Option<FileRecord>> { .. }
    pub fn delete_file(&self, file_id: FileId) -> anyhow::Result<()> { .. }
    pub fn file_checksum(&self, path: &Path) -> anyhow::Result<Option<u64>> { .. }

    // ── Symbol operations ────────────────────────────────────────────────────

    pub fn upsert_symbols(&self, symbols: &[Symbol]) -> anyhow::Result<()> { .. }
    pub fn symbol_by_id(&self, id: SymbolId) -> anyhow::Result<Option<Symbol>> { .. }
    pub fn symbol_by_qualified(&self, qname: &str) -> anyhow::Result<Option<Symbol>> { .. }
    pub fn symbols_in_file(&self, file_id: FileId) -> anyhow::Result<Vec<Symbol>> { .. }
    pub fn search_symbols_by_name(&self, pattern: &str, limit: usize) -> anyhow::Result<Vec<Symbol>> { .. }
    pub fn delete_symbols_for_file(&self, file_id: FileId) -> anyhow::Result<()> { .. }

    // ── Edge operations ──────────────────────────────────────────────────────

    pub fn upsert_edges(&self, edges: &[Edge]) -> anyhow::Result<()> { .. }
    pub fn callers_of(&self, symbol_id: SymbolId) -> anyhow::Result<Vec<Symbol>> { .. }
    pub fn callees_of(&self, symbol_id: SymbolId) -> anyhow::Result<Vec<Symbol>> { .. }
    pub fn neighbors(&self, symbol_id: SymbolId, hops: u8) -> anyhow::Result<Vec<Symbol>> { .. }
    pub fn impact_graph(&self, symbol_id: SymbolId, depth: u8) -> anyhow::Result<ImpactGraph> { .. }
    pub fn delete_edges_for_file(&self, file_id: FileId) -> anyhow::Result<()> { .. }

    // ── Graph analytics (basic) ──────────────────────────────────────────────

    pub fn in_degree(&self, symbol_id: SymbolId) -> anyhow::Result<u32> { .. }
    pub fn out_degree(&self, symbol_id: SymbolId) -> anyhow::Result<u32> { .. }
    pub fn top_by_in_degree(&self, limit: usize) -> anyhow::Result<Vec<(Symbol, u32)>> { .. }

    // ── Index metadata ───────────────────────────────────────────────────────

    pub fn set_meta(&self, key: &str, value: &str) -> anyhow::Result<()> { .. }
    pub fn get_meta(&self, key: &str) -> anyhow::Result<Option<String>> { .. }
}
```

### WAL Mode Rationale
WAL (Write-Ahead Logging) allows concurrent readers while a single writer updates the DB, which is critical because the watcher thread writes while the HTTP server reads simultaneously.

---

## 3. Batch Upsert Pattern

Use prepared statements + transactions for bulk inserts:

```rust
pub fn upsert_symbols_batch(&self, symbols: &[Symbol]) -> anyhow::Result<()> {
    let conn = self.conn.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, start_byte, end_byte,
                                  start_line, end_line, docstring, language)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(qualified_name) DO UPDATE SET
               start_byte=excluded.start_byte, end_byte=excluded.end_byte,
               start_line=excluded.start_line, end_line=excluded.end_line,
               docstring=excluded.docstring"
        )?;
        for s in symbols {
            stmt.execute(rusqlite::params![
                s.id, s.file_id, s.name, s.qualified_name, s.kind.as_str(),
                s.span.start_byte, s.span.end_byte, s.span.start_line, s.span.end_line,
                s.docstring, s.language.as_str()
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}
```

---

## 4. LanceDB Table Design

```rust
// crates/codegraph-store/src/vector_db.rs

// Arrow schema for the embeddings table
fn embedding_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("symbol_id",     DataType::UInt64, false),
        Field::new("qualified_name",DataType::Utf8,   false),
        Field::new("chunk_text",    DataType::Utf8,   false),
        Field::new("file_path",     DataType::Utf8,   false),
        Field::new("start_line",    DataType::UInt32,  false),
        Field::new("language",      DataType::Utf8,   false),
        Field::new("vector",        DataType::FixedSizeList(
            Arc::new(Field::new("item", DataType::Float32, true)),
            dim as i32,
        ), false),
    ]))
}

pub struct VectorDb {
    table: lancedb::Table,
    dim:   usize,
}

impl VectorDb {
    pub async fn open_or_create(uri: &str, dim: usize) -> anyhow::Result<Self> {
        let conn  = lancedb::connect(uri).execute().await?;
        let table = match conn.open_table("embeddings").execute().await {
            Ok(t)  => t,
            Err(_) => conn.create_table("embeddings", embedding_schema(dim)).execute().await?,
        };
        Ok(Self { table, dim })
    }

    pub async fn upsert(&self, chunks: &[EmbeddedChunk]) -> anyhow::Result<()> {
        // Delete old vectors for these symbol_ids, then insert
        let ids: Vec<u64> = chunks.iter().map(|c| c.symbol_id).collect();
        self.table
            .delete(&format!("symbol_id IN ({})", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")))
            .await?;

        let batch = chunks_to_record_batch(chunks, self.dim)?;
        self.table.add(vec![batch]).execute().await?;
        Ok(())
    }

    pub async fn search(
        &self,
        query_vector: &[f32],
        top_k: usize,
        filter: Option<&str>,  // LanceDB SQL filter, e.g. "language = 'rust'"
    ) -> anyhow::Result<Vec<VectorHit>> {
        let mut q = self.table
            .query()
            .nearest_to(query_vector)?
            .limit(top_k)
            .distance_type(lancedb::DistanceType::Cosine);

        if let Some(f) = filter {
            q = q.only_if(f);
        }

        let results = q.execute().await?
            .try_collect::<Vec<_>>().await?;

        Ok(record_batches_to_hits(results))
    }

    pub async fn delete_by_file(&self, file_path: &str) -> anyhow::Result<()> {
        self.table
            .delete(&format!("file_path = '{}'", file_path.replace('\'', "''")))
            .await?;
        Ok(())
    }

    /// Track upsert count since last IVF-PQ index build.
    /// Recreate the ANN index when > 10% of the corpus has changed.
    pub async fn maybe_rebuild_index(&self, total_vectors: usize, upserted_since_build: usize) -> anyhow::Result<()> {
        if total_vectors == 0 { return Ok(()); }
        let pct = upserted_since_build as f64 / total_vectors as f64;
        if pct > 0.10 {
            tracing::info!("rebuilding LanceDB IVF-PQ index ({:.0}% updated)", pct * 100.0);
            self.table
                .create_index(&["vector"], lancedb::index::Index::IvfPq(
                    lancedb::index::IvfPqIndexBuilder::default()
                        .num_partitions(256)
                        .num_sub_vectors(96)
                ))
                .execute()
                .await?;
            // Reset counter via caller storing new baseline in index_meta
        }
        Ok(())
    }
}
```

The `StoreCoordinator` tracks `upserted_since_build` in `index_meta` table (key: `"vector_upsert_count"`) and calls `maybe_rebuild_index()` after each `full_index()` completion and after every 1,000 incremental upserts.

---

## 5. Coordinated Write (`transaction.rs`)

The watcher drives coordinated writes. Both stores must be updated atomically from the application's perspective (SQLite has transactions; LanceDB does not, so we do best-effort with rollback):

```rust
pub struct StoreCoordinator {
    graph:  Arc<GraphDb>,
    vector: Arc<VectorDb>,
}

impl StoreCoordinator {
    pub async fn index_file(
        &self,
        extraction: FileExtraction,
        embedded_chunks: Vec<EmbeddedChunk>,
    ) -> anyhow::Result<()> {
        // Phase 1: SQLite (transactional)
        {
            let file_id = self.graph.upsert_file(&extraction.file_record)?;
            self.graph.delete_symbols_for_file(file_id)?;  // cascades to edges
            self.graph.upsert_symbols_batch(&extraction.symbols)?;
            self.graph.upsert_edges(&extraction.edges)?;
        }

        // Phase 2: LanceDB (best-effort; if this fails, mark index as dirty)
        match self.vector.upsert(&embedded_chunks).await {
            Ok(_)  => {},
            Err(e) => {
                tracing::error!("vector upsert failed for {:?}: {e}", extraction.file_record.path);
                self.graph.set_meta("vector_dirty", "true")?;
            }
        }

        Ok(())
    }

    pub async fn delete_file(&self, path: &Path) -> anyhow::Result<()> {
        let path_str = path.to_string_lossy();
        if let Some(file) = self.graph.file_by_path(path)? {
            self.graph.delete_file(file.id)?;          // cascades symbols + edges
        }
        self.vector.delete_by_file(&path_str).await?;
        Ok(())
    }
}
```

---

## 6. Migration System

Migrations are embedded SQL files, run in order on startup:

```rust
const MIGRATIONS: &[(&str, &str)] = &[
    ("001_init",               include_str!("migrations/001_init.sql")),
    ("002_add_weight_to_edges",include_str!("migrations/002_add_weight_to_edges.sql")),
];

fn run_migrations(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)"
    )?;

    for (name, sql) in MIGRATIONS {
        let already: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM _migrations WHERE name = ?1",
            [name], |r| r.get(0)
        )?;
        if !already {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO _migrations VALUES (?1, datetime('now'))",
                [name]
            )?;
        }
    }
    Ok(())
}
```

---

## 7. Storage Location

Default: `~/.codegraph/<project-hash>/`

```
~/.codegraph/
  a3f9c1e7/                  # sha256(canonicalized workspace root)[:8]
    graph.db                 # SQLite
    vectors/                 # LanceDB directory
    models/                  # downloaded ONNX weights (cached)
```

Configurable via `CODEGRAPH_DATA_DIR` env var or `config.toml`.

---

## 8. Neighbor Query (Graph Expansion)

The most frequent hot-path query in retrieval is "give me callers + callees for these N symbols":

```sql
-- Callers of symbol_id :id (symbols that call :id)
SELECT s.*
FROM   symbols s
JOIN   edges e ON e.from_id = s.id
WHERE  e.to_id = :id
  AND  e.kind  = 'Calls';

-- Callees of symbol_id :id (symbols that :id calls)
SELECT s.*
FROM   symbols s
JOIN   edges e ON e.to_id = s.id
WHERE  e.from_id = :id
  AND  e.kind    = 'Calls';
```

Both queries use the `idx_edges_from` and `idx_edges_to` indexes.

For multi-hop BFS (used in `trace_impact`):
```sql
-- Recursive CTE for impact traversal (up to N hops)
WITH RECURSIVE impact(id, depth) AS (
    SELECT :start_id, 0
    UNION ALL
    SELECT e.from_id, i.depth + 1
    FROM   impact i
    JOIN   edges e ON e.to_id = i.id
    WHERE  i.depth < :max_depth
      AND  e.kind = 'Calls'
)
SELECT DISTINCT s.* FROM impact i JOIN symbols s ON s.id = i.id;
```

---

## 9. LanceDB Index Creation

After initial bulk load, create an ANN index for fast approximate search:

```rust
self.table
    .create_index(&["vector"], lancedb::index::Index::IvfPq(
        lancedb::index::IvfPqIndexBuilder::default()
            .num_partitions(256)
            .num_sub_vectors(96)
    ))
    .execute()
    .await?;
```

Recreate the index after bulk re-indexing (triggered when > 10% of vectors are updated).

---

## 10. Implementation Steps

1. Write `migrations/001_init.sql`
2. Implement `GraphDb::open()` with WAL + migration runner
3. Implement all `GraphDb` methods (file, symbol, edge CRUD + analytics)
4. Implement `VectorDb::open_or_create()` and schema
5. Implement `VectorDb::upsert()`, `search()`, `delete_by_file()`
6. Implement `StoreCoordinator::index_file()` and `delete_file()`
7. Integration test: index a fixture project, verify SQLite row counts and LanceDB vector counts match
8. Benchmark: upsert 10,000 symbols → must be < 500 ms; ANN search 1M vectors → < 50 ms

---

## 11. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| SQLite connection pool | `Mutex<Connection>` vs `r2d2` pool | `Mutex` for now — watcher is single writer, HTTP reads are fast |
| LanceDB URI | local path vs. cloud | Always local path (`file://~/.codegraph/...`) |
| Vector dim | 384 (bge-small) vs 768 (bge-large) | 384 — matches bge-small-en-v1.5, good perf/quality trade-off |
| ANN index type | IVF-PQ vs HNSW | IVF-PQ for LanceDB (native); HNSW is supported but slower to build |
| Cascade delete strategy | `ON DELETE CASCADE` in SQL | Yes — deleting a file auto-removes its symbols and edges |
