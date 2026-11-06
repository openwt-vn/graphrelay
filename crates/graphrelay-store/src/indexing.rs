//! Plan 24 Phase 2 — atomic per-file graph replacement, token-guarded chunk
//! commits, scoped edge resolution and index hygiene.
use crate::graph_db::{
    apply_resolutions, decide_resolutions, import_last_segment, resolve_type_key, write_fts_rows,
    GraphDb, ResolveCand,
};
use graphrelay_core::types::{Edge, FileId, FileRecord, IndexedChunk, Symbol, SymbolId};
use rusqlite::{OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `INDEXED BY` pins the plan: without it the planner prefers `idx_edges_source`
/// (`source = ?` matches every syntax edge) over the selective `from_id` lookup.
pub(crate) const DELETE_SYNTAX_EDGES_OF_SYMBOL_SQL: &str =
    "DELETE FROM edges INDEXED BY idx_edges_from WHERE from_id = ?1 AND source = 'syntax'";

/// SQLite's default host-parameter limit is 32766; stay far below it.
const IN_CHUNK: usize = 500;

/// One file's built chunks, plus the identity of the graph write they came from.
pub struct ChunkCommit {
    pub path:     PathBuf,
    pub token:    u64,
    pub checksum: u64,
    pub chunks:   Vec<IndexedChunk>,
}

/// Replace one file's graph rows inside the caller's transaction.
///
/// Unlike the old delete-everything-then-reinsert, symbols that survive the edit
/// are kept in place (ids are content-addressed), so other files' edges pointing
/// at them are not nulled and SCIP enrichment on them is not lost. Only
/// symbols the edit removed are deleted (with their `chunk_fts` rows, which have
/// no cascade), and only this file's outgoing *syntax* edges are rewritten;
/// semantic and synthesized edges from surviving symbols are left alone.
pub(crate) fn replace_file_in_tx(
    tx: &Transaction,
    file: &FileRecord,
    symbols: &[Symbol],
    edges: &[Edge],
    token: u64,
) -> anyhow::Result<()> {
    tx.prepare_cached(
        "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size, pending_token)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(path) DO UPDATE SET
           checksum=excluded.checksum,
           language=excluded.language,
           indexed_at=excluded.indexed_at,
           byte_size=excluded.byte_size,
           pending_token=excluded.pending_token",
    )?
    .execute(rusqlite::params![
        file.id as i64,
        file.path.to_string_lossy().as_ref(),
        file.checksum as i64,
        file.language.as_str(),
        file.indexed_at,
        file.byte_size as i64,
        token as i64,
    ])?;

    let file_id = file.id as i64;
    let keep: HashSet<i64> = symbols.iter().map(|s| s.id as i64).collect();
    let existing: Vec<i64> = {
        let mut st = tx.prepare_cached("SELECT id FROM symbols WHERE file_id = ?1")?;
        let rows = st.query_map([file_id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    {
        let mut del_fts = tx.prepare_cached("DELETE FROM chunk_fts WHERE rowid = ?1")?;
        let mut del_sym = tx.prepare_cached("DELETE FROM symbols WHERE id = ?1")?;
        for id in existing.iter().filter(|id| !keep.contains(id)) {
            del_fts.execute([id])?;
            del_sym.execute([id])?;
        }
    }
    {
        // Only the surviving symbols can still own edges (a removed symbol's edges went
        // with it). Deleting by `from_id` uses `idx_edges_from`; the earlier form
        // (`source = 'syntax' AND from_id IN (SELECT ...)`) made SQLite scan
        // `idx_edges_source` — every syntax edge — once per file, which is quadratic
        // and cost ~250 s of a 590 s keycloak index.
        let mut del_edges = tx.prepare_cached(DELETE_SYNTAX_EDGES_OF_SYMBOL_SQL)?;
        for id in existing.iter().filter(|id| keep.contains(id)) {
            del_edges.execute([id])?;
        }
    }

    {
        // Where a symbol already carries SCIP data, keep its (more precise) kind
        // and a docstring at least as long as the fresh syntax one.
        let mut symbol_stmt = tx.prepare_cached(
            "INSERT INTO symbols (id, file_id, name, qualified_name, kind, start_byte,
                                 end_byte, start_line, end_line, docstring, language)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(qualified_name) DO UPDATE SET
               file_id=excluded.file_id,
               name=excluded.name,
               kind=CASE WHEN symbols.scip_symbol IS NOT NULL THEN symbols.kind ELSE excluded.kind END,
               start_byte=excluded.start_byte,
               end_byte=excluded.end_byte,
               start_line=excluded.start_line,
               end_line=excluded.end_line,
               docstring=CASE WHEN symbols.scip_symbol IS NOT NULL
                               AND length(COALESCE(symbols.docstring,'')) > length(COALESCE(excluded.docstring,''))
                              THEN symbols.docstring ELSE excluded.docstring END,
               language=excluded.language",
        )?;
        for s in symbols {
            symbol_stmt.execute(rusqlite::params![
                s.id as i64,
                s.file_id as i64,
                s.name,
                s.qualified_name,
                s.kind.as_str(),
                s.span.start_byte,
                s.span.end_byte,
                s.span.start_line,
                s.span.end_line,
                s.docstring,
                s.language.as_str(),
            ])?;
        }
    }
    {
        let mut edge_stmt = tx.prepare_cached(
            "INSERT INTO edges (from_id, to_id, to_name, kind, weight, source)
             VALUES (?1, ?2, ?3, ?4, 1.0, ?5)",
        )?;
        for e in edges {
            edge_stmt.execute(rusqlite::params![
                e.from_id as i64,
                e.to_id.map(|id| id as i64),
                e.to_name,
                e.kind.as_str(),
                e.source.as_str(),
            ])?;
        }
    }
    Ok(())
}

fn in_list(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Unresolved outgoing edges of a set of symbols. `INDEXED BY idx_edges_from`: with
/// `to_id IS NULL` in the WHERE the planner otherwise walks `idx_edges_to` at NULL —
/// every unresolved edge in the corpus (~650k on keycloak) — on every call.
fn sql_pending_from(n: usize) -> String {
    format!(
        "SELECT id, from_id, to_name FROM edges INDEXED BY idx_edges_from
         WHERE from_id IN ({}) AND to_id IS NULL AND to_name IS NOT NULL AND source != 'semantic'",
        in_list(n)
    )
}

/// Unresolved edges elsewhere that name one of `n` target names — served by the partial
/// index `idx_edges_unresolved_name` (migration 011), pinned for the same reason.
fn sql_pending_by_name(n: usize) -> String {
    format!(
        "SELECT id, from_id, to_name FROM edges INDEXED BY idx_edges_unresolved_name
         WHERE to_name IN ({}) AND to_id IS NULL AND source != 'semantic'",
        in_list(n)
    )
}

/// Imports of the given files. `CROSS JOIN` fixes the join order (files' symbols first,
/// then their edges by `idx_edges_from`) instead of scanning every `Imports` edge.
fn sql_imports_of_files(n: usize) -> String {
    format!(
        "SELECT s.file_id, e.to_name FROM symbols s CROSS JOIN edges e ON e.from_id = s.id
         WHERE s.file_id IN ({}) AND e.kind = 'Imports' AND e.to_name IS NOT NULL",
        in_list(n)
    )
}


impl GraphDb {
    /// Live-path single-file graph write: one transaction (was four separate lock
    /// acquisitions, which a concurrent full-index batch could interleave with,
    /// leaving the union of old and new symbols). Returns the write's pending
    /// token — the file stays at `checksum = 0` until `commit_chunk_batch`
    /// accepts chunks built from exactly this token.
    pub fn replace_file_graph(
        &self,
        file: &FileRecord,
        symbols: &[Symbol],
        edges: &[Edge],
    ) -> anyhow::Result<u64> {
        let token = self.next_pending_token();
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        replace_file_in_tx(&tx, file, symbols, edges, token)?;
        tx.commit()?;
        Ok(token)
    }

    /// Atomically write built chunks to `chunk_fts` and commit each file's real
    /// checksum, but only for files whose current `pending_token` still equals
    /// the token the chunks were built from. Token check, FTS write and checksum
    /// commit share one transaction, so a newer graph write can never be
    /// overwritten by (or lose to) a stale builder, and a failed FTS write rolls
    /// the checksums back with it (files stay pending and are retried).
    /// Returns the number of files committed.
    pub fn commit_chunk_batch(&self, items: &[ChunkCommit]) -> anyhow::Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let mut committed = 0;
        for it in items {
            let path = it.path.to_string_lossy();
            let current: Option<i64> = tx
                .query_row(
                    "SELECT pending_token FROM files WHERE path = ?1",
                    [path.as_ref()],
                    |r| r.get(0),
                )
                .optional()?;
            if current != Some(it.token as i64) {
                continue;
            }
            write_fts_rows(&tx, &it.chunks)?;
            tx.execute(
                "UPDATE files SET checksum = ?1, pending_token = 0
                 WHERE path = ?2 AND pending_token = ?3",
                rusqlite::params![it.checksum as i64, path.as_ref(), it.token as i64],
            )?;
            committed += 1;
        }
        tx.commit()?;
        Ok(committed)
    }

    /// Delete `chunk_fts` rows whose symbol no longer exists. `chunk_fts` has no
    /// foreign-key cascade, so any path that ever missed an explicit delete leaves
    /// orphans that keep winning `top_k` slots. Cheap enough for startup.
    pub fn gc_orphan_fts(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM chunk_fts WHERE rowid NOT IN (SELECT id FROM symbols)",
            [],
        )?;
        Ok(n)
    }

    /// Delete every indexed file under `dir` (a removed or renamed directory:
    /// the OS reports only the directory, and exact-path delete matches nothing).
    pub fn delete_files_under(&self, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let prefix = dir.to_string_lossy();
        let prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() {
            return Ok(vec![]);
        }
        // A binary range, not `LIKE`: SQLite's `LIKE` is ASCII case-insensitive,
        // so removing `src/Foo/` also deleted a sibling `src/foo/` on a
        // case-sensitive filesystem. `'0'` is the byte after `'/'`.
        let (lo, hi) = (format!("{prefix}/"), format!("{prefix}0"));
        let ids: Vec<(i64, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut st = conn.prepare("SELECT id, path FROM files WHERE path >= ?1 AND path < ?2")?;
            let rows = st.query_map([lo, hi], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut out = Vec::with_capacity(ids.len());
        for (id, path) in ids {
            self.delete_file(id as FileId)?;
            out.push(PathBuf::from(path));
        }
        Ok(out)
    }

    /// Number of outgoing edges (any source, resolved or not) from `from_id`.
    pub fn edge_count_from(&self, from_id: SymbolId) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE from_id = ?1",
            [from_id as i64],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Every indexed file path (used to prune files that vanished while nothing
    /// was watching).
    pub fn all_file_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let conn = self.conn.lock().unwrap();
        let mut st = conn.prepare("SELECT path FROM files")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?.into_iter().map(PathBuf::from).collect())
    }

    /// `resolve_edges` limited to what a set of changed files can affect: their
    /// own unresolved outgoing edges, plus unresolved edges elsewhere whose target
    /// name matches a symbol in those files. Decisions come from the same pure
    /// function as the full pass (`decide_resolutions`), so both agree on any edge
    /// they both see. The full pass rescans every symbol and all ~650k unresolved
    /// edges on a large repo (0.6-0.7 s on keycloak) while holding the single
    /// connection mutex, which stalled every concurrent query after each save.
    pub fn resolve_edges_for_files(&self, file_ids: &[FileId]) -> anyhow::Result<usize> {
        if file_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();

        let mut scope_ids: Vec<i64> = Vec::new();
        let mut scope_names: HashSet<String> = HashSet::new();
        for chunk in file_ids.chunks(IN_CHUNK) {
            let sql = format!("SELECT id, name FROM symbols WHERE file_id IN ({})", in_list(chunk.len()));
            let mut st = conn.prepare(&sql)?;
            let params: Vec<i64> = chunk.iter().map(|f| *f as i64).collect();
            let mut rows = st.query(rusqlite::params_from_iter(params.iter()))?;
            while let Some(r) = rows.next()? {
                scope_ids.push(r.get(0)?);
                scope_names.insert(r.get(1)?);
            }
        }

        let mut pending: HashMap<i64, (SymbolId, String)> = HashMap::new();
        let mut collect = |sql: String, params: Vec<rusqlite::types::Value>| -> anyhow::Result<()> {
            let mut st = conn.prepare(&sql)?;
            let mut rows = st.query(rusqlite::params_from_iter(params.iter()))?;
            while let Some(r) = rows.next()? {
                let edge_id: i64 = r.get(0)?;
                let from_id: i64 = r.get(1)?;
                let to_name: String = r.get(2)?;
                pending.insert(edge_id, (from_id as SymbolId, to_name));
            }
            Ok(())
        };
        for chunk in scope_ids.chunks(IN_CHUNK) {
            collect(sql_pending_from(chunk.len()), chunk.iter().map(|i| (*i).into()).collect())?;
        }
        let names: Vec<&String> = scope_names.iter().collect();
        for chunk in names.chunks(IN_CHUNK) {
            collect(sql_pending_by_name(chunk.len()), chunk.iter().map(|n| n.as_str().to_string().into()).collect())?;
        }
        if pending.is_empty() {
            return Ok(0);
        }
        let mut pending_vec: Vec<(i64, SymbolId, String)> =
            pending.into_iter().map(|(id, (from, name))| (id, from, name)).collect();
        pending_vec.sort_by_key(|p| p.0);

        // FROM side: language + file of each distinct from symbol.
        let mut sym_lang: HashMap<SymbolId, String> = HashMap::new();
        let mut sym_file: HashMap<SymbolId, FileId> = HashMap::new();
        let from_ids: Vec<i64> = pending_vec
            .iter()
            .map(|p| p.1 as i64)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for chunk in from_ids.chunks(IN_CHUNK) {
            let sql = format!("SELECT id, language, file_id FROM symbols WHERE id IN ({})", in_list(chunk.len()));
            let mut st = conn.prepare(&sql)?;
            let mut rows = st.query(rusqlite::params_from_iter(chunk.iter()))?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let fid: i64 = r.get(2)?;
                sym_lang.insert(id as SymbolId, r.get(1)?);
                sym_file.insert(id as SymbolId, fid as FileId);
            }
        }

        // TO side: candidates for exactly the names in play.
        let mut by_name_lang: HashMap<(String, String), Vec<ResolveCand>> = HashMap::new();
        let target_names: Vec<String> = pending_vec
            .iter()
            .map(|p| p.2.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for chunk in target_names.chunks(IN_CHUNK) {
            let sql = format!(
                "SELECT s.id, s.name, s.qualified_name, s.language, f.path
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.name IN ({}) ORDER BY s.id",
                in_list(chunk.len())
            );
            let mut st = conn.prepare(&sql)?;
            let mut rows = st.query(rusqlite::params_from_iter(chunk.iter()))?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let name: String = r.get(1)?;
                let qualified_name: String = r.get(2)?;
                let language: String = r.get(3)?;
                let path: String = r.get(4)?;
                by_name_lang.entry((name, language)).or_default().push(ResolveCand {
                    id: id as SymbolId,
                    in_degree: 0,
                    type_key: resolve_type_key(&qualified_name, &path),
                });
            }
        }
        let cand_ids: Vec<i64> = by_name_lang
            .values()
            .flatten()
            .map(|c| c.id as i64)
            .collect();
        let mut degrees: HashMap<SymbolId, i64> = HashMap::new();
        for chunk in cand_ids.chunks(IN_CHUNK) {
            let sql = format!(
                "SELECT to_id, COUNT(*) FROM edges WHERE to_id IN ({}) GROUP BY to_id",
                in_list(chunk.len())
            );
            let mut st = conn.prepare(&sql)?;
            let mut rows = st.query(rusqlite::params_from_iter(chunk.iter()))?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                degrees.insert(id as SymbolId, r.get(1)?);
            }
        }
        for c in by_name_lang.values_mut().flatten() {
            c.in_degree = degrees.get(&c.id).copied().unwrap_or(0);
        }

        // Imports of the importing files.
        let from_files: Vec<i64> = sym_file
            .values()
            .map(|f| *f as i64)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut imports_by_file: HashMap<FileId, Vec<String>> = HashMap::new();
        for chunk in from_files.chunks(IN_CHUNK) {
            let sql = sql_imports_of_files(chunk.len());
            let mut st = conn.prepare(&sql)?;
            let mut rows = st.query(rusqlite::params_from_iter(chunk.iter()))?;
            while let Some(r) = rows.next()? {
                let fid: i64 = r.get(0)?;
                let raw: String = r.get(1)?;
                let last = import_last_segment(&raw);
                if !last.is_empty() {
                    imports_by_file.entry(fid as FileId).or_default().push(last);
                }
            }
        }

        let resolutions = decide_resolutions(&pending_vec, &by_name_lang, &sym_lang, &sym_file, &imports_by_file);
        apply_resolutions(&conn, &resolutions)?;
        Ok(resolutions.len())
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    /// A per-file statement that scans the whole `edges` table turns a full index
    /// quadratic (it did: +250 s on a 10k-file Java repo, invisible on small corpora).
    #[test]
    fn deleting_a_symbols_syntax_edges_uses_the_from_id_index() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrations::run_migrations(&conn).unwrap();
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {DELETE_SYNTAX_EDGES_OF_SYMBOL_SQL}"))
            .unwrap()
            .query_map([1i64], |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let plan = plan.join(" | ");
        assert!(plan.contains("idx_edges_from"), "plan: {plan}");
        assert!(!plan.contains("idx_edges_source"), "plan: {plan}");
    }

    fn plan_of(conn: &rusqlite::Connection, sql: &str) -> String {
        conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// The scoped resolver runs after every save; none of its lookups may walk a
    /// corpus-sized index (all unresolved edges, or all `Imports` edges).
    #[test]
    fn scoped_resolution_queries_use_selective_indexes() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrations::run_migrations(&conn).unwrap();
        // Bound parameters would be NULL in a plan check, so use literal lists of the same shape.
        let lit = |sql: String| {
            let mut out = String::new();
            let mut n = 0;
            for ch in sql.chars() {
                if ch == '?' { n += 1; out.push_str(&n.to_string()); } else { out.push(ch); }
            }
            out
        };
        let from = plan_of(&conn, &lit(sql_pending_from(3)));
        assert!(from.contains("idx_edges_from") && !from.contains("idx_edges_to"), "plan: {from}");
        let by_name = plan_of(&conn, &lit(sql_pending_by_name(3)));
        assert!(by_name.contains("idx_edges_unresolved_name"), "plan: {by_name}");
        let imports = plan_of(&conn, &lit(sql_imports_of_files(3)));
        assert!(imports.contains("idx_symbols_file_id") && !imports.contains("idx_edges_kind"), "plan: {imports}");
    }
}
