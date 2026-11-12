use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    ("001_init",                include_str!("migrations/001_init.sql")),
    ("002_scip_foundation",     include_str!("migrations/002_scip_foundation.sql")),
    ("003_lexical_fts",         include_str!("migrations/003_lexical_fts.sql")),
    ("004_fts_name_tokens",     include_str!("migrations/004_fts_name_tokens.sql")),
    ("005_fts_bare_name_only",  include_str!("migrations/005_fts_bare_name_only.sql")),
    ("006_fts_stemmed_tokens",  include_str!("migrations/006_fts_stemmed_tokens.sql")),
    ("007_fts_shared_tokenizer", include_str!("migrations/007_fts_shared_tokenizer.sql")),
    ("008_fts_stemmed_body",    include_str!("migrations/008_fts_stemmed_body.sql")),
    ("009_fts_acronym_digit_tokenization", include_str!("migrations/009_fts_acronym_digit_tokenization.sql")),
    ("010_scip_heal_and_dedupe", include_str!("migrations/010_scip_heal_and_dedupe.sql")),
    ("011_pending_token_and_unresolved_index", include_str!("migrations/011_pending_token_and_unresolved_index.sql")),
    ("012_relative_qualified_names", include_str!("migrations/012_relative_qualified_names.sql")),
    ("013_fts_vocab", include_str!("migrations/013_fts_vocab.sql")),
    ("014_fts_deduped_tokens", include_str!("migrations/014_fts_deduped_tokens.sql")),
    ("015_drop_duplicate_indexes", include_str!("migrations/015_drop_duplicate_indexes.sql")),
    ("016_go_receiver_qualified_names", include_str!("migrations/016_go_receiver_qualified_names.sql")),
    ("017_python_decorated_dedupe", include_str!("migrations/017_python_decorated_dedupe.sql")),
    ("018_edge_resolution", include_str!("migrations/018_edge_resolution.sql")),
];

/// Migrations whose newly-applied run means `chunk_fts` has no (or stale)
/// content for the project's existing symbols. `003` introduces the table from
/// nothing; `004`-`009` and `014` each change what text gets indexed into it without
/// touching the schema — every case needs every existing symbol's FTS row
/// rebuilt. The rebuild is done by resetting every file to the pending sentinel
/// (`checksum = 0`) so the next index re-parses and re-chunks it through the
/// normal `flush_chunk_batch` path, which builds the full graph-aware chunk text
/// (callers/callees/class context). The old startup backfill wrote reduced text
/// (name + snippet only), which measurably degraded retrieval until each file was
/// next edited (Plan 24 finding #5).
const FTS_REBUILD_TRIGGERS: &[&str] = &[
    "003_lexical_fts",
    "004_fts_name_tokens",
    "005_fts_bare_name_only",
    "006_fts_stemmed_tokens",
    "007_fts_shared_tokenizer",
    "008_fts_stemmed_body",
    "009_fts_acronym_digit_tokenization",
    "014_fts_deduped_tokens",
];

pub fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
    )?;

    for (name, sql) in MIGRATIONS {
        let already: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM _migrations WHERE name = ?1",
            rusqlite::params![name],
            |r| r.get(0),
        )?;
        if !already {
            // One transaction per migration: a crash between the DDL and its
            // `_migrations` row used to re-run a non-idempotent `ALTER TABLE ADD
            // COLUMN` on the next open and fail it forever ("duplicate column").
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO _migrations VALUES (?1, datetime('now'))",
                rusqlite::params![name],
            )?;
            // No-op on a brand-new project (no files yet).
            if FTS_REBUILD_TRIGGERS.contains(name) {
                tx.execute("UPDATE files SET checksum = 0", [])?;
            }
            tx.commit()?;
        }
    }

    // A database last opened by a build that still used the reduced-text
    // backfill may carry its flag; honour it the same way, once.
    let legacy_flag: Option<String> = conn
        .query_row("SELECT value FROM index_meta WHERE key = 'fts_backfill_needed'", [], |r| r.get(0))
        .ok();
    if legacy_flag.as_deref() == Some("true") {
        conn.execute("UPDATE files SET checksum = 0", [])?;
        conn.execute("UPDATE index_meta SET value = 'false' WHERE key = 'fts_backfill_needed'", [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_migration_leaves_no_partial_state() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Simulate a migration whose DDL applied but whose record was lost: in a
        // transaction that rolls back, neither must survive.
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute_batch("ALTER TABLE files ADD COLUMN probe INTEGER").unwrap();
        drop(tx);
        let has_probe: bool = conn
            .query_row("SELECT COUNT(*) > 0 FROM pragma_table_info('files') WHERE name = 'probe'", [], |r| r.get(0))
            .unwrap();
        assert!(!has_probe);
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn duplicate_unique_indexes_are_dropped() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('idx_symbols_qualified', 'idx_files_path')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn go_receiver_migration_resets_only_go_files() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        for (id, path, lang) in [(1, "a.go", "go"), (2, "b.rs", "rust")] {
            conn.execute(
                "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size) VALUES (?1, ?2, 99, ?3, 'now', 1)",
                rusqlite::params![id, path, lang],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO index_meta (key, value) VALUES ('scip_ingested_at', 'x')", []).unwrap();
        conn.execute_batch(include_str!("migrations/016_go_receiver_qualified_names.sql")).unwrap();
        let sum = |id: i64| -> i64 {
            conn.query_row("SELECT checksum FROM files WHERE id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(sum(1), 0, "Go file is pending again");
        assert_eq!(sum(2), 99, "other languages are untouched");
        let marker: i64 = conn
            .query_row("SELECT COUNT(*) FROM index_meta WHERE key = 'scip_ingested_at'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(marker, 0, "SCIP marker cleared so semantic edges are re-ingested");
    }

    #[test]
    fn python_decorated_migration_resets_only_python_files() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        for (id, path, lang) in [(1, "a.py", "python"), (2, "b.rs", "rust")] {
            conn.execute(
                "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size) VALUES (?1, ?2, 99, ?3, 'now', 1)",
                rusqlite::params![id, path, lang],
            )
            .unwrap();
        }
        conn.execute_batch(include_str!("migrations/017_python_decorated_dedupe.sql")).unwrap();
        let sum = |id: i64| -> i64 {
            conn.query_row("SELECT checksum FROM files WHERE id = ?1", [id], |r| r.get(0)).unwrap()
        };
        assert_eq!(sum(1), 0, "Python file is pending again");
        assert_eq!(sum(2), 99, "other languages are untouched");
    }
}
