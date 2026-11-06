use graphrelay_core::retrieval::tokenize_name;
use graphrelay_core::types::{
    Edge, EdgeKind, EdgeSource, FileId, FileRecord, ImpactGraph, ImpactNode, IndexedChunk, Language, Span,
    Symbol, SymbolId, SymbolKind,
};

/// Default cap on symbols an impact traversal visits.
pub const IMPACT_NODE_BUDGET: usize = 5000;
/// `(caller names, callee names)` for one symbol.
pub type CallNeighborNames = (Vec<String>, Vec<String>);

/// Cap on `implementors_of` rows.
pub const IMPLEMENTORS_LIMIT: usize = 50;
// EdgeSource is used in row_to_edge and SCIP helper methods
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::migrations::run_migrations;

/// Weight applied to a synonym-only FTS match relative to an original-term
/// match (Plan 21 item 14 — see `lexical_search`'s doc comment). Picked from
/// the audit's suggested 0.3-0.5 range, not independently swept against
/// neighboring values yet — treat as provisional the same way
/// `DIRECT_HIT_GRAPH_CREDIT_DAMPING` was before its own dedicated sweep.
const SYNONYM_TERM_WEIGHT: f32 = 0.4;

pub struct GraphDb {
    pub(crate) conn: Mutex<Connection>,
    /// Source of per-write `files.pending_token` values (Plan 24 item 2.3).
    /// Seeded from the wall clock so tokens from a previous process are always
    /// smaller than any issued now.
    pending_seq: std::sync::atomic::AtomicU64,
    /// (measured at, number of indexed chunks) — see `fts_total_rows`.
    fts_rows_cache: Mutex<Option<(std::time::Instant, f64)>>,
    /// term -> document frequency, dropped wholesale every `FTS_ROWS_CACHE_TTL`.
    /// One vocab lookup costs ~2 ms, so a 47-term query paid ~100 ms uncached.
    fts_df_cache: Mutex<(Option<std::time::Instant>, std::collections::HashMap<String, i64>)>,
}

fn initial_pending_seq() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1)
        .max(1)
}

/// Tokens produced by decomposition/stemming that are not already a raw word of
/// `text`. Appending all of them counted every uninflected word twice (raw +
/// identical stem) but an inflected one once, biasing BM25 toward uninflected
/// words and doubling document length (Plan 24 item 3.3; measured net-positive:
/// normalized MRR lexical -0.010, erpnext +0.012, keycloak +0.035).
pub(crate) fn appended_tokens(text: &str) -> String {
    let raw: std::collections::HashSet<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    tokenize_name(text)
        .into_iter()
        .filter(|t| !raw.contains(t))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod appended_tokens_tests {
    use super::appended_tokens;

    #[test]
    fn adds_only_tokens_that_are_not_already_raw_words() {
        assert_eq!(appended_tokens("reset editor"), "", "already raw words: nothing to add");
        assert_eq!(appended_tokens("resetEditor"), "reset editor", "the compound is split");
        assert_eq!(appended_tokens("creates"), "creat", "an inflected word gains its stem once");
        assert_eq!(appended_tokens("reset resetEditor"), "editor", "reset is already a raw word");
    }
}

/// A term present in more than this fraction of all indexed chunks is skipped
/// before the FTS5 MATCH (Plan 24 item 3.4). Such terms carry almost no BM25 signal
/// (IDF near zero) but have the largest posting lists; on keycloak they dominated
/// query time. Swept on keycloak/erpnext/lexical (normalized MRR, no cap -> 0.2):
/// 0.723->0.749, 0.2405->0.2474, 0.761->0.750 with keycloak p50 527->269 ms;
/// 0.5 is a ranking no-op, 0.1 starts to cost lexical. See the Plan 24 report.
const FTS_MAX_DF_FRACTION: f64 = 0.2;

/// Below this many indexed chunks document frequencies are too coarse to trust
/// (every term in one of three documents is "33%"), so no term is skipped.
const FTS_DF_CAP_MIN_ROWS: f64 = 1000.0;

/// How long the cached corpus size is reused. Exactness is irrelevant here; the
/// count itself costs tens of milliseconds on a large index.
const FTS_ROWS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

fn cap_terms_by_df<'a>(total_rows: f64, dfs: &HashMap<String, i64>, terms: &[&'a str]) -> Vec<&'a str> {
    if terms.len() < 2 || total_rows < FTS_DF_CAP_MIN_ROWS {
        return terms.to_vec();
    }
    let kept: Vec<&str> = terms
        .iter()
        .copied()
        // An unknown term (missing vocab table, or not indexed at all) means "keep it".
        .filter(|t| dfs.get(*t).map(|df| (*df as f64) / total_rows <= FTS_MAX_DF_FRACTION).unwrap_or(true))
        .collect();
    // Never leave the query with nothing to match on.
    if kept.is_empty() { terms.to_vec() } else { kept }
}

/// Writes chunk rows into `chunk_fts` inside the caller's transaction. Shared
/// by `upsert_fts_batch` and `commit_chunk_batch` so both index text identically.
pub(crate) fn write_fts_rows(tx: &rusqlite::Transaction, chunks: &[IndexedChunk]) -> anyhow::Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT OR REPLACE INTO chunk_fts (rowid, qualified_name, file_path, chunk_text)
         VALUES (?1, ?2, ?3, ?4)",
    )?;
    let extra_tokens = appended_tokens;
    for c in chunks {
        let bare_name = c.qualified_name.rsplit_once("::")
            .map(|(_, n)| n)
            .unwrap_or(&c.qualified_name);
        // Strip a trailing `#N` overload-disambiguator (Plan 21 item 9) — it's a
        // uniqueness suffix, not part of the identifier a query would ever type.
        let bare_name = match bare_name.rsplit_once('#') {
            Some((base, suffix)) if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) => base,
            _ => bare_name,
        };
        let name_tokens = extra_tokens(bare_name);
        let indexed_name = format!("{} {name_tokens}", c.qualified_name);
        // Plan 21 item 12: decompose+stem the body text the same way the bare
        // identifier is, so a query stemmed/split by `tokenize_query` can match
        // body content instead of only exact surface form.
        let body_tokens = extra_tokens(&c.chunk_text);
        let indexed_chunk_text = format!("{} {body_tokens}", c.chunk_text);
        stmt.execute(rusqlite::params![
            c.symbol_id as i64,
            indexed_name,
            c.file_path,
            indexed_chunk_text,
        ])?;
    }
    Ok(())
}

pub(crate) struct ResolveCand {
    pub(crate) id: SymbolId,
    pub(crate) in_degree: i64,
    /// Enclosing type, or file stem, lowercased.
    pub(crate) type_key: Option<String>,
}

/// A candidate's "locality key": the enclosing type name from a 3-part
/// qualified_name (`path::Type::name[#N]`), or the file's stem when the symbol
/// has no enclosing type.
pub(crate) fn resolve_type_key(qualified_name: &str, file_path: &str) -> Option<String> {
    let parts: Vec<&str> = qualified_name.split("::").collect();
    if parts.len() >= 3 {
        Some(parts[parts.len() - 2].to_ascii_lowercase())
    } else {
        Path::new(file_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
    }
}

/// Imports are keyed by importing file, last path segment only, lowercased, with
/// quotes/relative-path noise trimmed: a raw import can be
/// `"org.keycloak.models.RealmModel"`, `"./realm-model"`, or `'RealmModel'`; the
/// last `.`/`/`-delimited segment is the closest cross-language proxy for "the
/// type name this file said it depends on" without a per-language grammar.
pub(crate) fn import_last_segment(raw: &str) -> String {
    let trimmed = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    trimmed
        .rsplit(['.', '/', '\\'])
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase()
}

/// The pure decision half of `resolve_edges`: shared by the full pass and the
/// scoped per-file pass so both make identical choices for the same edge.
pub(crate) fn decide_resolutions(
    pending: &[(i64, SymbolId, String)],
    by_name_lang: &std::collections::HashMap<(String, String), Vec<ResolveCand>>,
    sym_lang: &std::collections::HashMap<SymbolId, String>,
    sym_file: &std::collections::HashMap<SymbolId, FileId>,
    imports_by_file: &std::collections::HashMap<FileId, Vec<String>>,
) -> Vec<(i64, SymbolId)> {
    let mut resolutions: Vec<(i64, SymbolId)> = Vec::new();
    for (edge_id, from_id, to_name) in pending {
        let Some(from_lang) = sym_lang.get(from_id) else { continue };
        let Some(candidates) = by_name_lang.get(&(to_name.clone(), from_lang.clone())) else { continue };

        let resolved_id = if candidates.len() == 1 {
            Some(candidates[0].id)
        } else {
            let from_file = sym_file.get(from_id).copied();
            let imports = from_file
                .and_then(|f| imports_by_file.get(&f))
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let matched: Vec<&ResolveCand> = candidates
                .iter()
                .filter(|c| {
                    c.type_key.as_deref()
                        .is_some_and(|k| imports.iter().any(|imp| imp == k))
                })
                .collect();
            match matched.len() {
                1 => Some(matched[0].id),
                // Multiple import-matched candidates sharing a type_key (e.g.
                // two files both named `realm_model.py` on different paths) —
                // break the remaining tie with in-degree rather than leaving a
                // real import signal on the table entirely.
                n if n > 1 => matched.iter().max_by_key(|c| (c.in_degree, std::cmp::Reverse(c.id))).map(|c| c.id),
                _ => None, // no import signal at all — leave unresolved
            }
        };
        if let Some(id) = resolved_id {
            resolutions.push((*edge_id, id));
        }
    }
    resolutions
}

pub(crate) fn apply_resolutions(conn: &Connection, resolutions: &[(i64, SymbolId)]) -> anyhow::Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut upd = tx.prepare_cached("UPDATE edges SET to_id = ?1 WHERE id = ?2")?;
        for (edge_id, to_id) in resolutions {
            upd.execute(rusqlite::params![*to_id as i64, *edge_id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// One SCIP definition to fold onto an already-indexed symbol (Plan 24 item 1.2).
pub struct ScipEnrichment {
    pub file_id:     FileId,
    pub line:        u32,
    pub name:        String,
    pub scip_symbol: String,
    pub kind:        Option<graphrelay_core::types::SymbolKind>,
    pub docstring:   Option<String>,
}

/// One SCIP reference/import occurrence to turn into a semantic edge.
pub struct ScipRef {
    pub file_id:     FileId,
    pub line:        u32,
    pub scip_symbol: String,
    pub to_name:     String,
    pub kind:        graphrelay_core::types::EdgeKind,
}

#[derive(Debug, Default)]
pub struct ScipMergeCounts {
    pub enriched:   usize,
    pub docstrings: usize,
    pub edges:      usize,
}

impl GraphDb {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            pending_seq: std::sync::atomic::AtomicU64::new(initial_pending_seq()),
            fts_rows_cache: Mutex::new(None),
            fts_df_cache: Mutex::new((None, std::collections::HashMap::new())),
        })
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            pending_seq: std::sync::atomic::AtomicU64::new(initial_pending_seq()),
            fts_rows_cache: Mutex::new(None),
            fts_df_cache: Mutex::new((None, std::collections::HashMap::new())),
        })
    }

    /// Number of chunks in `chunk_fts`, cached for `FTS_ROWS_CACHE_TTL`.
    fn fts_total_rows(&self, conn: &Connection) -> f64 {
        let mut cache = self.fts_rows_cache.lock().unwrap();
        if let Some((at, n)) = *cache {
            if at.elapsed() < FTS_ROWS_CACHE_TTL {
                return n;
            }
        }
        let n = conn
            .query_row("SELECT COUNT(*) FROM chunk_fts_docsize", [], |r| r.get::<_, i64>(0))
            .map(|n| n as f64)
            .unwrap_or(0.0);
        *cache = Some((std::time::Instant::now(), n));
        n
    }

    /// Document frequency of each of `terms`, from the `chunk_fts_vocab` view.
    /// Cached per term; misses are fetched with one statement per 200 terms
    /// (every lookup walks the FTS segments, so batching and caching both matter).
    /// Terms the vocab does not list are absent from the result.
    fn fts_term_dfs(&self, conn: &Connection, terms: &[&str]) -> HashMap<String, i64> {
        let mut cache = self.fts_df_cache.lock().unwrap();
        if cache.0.is_none_or(|at| at.elapsed() >= FTS_ROWS_CACHE_TTL) {
            *cache = (Some(std::time::Instant::now()), HashMap::new());
        }
        let misses: Vec<&str> = terms.iter().copied().filter(|t| !cache.1.contains_key(*t)).collect();
        for chunk in misses.chunks(200) {
            let sql = format!(
                "SELECT term, doc FROM chunk_fts_vocab WHERE term IN ({})",
                vec!["?"; chunk.len()].join(",")
            );
            let Ok(mut stmt) = conn.prepare(&sql) else { return HashMap::new() };
            let Ok(mut rows) = stmt.query(rusqlite::params_from_iter(chunk.iter())) else { return HashMap::new() };
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            while let Ok(Some(r)) = rows.next() {
                if let (Ok(term), Ok(doc)) = (r.get::<_, String>(0), r.get::<_, i64>(1)) {
                    seen.insert(term.clone());
                    cache.1.insert(term, doc);
                }
            }
            // A term with no posting list at all has df 0; cache that too.
            for t in chunk {
                if !seen.contains(*t) {
                    cache.1.insert((*t).to_string(), 0);
                }
            }
        }
        terms.iter().filter_map(|t| cache.1.get(*t).map(|d| ((*t).to_string(), *d))).collect()
    }

    pub(crate) fn next_pending_token(&self) -> u64 {
        self.pending_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    // ── File operations ──────────────────────────────────────────────────────

    pub fn upsert_file(&self, file: &FileRecord) -> anyhow::Result<FileId> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET
               checksum=excluded.checksum,
               language=excluded.language,
               indexed_at=excluded.indexed_at,
               byte_size=excluded.byte_size",
            rusqlite::params![
                file.id as i64,
                file.path.to_string_lossy().as_ref(),
                file.checksum as i64,
                file.language.as_str(),
                file.indexed_at,
                file.byte_size as i64,
            ],
        )?;
        Ok(file.id)
    }

    pub fn file_by_path(&self, path: &Path) -> anyhow::Result<Option<FileRecord>> {
        let conn = self.conn.lock().unwrap();
        let path_str = path.to_string_lossy();
        let result = conn.query_row(
            "SELECT id, path, checksum, language, indexed_at, byte_size FROM files WHERE path = ?1",
            rusqlite::params![path_str.as_ref()],
            row_to_file,
        );
        match result {
            Ok(f)                            => Ok(Some(f)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e)                           => Err(e.into()),
        }
    }

    /// Suffix-match fallback for `file_by_path`: matches indexed files whose
    /// stored (always-absolute) path ends with the given relative path —
    /// e.g. `erpnext/stock/doctype/item/item.py` matches
    /// `/…/repos/erpnext/erpnext/stock/doctype/item/item.py`. `get_file_context`
    /// and the REST `/api/file/*` handler are both agent/user-facing with a
    /// `path` argument; an exact-match-only lookup silently fails whenever a
    /// caller passes the repo-relative path Read/Grep/Glob accept everywhere
    /// else (confirmed: 20 "file not indexed" errors across 14/30 benchmark
    /// questions, all repo-relative paths, before this fallback existed).
    /// Returns every match so the caller can decide how to handle >1 (ambiguous
    /// suffix, e.g. two same-named files in different packages).
    pub fn files_by_suffix(&self, path: &Path) -> anyhow::Result<Vec<FileRecord>> {
        let given = path.to_string_lossy();
        let given = given.trim_matches('/');
        if given.is_empty() {
            return Ok(Vec::new());
        }
        // Escape SQL LIKE metacharacters in the (user-controlled) path — most
        // indexed filenames contain `_` (e.g. delivery_note.py), which is a
        // single-char LIKE wildcard and would otherwise silently over-match.
        let escaped: String = given
            .chars()
            .flat_map(|c| match c {
                '%' | '_' | '\\' => vec!['\\', c],
                other => vec![other],
            })
            .collect();
        let pattern = format!("%/{escaped}");
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, checksum, language, indexed_at, byte_size FROM files \
             WHERE path LIKE ?1 ESCAPE '\\'",
        )?;
        let rows = stmt.query_map(rusqlite::params![pattern], row_to_file)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// `chunk_fts` is a plain FTS5 virtual table, not a real foreign-key-
    /// constrained one, so `symbols.file_id ON DELETE CASCADE` (which does
    /// clean up `symbols`/`edges`) never touched it — a pre-existing gap
    /// found while removing vector search (Plan 20), now more consequential
    /// since FTS5 is the sole retrieval channel rather than a fallback next
    /// to LanceDB's own `delete_by_file`. Delete `chunk_fts` rows by rowid
    /// (== symbol id, see `upsert_fts_batch`) *before* the cascade removes
    /// the symbols the subquery reads, in the same transaction.
    pub fn delete_file(&self, file_id: FileId) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM chunk_fts WHERE rowid IN (SELECT id FROM symbols WHERE file_id = ?1)",
            rusqlite::params![file_id as i64],
        )?;
        tx.execute("DELETE FROM files WHERE id = ?1", rusqlite::params![file_id as i64])?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_file_checksum(&self, path: &Path, checksum: u64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET checksum = ?1 WHERE path = ?2",
            rusqlite::params![checksum as i64, path.to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    /// Bumps `indexed_at` for files the watcher determined are unchanged
    /// (checksum match) without touching `symbols`/`edges`. Both the live
    /// single-file path and the parallel full-index path skip re-parsing an
    /// unchanged file entirely and, before this method existed, skipped
    /// updating `indexed_at` too — leaving a file whose disk mtime moves
    /// without its content changing (`git checkout`/`rebase`/`stash`, a fresh
    /// clone, a CI checkout) permanently `mtime > indexed_at`, which is
    /// exactly the Plan 22 item 2 `is_possibly_stale` staleness condition.
    /// Confirmed live: a `touch` with no content change left a file flagged
    /// `possibly_stale` even after the watcher's next batch ran and found
    /// nothing to re-index. One transaction per call so the parallel
    /// full-index path can batch this once per chunk instead of once per
    /// unchanged file (the common case on a re-index of a large, mostly-
    /// unchanged repo).
    pub fn touch_files_indexed_at(&self, paths: &[std::path::PathBuf], indexed_at: &str) -> anyhow::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached("UPDATE files SET indexed_at = ?1 WHERE path = ?2")?;
            for path in paths {
                stmt.execute(rusqlite::params![indexed_at, path.to_string_lossy().as_ref()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn file_checksum(&self, path: &Path) -> anyhow::Result<Option<u64>> {
        let conn = self.conn.lock().unwrap();
        let path_str = path.to_string_lossy();
        let result = conn.query_row(
            "SELECT checksum FROM files WHERE path = ?1",
            rusqlite::params![path_str.as_ref()],
            |r| r.get::<_, i64>(0),
        );
        match result {
            Ok(v)                            => Ok(Some(v as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e)                           => Err(e.into()),
        }
    }

    /// All `(path, checksum)` pairs in one query — used by the parallel
    /// full-index path to filter out unchanged files before the (expensive)
    /// parse phase, instead of one `file_checksum` lookup per file serialized
    /// through the connection mutex.
    pub fn all_file_checksums(&self) -> anyhow::Result<std::collections::HashMap<std::path::PathBuf, u64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path, checksum FROM files")?;
        let rows = stmt.query_map([], |r| {
            let path: String = r.get(0)?;
            let checksum: i64 = r.get(1)?;
            Ok((std::path::PathBuf::from(path), checksum as u64))
        })?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (path, checksum) = row?;
            out.insert(path, checksum);
        }
        Ok(out)
    }

    /// Write a batch of file extractions (file record + symbols + edges) to
    /// SQLite inside a single transaction, instead of one commit per file per
    /// table. Used by the parallel full-index path, where parsing has already
    /// happened off this connection entirely — this call is the one place
    /// batched files actually touch the database. Preserves the existing
    /// per-file "upsert file, then delete+reinsert its symbols, then its
    /// edges" ordering, just amortized across the whole batch's commit.
    /// Returns one pending token per batch entry, in order (see
    /// `replace_file_graph`); callers hand each to the chunk-commit step.
    pub fn upsert_batch(&self, batch: &[(FileRecord, Vec<Symbol>, Vec<Edge>)]) -> anyhow::Result<Vec<u64>> {
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let mut tokens = Vec::with_capacity(batch.len());
        for (file, symbols, edges) in batch {
            let token = self.next_pending_token();
            crate::indexing::replace_file_in_tx(&tx, file, symbols, edges, token)?;
            tokens.push(token);
        }
        tx.commit()?;
        Ok(tokens)
    }

    /// Called from `flush_chunk_batch` on each batch's built `IndexedChunk`s
    /// — the sole write path for the lexical index since Plan 20 removed the
    /// vector store this used to run alongside. `INSERT OR REPLACE` by
    /// explicit `rowid` (set to the symbol's `SymbolId`, same bit-cast
    /// convention as `symbols.id`) makes re-indexing a symbol an O(1) rowid
    /// replace rather than an O(n) FTS5 column-value scan.
    ///
    /// The indexed `qualified_name` text is the raw qualified name plus the
    /// camelCase/snake_case-decomposed tokens (`tokenize_name`) of its **bare
    /// identifier only** — the part after the last `::` (`Extractor` builds
    /// `qualified_name` as `{rel_path}::{name}`, or `{rel_path}::{EnclosingType}::{name}`
    /// when nested in a class/struct/enum/trait/interface — Plan 21 item 9 —
    /// optionally with a `#N` overload-disambiguator suffix, stripped below
    /// before tokenizing since it isn't part of the searchable identifier).
    /// SQLite FTS5's
    /// stock `unicode61` tokenizer treats `resetEditor` as one opaque token,
    /// so without this a query paraphrasing "resets the editor" could never
    /// match it regardless of corpus-IDF quality. Bare-identifier-only,
    /// specifically, matters: decomposing the *whole* qualified name
    /// (including the file path) floods the index with generic tokens
    /// whenever many files share a naming prefix (this codebase's
    /// `LexicalEditor.ts`/`LexicalSelection.ts`/`LexicalUtils.ts`), burying
    /// genuinely relevant matches under false positives from unrelated files.
    /// See `checklist.md` Plan 16.
    pub fn upsert_fts_batch(&self, chunks: &[IndexedChunk]) -> anyhow::Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        write_fts_rows(&tx, chunks)?;
        tx.commit()?;
        Ok(())
    }

    /// Lexical candidate generation via SQLite FTS5's built-in `bm25()` ranking
    /// function — real corpus-level IDF, unlike the per-candidate approximation
    /// in `graphrelay-core`'s reranker (which only ever sees documents already
    /// selected by this search, so it can't compute a corpus-wide document
    /// frequency). This is the sole candidate-generation channel that seeds
    /// `RetrievalPipeline`'s graph expansion.
    ///
    /// `qualified_name`/`file_path` are weighted higher than raw `chunk_text`:
    /// an identifier or path match is a stronger relevance signal than an
    /// incidental word match inside a function body.
    ///
    /// A specificity-rescoring "next lever" was attempted and reverted here —
    /// see `checklist.md` Plan 16. It fixed its one motivating case
    /// (`readEditorState`, reported stuck at raw rank #157) but regressed the
    /// full 40-task benchmark by displacing other, previously-correct
    /// queries — a single fixed case is not sufficient evidence a reranking
    /// change is net-positive.
    ///
    /// A 2026-08-17 follow-up (`bench-reports/lexical-readeditorstate-tiebreak-investigation-2026-08-17.md`)
    /// found that #157 figure (and this function's own earlier draft claims
    /// about `readEditorState`) had been computed against the *full*
    /// question text — `llm-task-eval.py` actually truncates every query to
    /// its first 120 characters before retrieval, and for this question that
    /// cuts the identifier itself in half. Against the truncated query the
    /// harness really sends, `readEditorState`'s raw rank at these weights is
    /// #479 (pre-fix tokenizer) / #372 (post-fix) — nowhere near `top_k`,
    /// unaffected by anything in this function. That follow-up also tried
    /// discounting `chunk_text`'s weight
    /// (favoring `qualified_name`) and measured a +1-task net positive on
    /// one full-benchmark run — but the weighting tested was chosen from a
    /// pre-screen that itself used the same invalidated full-text queries,
    /// and was never re-confirmed at neighboring weightings or repeat runs
    /// (the check Round 5 used to distinguish real signal from knob-luck).
    /// Deliberately **not shipped** pending that validation — see the report
    /// for the full pre-screen data if picking this back up.
    ///
    /// Returns `(SymbolId, score)` pairs, score normalized to `[0, 1]` (1.0 =
    /// best match in this result set). Terms are pre-tokenized by the caller
    /// (`tokenize_query` in `graphrelay-core`) — every term is alphanumeric/`_`
    /// only, so no FTS5 query-syntax escaping is needed before quoting.
    ///
    /// Plan 21 item 14: `synonym_terms` (a subset of `terms`, from
    /// `tokenize_query_split`) is scored in a separate FTS pass and combined
    /// at `SYNONYM_TERM_WEIGHT` — previously every expanded synonym was
    /// `OR`-ed into one query alongside the user's own words with no
    /// distinction, so a query sharing zero real vocabulary with a symbol but
    /// happening to land in the same synonym group scored identically to an
    /// exact literal match. A row matched by an original term keeps its full
    /// score (plus a damped bonus if a synonym also matched); a row matched
    /// *only* via a synonym is still found — recall doesn't drop — but enters
    /// the pool at reduced strength.
    pub fn lexical_search(&self, terms: &[String], synonym_terms: &[String], top_k: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let original_terms: Vec<&String> = terms.iter()
            .filter(|t| !synonym_terms.contains(t))
            .collect();

        // Fetch generously past `top_k` in each pass before combining — a row
        // that ranks outside `top_k` on original terms alone but has a
        // synonym match too could still belong in the final top_k once
        // combined, and per-pass truncation would hide it.
        let fetch_limit = (top_k * 3).max(90);
        let original_hits = if original_terms.is_empty() {
            Vec::new()
        } else {
            self.fts_match(&original_terms.iter().map(|s| s.as_str()).collect::<Vec<_>>(), fetch_limit)?
        };
        let synonym_hits = if synonym_terms.is_empty() {
            Vec::new()
        } else {
            self.fts_match(&synonym_terms.iter().map(|s| s.as_str()).collect::<Vec<_>>(), fetch_limit)?
        };

        let mut combined: std::collections::HashMap<SymbolId, f32> = std::collections::HashMap::new();
        for (id, score) in original_hits {
            *combined.entry(id).or_insert(0.0) += score;
        }
        for (id, score) in synonym_hits {
            *combined.entry(id).or_insert(0.0) += SYNONYM_TERM_WEIGHT * score;
        }

        let mut hits: Vec<(SymbolId, f32)> = combined.into_iter().collect();
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        hits.truncate(top_k);

        let max_score = hits.iter().map(|(_, s)| *s).fold(0f32, f32::max);
        if max_score > 0.0 {
            for (_, s) in hits.iter_mut() {
                *s /= max_score;
            }
        }
        Ok(hits)
    }

    /// Single FTS5 `MATCH` pass — one `OR`-joined query over `terms`, raw
    /// (unnormalized) `bm25()`-derived scores. Factored out of
    /// `lexical_search` so it can be run once per term-weight class (Plan 21
    /// item 14) without duplicating the query-building/row-mapping logic.
    fn fts_match(&self, terms: &[&str], limit: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        let conn = self.conn.lock().unwrap();
        let total_rows = self.fts_total_rows(&conn);
        let dfs = if terms.len() >= 2 && total_rows >= FTS_DF_CAP_MIN_ROWS {
            self.fts_term_dfs(&conn, terms)
        } else {
            HashMap::new()
        };
        let kept = cap_terms_by_df(total_rows, &dfs, terms);
        let match_query = kept
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR ");

        let mut stmt = conn.prepare(
            "SELECT rowid, bm25(chunk_fts, 3.0, 2.0, 1.0) AS rank
             FROM chunk_fts
             WHERE chunk_fts MATCH ?1
             ORDER BY rank ASC, rowid ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![match_query, limit as i64], |r| {
            let rowid: i64 = r.get(0)?;
            let rank: f64  = r.get(1)?;
            // SQLite's bm25() convention: more negative = better match. Clamp
            // to 0.0 rather than trusting that convention unconditionally —
            // if a future SQLite/weighting change ever returned a positive
            // rank for some row, an unclamped `-rank` would go negative and
            // flow straight into the additive combination in `lexical_search`
            // unnormalized.
            Ok((rowid as u64, (-rank as f32).max(0.0)))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Plan 22 item 3 — file-path pinning, step 1: resolve a query-detected
    /// path-like substring to indexed file path(s). Exact match first (the
    /// common case when a caller passes back an absolute path from a prior
    /// result), falling back to `files_by_suffix`'s suffix match — same
    /// precedence as `get_file_context`'s `resolve_file`. Returns every
    /// suffix match (not just one) so the caller (`RetrievalPipeline::
    /// resolve_path_pins`) can apply its own ambiguity policy.
    pub fn resolve_pinned_files(&self, path_text: &str) -> anyhow::Result<Vec<String>> {
        if let Some(f) = self.file_by_path(Path::new(path_text))? {
            return Ok(vec![f.path.to_string_lossy().into_owned()]);
        }
        let matches = self.files_by_suffix(Path::new(path_text))?;
        Ok(matches.into_iter().map(|f| f.path.to_string_lossy().into_owned()).collect())
    }

    /// Plan 22 item 3 — file-path pinning, step 2: for one pinned file,
    /// return up to `max_symbols` `(SymbolId, score)` pairs — FTS `bm25()`
    /// scored against `remaining_terms` restricted to this file's own
    /// symbols (`rowid IN (SELECT id FROM symbols WHERE file_id = ?)`, same
    /// weighting as `fts_match`), falling back to a stable span-order slice
    /// (score 0.0, first `max_symbols` by `start_line`) when no remaining
    /// term matches anything in the file — e.g. the query is *just* the
    /// path, or every remaining term is too short/generic to have been
    /// indexed (`tokenize_query`'s 3-char floor).
    pub fn symbols_for_path_pin(&self, file_path: &str, remaining_terms: &[String], max_symbols: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        let Some(file) = self.file_by_path(Path::new(file_path))? else {
            return Ok(vec![]);
        };

        if !remaining_terms.is_empty() {
            let match_query = remaining_terms
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(" OR ");
            let hits: Vec<(SymbolId, f32)> = {
                let conn = self.conn.lock().unwrap();
                let mut stmt = conn.prepare(
                    "SELECT rowid, bm25(chunk_fts, 3.0, 2.0, 1.0) AS rank
                     FROM chunk_fts
                     WHERE chunk_fts MATCH ?1
                       AND rowid IN (SELECT id FROM symbols WHERE file_id = ?2)
                     ORDER BY rank ASC, rowid ASC
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![match_query, file.id as i64, max_symbols as i64],
                    |r| {
                        let rowid: i64 = r.get(0)?;
                        let rank: f64  = r.get(1)?;
                        Ok((rowid as u64, (-rank as f32).max(0.0)))
                    },
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            if !hits.is_empty() {
                let max_score = hits.iter().map(|(_, s)| *s).fold(0f32, f32::max);
                return Ok(if max_score > 0.0 {
                    hits.into_iter().map(|(id, s)| (id, s / max_score)).collect()
                } else {
                    hits
                });
            }
        }

        // Fallback: no remaining term matched anything in the file (or there
        // were no remaining terms at all) — admit a stable, deterministic
        // slice by span order instead of admitting nothing.
        let mut symbols = self.symbols_in_file(file.id)?;
        symbols.sort_by_key(|s| s.span.start_line);
        symbols.truncate(max_symbols);
        Ok(symbols.into_iter().map(|s| (s.id, 0.0)).collect())
    }

    pub fn total_file_count(&self) -> anyhow::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        Ok(count as u64)
    }

    pub fn total_symbol_count(&self) -> anyhow::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
        Ok(count as u64)
    }

    // ── Symbol operations ────────────────────────────────────────────────────

    pub fn upsert_symbols(&self, symbols: &[Symbol]) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO symbols (id, file_id, name, qualified_name, kind, start_byte,
                                     end_byte, start_line, end_line, docstring, language)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(qualified_name) DO UPDATE SET
                   file_id=excluded.file_id,
                   name=excluded.name,
                   kind=excluded.kind,
                   start_byte=excluded.start_byte,
                   end_byte=excluded.end_byte,
                   start_line=excluded.start_line,
                   end_line=excluded.end_line,
                   docstring=excluded.docstring,
                   language=excluded.language",
            )?;
            for s in symbols {
                stmt.execute(rusqlite::params![
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
        tx.commit()?;
        Ok(())
    }

    pub fn symbol_by_id(&self, id: SymbolId) -> anyhow::Result<Option<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.id = ?1",
            rusqlite::params![id as i64],
            row_to_symbol,
        );
        match result {
            Ok(s)                            => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e)                           => Err(e.into()),
        }
    }

    pub fn symbol_by_qualified(&self, qname: &str) -> anyhow::Result<Option<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.qualified_name = ?1",
            rusqlite::params![qname],
            row_to_symbol,
        );
        match result {
            Ok(s)                            => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e)                           => Err(e.into()),
        }
    }

    pub fn symbols_in_file(&self, file_id: FileId) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.file_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![file_id as i64], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Exact (non-fuzzy) name match, backed by `idx_symbols_name`. Unlike
    /// `search_symbols_by_name` (substring `LIKE`), this returns only symbols whose
    /// `name` equals `name` exactly — used where callers need to rank *all* same-named
    /// candidates rather than an arbitrary prefix of a fuzzy match.
    pub fn symbols_by_exact_name(&self, name: &str, limit: usize) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.name = ?1
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![name, limit as i64],
            row_to_symbol,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// All symbols for a given language in one query — used by the
    /// dynamic-dispatch synthesis pass, which only needs C/C++ symbols
    /// regardless of how many other languages the corpus contains.
    pub fn symbols_by_language(&self, lang: &str) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.language = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![lang], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Resolved `(from_id, to_id)` pairs of one edge kind whose source symbol is in `lang`.
    pub fn resolved_edge_pairs(&self, kind: &str, lang: &str) -> anyhow::Result<Vec<(SymbolId, SymbolId)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT e.from_id, e.to_id FROM edges e JOIN symbols s ON s.id = e.from_id
             WHERE e.kind = ?1 AND e.to_id IS NOT NULL AND s.language = ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![kind, lang], |r| Ok((r.get::<_, i64>(0)? as SymbolId, r.get::<_, i64>(1)? as SymbolId)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every symbol in the project. Used only by the one-time lexical-FTS
    /// backfill (`graphrelay_watcher::backfill_fts_index`) for projects indexed
    /// before `003_lexical_fts` existed — not on any query hot path.
    pub fn all_symbols(&self) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id",
        )?;
        let rows = stmt.query_map([], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Symbols across multiple files in one round trip, instead of one query per file.
    pub fn symbols_in_files(&self, file_ids: &[FileId]) -> anyhow::Result<Vec<Symbol>> {
        if file_ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        for chunk in file_ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                        s.start_line, s.end_line, s.docstring, s.language, f.path
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.file_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let ids: Vec<i64> = chunk.iter().map(|&id| id as i64).collect();
            let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), row_to_symbol)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn search_symbols_by_name(
        &self,
        pattern: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let like_pattern = format!("%{pattern}%");
        let mut stmt = conn.prepare(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.name LIKE ?1
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![like_pattern, limit as i64],
            row_to_symbol,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Called by the live-watcher's per-file re-index path
    /// (`IndexWorker::index_path_graph_only`) before re-inserting a file's
    /// freshly-extracted symbols. Must clean up `chunk_fts` rows for the
    /// symbols being deleted here (Plan 21 item 5) — `chunk_fts` is a plain
    /// FTS5 virtual table with no real foreign-key cascade from `symbols`
    /// (same gap `delete_file` already works around, see its doc comment),
    /// so a symbol removed or renamed by an edit-and-save (not a whole-file
    /// delete) previously left its old `chunk_fts` row behind forever: it
    /// kept matching queries, occupied a `top_k` slot, and `enrich()` would
    /// then silently drop it since `symbols` no longer had that id. Deleting
    /// `chunk_fts` rows by subquery *before* the `symbols` delete (same
    /// ordering as `delete_file`) means the subquery still sees the rows
    /// about to disappear.
    pub fn delete_symbols_for_file(&self, file_id: FileId) -> anyhow::Result<()> {
        self.delete_stale_symbols_for_file(file_id, &[])
    }

    /// Plan 21 item 10's second half: a plain delete-then-reinsert on every
    /// save (the old `delete_symbols_for_file` behavior, still used via the
    /// empty-`keep_ids` wrapper above for full-file-delete callers) deletes
    /// EVERY symbol in the file even when most of it is untouched by the
    /// edit. Since `SymbolId` is content-addressed (unchanged code produces
    /// the identical id), that DELETE fires SQLite's `ON DELETE SET NULL`
    /// cascade on every OTHER file's edges pointing at those unchanged
    /// symbols too — even though the very next statement re-inserts the same
    /// ids right back. `resolve_edges()`'s next run then sees those
    /// newly-NULLed edges as fresh ambiguity and can re-resolve them to a
    /// *different* candidate than before, silently re-routing a caller's
    /// already-correct edge just because an unrelated file elsewhere in the
    /// repo happened to save first. Passing the freshly-extracted symbol ids
    /// as `keep_ids` here means only symbols that are genuinely gone (deleted
    /// or renamed by the edit) get deleted — everything else is left alone
    /// for `upsert_symbols`'s `ON CONFLICT(qualified_name) DO UPDATE` to
    /// update in place, without ever cascading.
    pub fn delete_stale_symbols_for_file(&self, file_id: FileId, keep_ids: &[SymbolId]) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if keep_ids.is_empty() {
            tx.execute(
                "DELETE FROM chunk_fts WHERE rowid IN (SELECT id FROM symbols WHERE file_id = ?1)",
                rusqlite::params![file_id as i64],
            )?;
            tx.execute(
                "DELETE FROM symbols WHERE file_id = ?1",
                rusqlite::params![file_id as i64],
            )?;
        } else {
            let placeholders = keep_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let mut fts_params: Vec<rusqlite::types::Value> = Vec::with_capacity(keep_ids.len() + 1);
            fts_params.push((file_id as i64).into());
            fts_params.extend(keep_ids.iter().map(|id| (*id as i64).into()));

            let sql_fts = format!(
                "DELETE FROM chunk_fts WHERE rowid IN (SELECT id FROM symbols WHERE file_id = ?1 AND id NOT IN ({placeholders}))"
            );
            let sql_sym = format!(
                "DELETE FROM symbols WHERE file_id = ?1 AND id NOT IN ({placeholders})"
            );
            tx.execute(&sql_fts, rusqlite::params_from_iter(fts_params.iter()))?;
            tx.execute(&sql_sym, rusqlite::params_from_iter(fts_params.iter()))?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── Edge operations ──────────────────────────────────────────────────────

    pub fn upsert_edges(&self, edges: &[Edge]) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO edges (from_id, to_id, to_name, kind, weight, source)
                 VALUES (?1, ?2, ?3, ?4, 1.0, ?5)",
            )?;
            for e in edges {
                stmt.execute(rusqlite::params![
                    e.from_id as i64,
                    e.to_id.map(|id| id as i64),
                    e.to_name,
                    e.kind.as_str(),
                    e.source.as_str(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Return up to 200 callers of the given symbol (Plan 12 A2: bounded for cross-file safety).
    /// Ordered by symbol id: without an `ORDER BY` the 200-row cut, the first-5 names in chunk
    /// text and PageRank's summation order all depended on edge row order, so two databases
    /// with identical content could rank differently (Plan 24 final validation, erp-15).
    pub fn callers_of(&self, symbol_id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN edges e ON e.from_id = s.id
             WHERE e.to_id = ?1 AND e.kind = 'Calls'
             ORDER BY s.id
             LIMIT 200",
        )?;
        let rows = stmt.query_map(rusqlite::params![symbol_id as i64], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn symbols_by_ids(&self, ids: &[SymbolId]) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                        s.start_line, s.end_line, s.docstring, s.language, f.path
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE s.id IN ({placeholders})
                 ORDER BY s.id"
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<i64> = chunk.iter().map(|&id| id as i64).collect();
            let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), row_to_symbol)?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// Same set and order as `callees_of`, ids only.
    pub fn callee_ids_of(&self, symbol_id: SymbolId) -> anyhow::Result<Vec<SymbolId>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT s.id FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN edges e ON e.to_id = s.id
             WHERE e.from_id = ?1 AND e.kind = 'Calls'
             ORDER BY s.id
             LIMIT 200",
        )?;
        let rows = stmt.query_map(rusqlite::params![symbol_id as i64], |r| r.get::<_, i64>(0))?;
        rows.map(|r| r.map(|id| id as SymbolId).map_err(Into::into)).collect()
    }

    /// First `limit` caller and callee names (by caller/callee id, the same
    /// order and cap `callers_of`/`callees_of` + `take(limit)` produce) for every
    /// symbol in one file, in two queries instead of two per symbol. Used by
    /// chunk building, which only ever reads the first few names.
    // The CROSS JOINs and `+e.kind` pin the plan to symbols(file_id) -> edges(to_id|from_id).
    // Left alone, SQLite drives from idx_edges_kind and scans every Calls edge once per
    // file (about 91k on ERPNext), which made a full index take minutes instead of seconds.
    pub fn call_neighbor_names_for_file(
        &self,
        file_id: FileId,
        limit: usize,
    ) -> anyhow::Result<HashMap<SymbolId, CallNeighborNames>> {
        let conn = self.conn.lock().unwrap();
        let mut out: HashMap<SymbolId, CallNeighborNames> = HashMap::new();
        for (target_col, other_col, is_callers) in [("to_id", "from_id", true), ("from_id", "to_id", false)] {
            let sql = neighbor_names_sql(target_col, other_col);
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(rusqlite::params![file_id as i64, limit as i64], |r| {
                Ok((r.get::<_, i64>(0)? as SymbolId, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (target, name) = row?;
                let entry = out.entry(target).or_default();
                if is_callers { entry.0.push(name) } else { entry.1.push(name) }
            }
        }
        Ok(out)
    }

    /// Return up to 200 callees of the given symbol (Plan 12 A2: bounded for cross-file safety).
    pub fn callees_of(&self, symbol_id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN edges e ON e.to_id = s.id
             WHERE e.from_id = ?1 AND e.kind = 'Calls'
             ORDER BY s.id
             LIMIT 200",
        )?;
        let rows = stmt.query_map(rusqlite::params![symbol_id as i64], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Graph-expansion neighbors for retrieval candidate generation and the
    /// `/api/graph` viewer — deliberately wider than `callers_of`/`callees_of`
    /// (Calls only): also walks `Inherits`/`References`/`Implements` edges,
    /// since "related code" for a query or a graph-view focus reasonably
    /// includes a symbol's base class/subclasses, its type-annotation
    /// relationships, and its trait/interface implementations, not just its
    /// call graph. `callers_of`/`callees_of` themselves stay Calls-only —
    /// `trace_impact`, `personalized_pagerank`, `explain_symbol`, and the
    /// REST symbol-detail endpoint all rely on that narrower, precise
    /// "who calls / is called by this" semantics and are deliberately left
    /// untouched by this widening. `ORDER BY 1` (the output `id` column —
    /// a `UNION`'s `ORDER BY` must reference the result set's own column
    /// position/name, not a source-table-qualified one like `s.id`) before
    /// `LIMIT 200` makes the cap deterministic for a >200-neighbor hub,
    /// rather than leaving which 200 survive up to SQLite's own dedup order.
    pub fn neighbors(&self, symbol_id: SymbolId, hops: u8) -> anyhow::Result<Vec<Symbol>> {
        if hops == 0 {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN edges e ON e.from_id = s.id
             WHERE e.to_id = ?1 AND e.kind IN ('Calls', 'Inherits', 'References', 'Implements')
             UNION
             SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN edges e ON e.to_id = s.id
             WHERE e.from_id = ?1 AND e.kind IN ('Calls', 'Inherits', 'References', 'Implements')
             ORDER BY 1
             LIMIT 200",
        )?;
        let rows = stmt.query_map(rusqlite::params![symbol_id as i64], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Symbols with an edge of one of `kinds` into any of `to_ids`, in one round
    /// trip, each tagged with the real `edges.source` and kind of the edge that
    /// connects it (not inferred from a file-path comparison). Ordered so BFS
    /// discovery (and with it each node's reported edge) is deterministic.
    /// Chunked to stay under SQLite's default bound parameter limit
    /// (`SQLITE_LIMIT_VARIABLE_NUMBER`, 999) for large frontiers.
    fn dependents_of_batch(
        &self,
        to_ids: &[SymbolId],
        kinds: &[EdgeKind],
    ) -> anyhow::Result<Vec<(SymbolId, Symbol, EdgeSource, EdgeKind)>> {
        if to_ids.is_empty() || kinds.is_empty() {
            return Ok(vec![]);
        }
        let kind_list = kinds.iter().map(|k| format!("'{}'", k.as_str())).collect::<Vec<_>>().join(",");
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        for chunk in to_ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                        s.start_line, s.end_line, s.docstring, s.language, f.path,
                        e.source, e.to_id, e.kind
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 JOIN edges e ON e.from_id = s.id
                 WHERE e.to_id IN ({placeholders}) AND e.kind IN ({kind_list})
                 ORDER BY s.id, e.to_id, e.kind"
            );
            let mut stmt = conn.prepare(&sql)?;
            let ids: Vec<i64> = chunk.iter().map(|&id| id as i64).collect();
            let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                let sym = row_to_symbol(row)?;
                let source_str: String = row.get(12)?;
                let to_id: i64 = row.get(13)?;
                let kind_str: String = row.get(14)?;
                Ok((
                    to_id as SymbolId,
                    sym,
                    source_str.parse::<EdgeSource>().unwrap_or(EdgeSource::Syntax),
                    EdgeKind::from_str(&kind_str).unwrap_or(EdgeKind::Calls),
                ))
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// BFS out from `symbol_id` up to `max_depth` hops of callers, visiting each symbol
    /// at most once. This bounds both cost and lock hold time to O(unique symbols
    /// reached) instead of the combinatorial fan-out a plain `UNION ALL` recursive CTE
    /// produces for high-fan-in nodes, and lets each discovered node carry the *real*
    /// edge source of the edge that first reached it (shortest-path tie-break) rather
    /// than a heuristic derived from file-path comparison.
    pub fn impact_graph(&self, symbol_id: SymbolId, max_depth: u8) -> anyhow::Result<ImpactGraph> {
        self.impact_graph_with(symbol_id, max_depth, &[EdgeKind::Calls], IMPACT_NODE_BUDGET)
    }

    /// `impact_graph` over any set of dependency edge kinds, stopping once
    /// `node_budget` symbols have been reached (`budget_hit` then marks the
    /// result as a lower bound): a hub at a large depth otherwise built its
    /// whole reverse closure, cloning every symbol, before the caller truncated.
    pub fn impact_graph_with(
        &self,
        symbol_id: SymbolId,
        max_depth: u8,
        kinds: &[EdgeKind],
        node_budget: usize,
    ) -> anyhow::Result<ImpactGraph> {
        use std::collections::HashMap;

        let mut visited: HashMap<SymbolId, (Symbol, EdgeSource, EdgeKind, u8, SymbolId)> = HashMap::new();
        let mut frontier = vec![symbol_id];
        let mut depth: u8 = 0;
        let mut budget_hit = false;

        'bfs: while depth < max_depth && !frontier.is_empty() {
            let batch = self.dependents_of_batch(&frontier, kinds)?;
            let mut next_frontier = Vec::new();
            for (to_id, dependent, source, kind) in batch {
                if dependent.id == symbol_id || visited.contains_key(&dependent.id) {
                    continue;
                }
                if visited.len() >= node_budget {
                    budget_hit = true;
                    break 'bfs;
                }
                next_frontier.push(dependent.id);
                visited.insert(dependent.id, (dependent, source, kind, depth + 1, to_id));
            }
            frontier = next_frontier;
            depth += 1;
        }

        // `via` needs the parent's qualified name; every parent is the root or
        // another visited node.
        let root_name = match self.symbol_by_id(symbol_id)? {
            Some(s) => s.qualified_name,
            None => String::new(),
        };
        let names: HashMap<SymbolId, String> = visited
            .iter()
            .map(|(&id, v)| (id, v.0.qualified_name.clone()))
            .collect();
        let affected = visited
            .into_values()
            .map(|(s, edge_source, kind, hop_depth, parent)| {
                let via = if hop_depth <= 1 || parent == symbol_id {
                    None
                } else {
                    names.get(&parent).cloned().or_else(|| Some(root_name.clone()))
                };
                let path = s.file_path.clone();
                let reason = match kind {
                    EdgeKind::Inherits   => "directly or transitively inherits from the changed symbol",
                    EdgeKind::Implements => "directly or transitively implements the changed symbol",
                    EdgeKind::References => "directly or transitively references the changed symbol",
                    _                    => "directly or transitively calls the changed symbol",
                };
                ImpactNode {
                    symbol: s,
                    path,
                    reason: reason.to_string(),
                    edge_source,
                    hop_depth,
                    via,
                }
            })
            .collect();
        Ok(ImpactGraph {
            root: symbol_id,
            affected,
            depth: max_depth,
            budget_hit,
        })
    }

    pub fn delete_edges_for_file(&self, file_id: FileId) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM edges WHERE from_id IN (SELECT id FROM symbols WHERE file_id = ?1)",
            rusqlite::params![file_id as i64],
        )?;
        Ok(())
    }

    /// Resolve cross-file edges: for every edge where `to_id IS NULL` and `to_name` is set,
    /// find the matching symbol by name and fill in `to_id`. When multiple symbols share the
    /// same name, prefer the one with the highest in-degree (most-called = most likely target).
    /// Returns the number of edges resolved.
    ///
    /// The in-degree lookup is materialized into a temp table ONCE before the
    /// `UPDATE`, not recomputed per row: an earlier version put the
    /// `GROUP BY to_id, COUNT(*)` aggregation directly inside the outer
    /// `UPDATE`'s correlated subquery, where it doesn't reference any
    /// outer-row column but is still nested inside a subquery that DOES
    /// (`s.name = edges.to_name`) — SQLite has no way to hoist an
    /// independent aggregation out of a correlated subquery, so it re-ran
    /// that full-table `GROUP BY` once per unresolved edge, an
    /// O(unresolved_edges × total_edges) full-table-scan-per-row pattern.
    /// On a real `apple/swift` full-index run (see
    /// `bench-reports/large-repo-indexing-2026-08-04.md`)
    /// this was quadratic enough to run for 2+ hours before the kernel
    /// killed the process for 81.6GB of accumulated compressed memory (large
    /// repeated `GROUP BY` temp result sets) on a corpus only ~20x larger
    /// than a subset that finished the whole pipeline, resolve_edges
    /// included, in 23 seconds — linear scaling predicts single-digit
    /// minutes and low-single-digit-GB, not hours and tens of GB. Computing
    /// `_edge_in_degree` once turns this into a single `GROUP BY` scan plus
    /// one indexed join per unresolved edge. One semantic difference from
    /// the old query, found in code review: the old correlated subquery
    /// recomputed the aggregation live inside the same `UPDATE`, so a row
    /// resolved earlier in the same call could (depending on SQLite's
    /// unspecified internal row order) shift the in-degree seen by a later
    /// ambiguous row in that same call. Freezing `_edge_in_degree` before the
    /// `UPDATE` runs removes that order-dependency — multiple ambiguous
    /// edges resolved in one `resolve_edges()` call can no longer influence
    /// each other's tie-break, which is arguably a latent-bug fix, not a
    /// new behavior change, but is worth flagging as not byte-for-byte
    /// identical to the old query's behavior.
    ///
    /// `GraphDb` holds one long-lived connection behind a `Mutex` for the
    /// whole process lifetime (see `open`/`open_in_memory`), and this method
    /// is called repeatedly on it — so the temp table must never be left
    /// behind on any exit path (a partial `CREATE TABLE`-then-failed-`CREATE
    /// INDEX`, or any other error) or every subsequent call fails forever
    /// with "table already exists". `DROP TABLE IF EXISTS` up front makes
    /// re-creation idempotent regardless of what a prior call left behind;
    /// the trailing drop's own failure is logged, not propagated in place of
    /// the `UPDATE`'s real result — an error from the drop must not mask an
    /// error (or success) from the `UPDATE` itself.
    /// Plan 21 item 10 rewrite: everything below runs in Rust over bulk-loaded
    /// in-memory maps, not per-row correlated SQL subqueries — this exact
    /// method has a documented history of a real quadratic-blowup incident
    /// (see the doc comment above on the `_edge_in_degree` temp table, added
    /// after a `GROUP BY`-inside-a-correlated-subquery pattern ran for 2+
    /// hours and 81.6GB on a large corpus). A per-candidate-count correlated
    /// subquery to detect "is this name ambiguous" would reintroduce exactly
    /// that shape of bug, so ambiguity detection and import-based
    /// disambiguation are both done as one-pass Rust lookups instead.
    ///
    /// Three-tier resolution per unresolved `to_name`, same-language only
    /// (Plan 21 item 3's constraint, preserved):
    /// 1. Exactly one same-language candidate named `to_name` → resolve to it
    ///    directly, no tie-break needed.
    /// 2. More than one candidate (ambiguous) → consult the calling symbol's
    ///    own file's `Imports` edges (extracted since Plan 12 A3, never
    ///    consulted before this — Plan 02 §6 step 2's original design).
    ///    If exactly one candidate's enclosing-type name (or file stem, for
    ///    symbols with no enclosing type) matches an imported path's last
    ///    segment, prefer it over any in-degree guess.
    /// 3. Still ambiguous with no import signal (zero or multiple import
    ///    matches) → **leave the edge unresolved** rather than routing it to
    ///    a global highest-in-degree winner. Measured motivation on keycloak:
    ///    the old blind in-degree tie-break let one `getId()` overload absorb
    ///    4,270 calls that belonged to 1,004 other methods, corrupting
    ///    PageRank/`trace_impact`/`summarize_module`'s in-degree ranking. A
    ///    missing edge is a known, visible gap; a misattributed one silently
    ///    lies. This is a deliberate precision-over-recall trade — expect
    ///    fewer resolved edges than before on corpora with many ambiguous
    ///    names, not a bug.
    pub fn resolve_edges(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();

        // Bulk-load every symbol's (name, language) -> candidates, and every
        // symbol's own file_id/language for looking up the FROM side of an
        // edge. One pass over `symbols`, O(n).
        let mut by_name_lang: std::collections::HashMap<(String, String), Vec<ResolveCand>> =
            std::collections::HashMap::new();
        let mut sym_lang: std::collections::HashMap<SymbolId, String> = std::collections::HashMap::new();
        let mut sym_file: std::collections::HashMap<SymbolId, FileId> = std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.name, s.qualified_name, s.language, s.file_id, f.path
                 FROM symbols s JOIN files f ON f.id = s.file_id",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let id = id as SymbolId;
                let name: String = r.get(1)?;
                let qualified_name: String = r.get(2)?;
                let language: String = r.get(3)?;
                let file_id: i64 = r.get(4)?;
                let file_id = file_id as FileId;
                let file_path: String = r.get(5)?;

                sym_lang.insert(id, language.clone());
                sym_file.insert(id, file_id);
                by_name_lang
                    .entry((name, language))
                    .or_default()
                    .push(ResolveCand { id, in_degree: 0, type_key: resolve_type_key(&qualified_name, &file_path) });
            }
        }

        // In-degree, still used as the tie-break signal WITHIN an
        // import-matched set if more than one candidate happens to share a
        // type_key (rare) — not as a blind fallback for genuine ambiguity.
        {
            let mut stmt = conn.prepare(
                "SELECT to_id, COUNT(*) FROM edges WHERE to_id IS NOT NULL GROUP BY to_id",
            )?;
            let mut rows = stmt.query([])?;
            let mut degrees: std::collections::HashMap<SymbolId, i64> = std::collections::HashMap::new();
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                let cnt: i64 = r.get(1)?;
                degrees.insert(id as SymbolId, cnt);
            }
            for cands in by_name_lang.values_mut() {
                for c in cands.iter_mut() {
                    c.in_degree = degrees.get(&c.id).copied().unwrap_or(0);
                }
            }
        }

        let mut imports_by_file: std::collections::HashMap<FileId, Vec<String>> =
            std::collections::HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT s.file_id, e.to_name FROM edges e
                 JOIN symbols s ON s.id = e.from_id
                 WHERE e.kind = 'Imports' AND e.to_name IS NOT NULL",
            )?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let file_id: i64 = r.get(0)?;
                let raw: String = r.get(1)?;
                let last = import_last_segment(&raw);
                if !last.is_empty() {
                    imports_by_file.entry(file_id as FileId).or_default().push(last);
                }
            }
        }

        let mut stmt = conn.prepare("SELECT id, from_id, to_name FROM edges WHERE to_id IS NULL AND to_name IS NOT NULL AND source != 'semantic'")?;
        let mut pending: Vec<(i64, SymbolId, String)> = Vec::new();
        {
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                let edge_id: i64 = r.get(0)?;
                let from_id: i64 = r.get(1)?;
                let to_name: String = r.get(2)?;
                pending.push((edge_id, from_id as SymbolId, to_name));
            }
        }
        drop(stmt);

        let resolutions = decide_resolutions(&pending, &by_name_lang, &sym_lang, &sym_file, &imports_by_file);
        apply_resolutions(&conn, &resolutions)?;
        Ok(resolutions.len())
    }

    // ── Graph analytics ──────────────────────────────────────────────────────

    pub fn top_symbols_by_in_degree(
        &self,
        limit: usize,
        lang_filter: Option<&str>,
    ) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let base = "SELECT s.id, s.file_id, s.name, s.qualified_name, s.kind, s.start_byte, s.end_byte,
                    s.start_line, s.end_line, s.docstring, s.language, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id";
        let order = " ORDER BY (SELECT COUNT(*) FROM edges e WHERE e.to_id = s.id) DESC LIMIT ?";
        if let Some(lang) = lang_filter {
            let sql = format!("{base} WHERE s.language = ? {order}");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![lang, limit as i64], row_to_symbol)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let sql = format!("{base}{order}");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![limit as i64], row_to_symbol)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    pub fn edges_between(&self, symbol_ids: &[SymbolId]) -> anyhow::Result<Vec<Edge>> {
        if symbol_ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.conn.lock().unwrap();
        // Use unnumbered `?` placeholders — rusqlite positional params
        let placeholders = vec!["?"; symbol_ids.len()].join(",");
        let sql = format!(
            "SELECT from_id, to_id, to_name, kind, source FROM edges \
             WHERE from_id IN ({p}) AND to_id IN ({p})",
            p = placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let ids: Vec<i64> = symbol_ids.iter().map(|&id| id as i64).collect();
        let rows = stmt.query_map(
            rusqlite::params_from_iter(ids.iter().chain(ids.iter())),
            row_to_edge,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn language_counts(&self) -> anyhow::Result<Vec<(String, u64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT language, COUNT(*) as cnt FROM files GROUP BY language ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let lang: String = r.get(0)?;
            let count: i64 = r.get(1)?;
            Ok((lang, count as u64))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn in_degree(&self, symbol_id: SymbolId) -> anyhow::Result<u32> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE to_id = ?1",
            rusqlite::params![symbol_id as i64],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    /// In-degree for a batch of symbols in one round trip (one `GROUP BY` query per
    /// chunk instead of one `COUNT(*)` query per symbol). Symbols with zero incoming
    /// edges are simply absent from the map — callers should default to 0.
    pub fn in_degrees_batch(
        &self,
        symbol_ids: &[SymbolId],
    ) -> anyhow::Result<std::collections::HashMap<SymbolId, u32>> {
        if symbol_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let conn = self.conn.lock().unwrap();
        let mut out = std::collections::HashMap::new();
        for chunk in symbol_ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT to_id, COUNT(*) FROM edges WHERE to_id IN ({placeholders}) GROUP BY to_id"
            );
            let mut stmt = conn.prepare(&sql)?;
            let ids: Vec<i64> = chunk.iter().map(|&id| id as i64).collect();
            let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                let id: i64 = r.get(0)?;
                let count: i64 = r.get(1)?;
                Ok((id as SymbolId, count as u32))
            })?;
            for row in rows {
                let (id, count) = row?;
                out.insert(id, count);
            }
        }
        Ok(out)
    }

    pub fn out_degree(&self, symbol_id: SymbolId) -> anyhow::Result<u32> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE from_id = ?1",
            rusqlite::params![symbol_id as i64],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    // ── Plan 12: SCIP semantic layer helpers ─────────────────────────────────

    /// C1: Delete all edges with `source = 'semantic'` (before re-ingesting SCIP).
    /// find_implementations: return all symbols that (transitively) implement
    /// or inherit from `symbol_id`.
    ///
    /// Plan 21 item 4: previously included `References` edges, which are
    /// dramatically more common than real `Inherits`/`Implements` edges (any
    /// parameter or return-type mention of the interface counts) and drowned
    /// out genuine implementors entirely — measured on keycloak, `RealmModel`
    /// had 2,483 `References` edges against only 4 real `Inherits` edges, so
    /// every one of the 15 returned "implementations" was parameter-mention
    /// noise, zero real hits. Restricted to `Inherits`/`Implements` only.
    ///
    /// Also walks the inheritance chain transitively via a recursive CTE —
    /// previously only direct implementors surfaced, so `B implements
    /// Interface` was found but `A extends B` was not, even though `A` is
    /// also a real (indirect) implementor. `UNION` (not `UNION ALL`)
    /// deduplicates as it accumulates, which also makes this safe against a
    /// cyclic `Inherits` graph (a malformed/synthetic one, or a language
    /// with unusual mixin semantics): a symbol already in the accumulated
    /// set is never re-queued, so a cycle can add no new rows and recursion
    /// terminates on its own.
    pub fn implementors_of(&self, symbol_id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "WITH RECURSIVE impl_chain(id) AS (
                 SELECT from_id FROM edges
                 WHERE to_id = ?1 AND kind IN ('Inherits', 'Implements')
                 UNION
                 SELECT e.from_id FROM edges e
                 JOIN impl_chain ic ON e.to_id = ic.id
                 WHERE e.kind IN ('Inherits', 'Implements')
             )
             SELECT DISTINCT s.id, s.file_id, s.name, s.qualified_name, s.kind,
                    s.start_byte, s.end_byte, s.start_line, s.end_line,
                    s.docstring, s.language, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN impl_chain ic ON ic.id = s.id
             ORDER BY s.qualified_name
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![symbol_id as i64, IMPLEMENTORS_LIMIT as i64], row_to_symbol)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_semantic_edges(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM edges WHERE source = 'semantic'",
            [],
        )?;
        Ok(n)
    }

    /// Delete all edges with `source = 'synthesized'` — must be called before
    /// re-inserting a fresh batch from the dynamic-dispatch synthesis pass
    /// (`graphrelay_parser::synthesis`), mirroring `delete_semantic_edges()`'s
    /// role ahead of a fresh SCIP merge. `edges` has no unique constraint
    /// beyond its own `id`, so `upsert_edges`'s `INSERT OR REPLACE` never
    /// actually replaces a matching row — without deleting stale synthesized
    /// edges first, every full-index run would append duplicates of the same
    /// edges rather than refreshing them, permanently inflating in-degree
    /// (and therefore rerank score) for repeatedly-detected dispatch targets.
    pub fn delete_synthesized_edges(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM edges WHERE source = 'synthesized'",
            [],
        )?;
        Ok(n)
    }

    /// Plan 24 items 1.1-1.4/1.7 — apply a whole SCIP ingest atomically.
    ///
    /// Everything runs in ONE transaction while this connection's mutex is held:
    /// (1) drop all semantic edges, (2) enrich symbols, (3) resolve and insert
    /// reference edges. A failure rolls back to the previous semantic layer
    /// instead of leaving it deleted, and no concurrent writer can delete a
    /// symbol between resolution and insert (so no FK violation can abort it).
    ///
    /// Enrichment matches a definition to the *smallest symbol with the same
    /// bare name whose span contains the occurrence's line*. The old
    /// `(file_id, start_line)`-only match let a parameter defined on a method's
    /// own line overwrite the method's `scip_symbol` (Plan 24 M2).
    ///
    /// Edges are only kept when both ends resolve to real symbols
    /// (precision over recall): an unresolved semantic edge has no consumer
    /// except `resolve_edges`, which name-matched them and misattributed calls
    /// (Plan 24 finding #8). Semantic edges are deduped on (from, to, kind).
    pub fn apply_scip_merge(
        &self,
        enrichments: &[ScipEnrichment],
        refs: &[ScipRef],
    ) -> anyhow::Result<ScipMergeCounts> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        let mut counts = ScipMergeCounts::default();

        tx.execute("DELETE FROM edges WHERE source = 'semantic'", [])?;

        {
            let mut enrich = tx.prepare_cached(
                "UPDATE symbols
                 SET kind        = COALESCE(?1, kind),
                     scip_symbol = ?2,
                     docstring   = CASE WHEN ?3 IS NOT NULL
                                         AND (docstring IS NULL OR length(docstring) < length(?3))
                                        THEN ?3 ELSE docstring END
                 WHERE id = (SELECT id FROM symbols
                             WHERE file_id = ?4 AND name = ?5
                               AND start_line <= ?6 AND end_line >= ?6
                             ORDER BY (end_line - start_line) ASC, id ASC LIMIT 1)",
            )?;
            for e in enrichments {
                let n = enrich.execute(rusqlite::params![
                    e.kind.map(|k| k.as_str()),
                    e.scip_symbol,
                    e.docstring,
                    e.file_id as i64,
                    e.name,
                    e.line,
                ])?;
                if n > 0 {
                    counts.enriched += 1;
                    if e.docstring.is_some() {
                        counts.docstrings += 1;
                    }
                }
            }
        }

        {
            let mut from_at = tx.prepare_cached(
                "SELECT id FROM symbols
                 WHERE file_id = ?1 AND start_line <= ?2 AND end_line >= ?2
                 ORDER BY (end_line - start_line) ASC, id ASC LIMIT 1",
            )?;
            let mut by_scip = tx.prepare_cached(
                "SELECT id FROM symbols WHERE scip_symbol = ?1 ORDER BY id ASC LIMIT 1",
            )?;
            let mut insert = tx.prepare_cached(
                "INSERT OR IGNORE INTO edges (from_id, to_id, to_name, kind, weight, source)
                 VALUES (?1, ?2, ?3, ?4, 1.0, 'semantic')",
            )?;
            let mut to_cache: std::collections::HashMap<&str, Option<i64>> = std::collections::HashMap::new();
            let mut seen: std::collections::HashSet<(i64, i64, &'static str)> = std::collections::HashSet::new();
            for r in refs {
                let to_id = match to_cache.get(r.scip_symbol.as_str()) {
                    Some(v) => *v,
                    None => {
                        let v: Option<i64> = match by_scip.query_row(rusqlite::params![r.scip_symbol], |row| row.get(0)) {
                            Ok(id) => Some(id),
                            Err(rusqlite::Error::QueryReturnedNoRows) => None,
                            Err(e) => return Err(e.into()),
                        };
                        to_cache.insert(r.scip_symbol.as_str(), v);
                        v
                    }
                };
                let Some(to_id) = to_id else { continue };
                let from_id: i64 = match from_at.query_row(rusqlite::params![r.file_id as i64, r.line], |row| row.get(0)) {
                    Ok(id) => id,
                    Err(rusqlite::Error::QueryReturnedNoRows) => continue,
                    Err(e) => return Err(e.into()),
                };
                let kind = r.kind.as_str();
                if !seen.insert((from_id, to_id, kind)) {
                    continue;
                }
                counts.edges += insert.execute(rusqlite::params![from_id, to_id, r.to_name, kind])?;
            }
        }

        tx.commit()?;
        Ok(counts)
    }

    /// E3: Return the scip_symbol string for a given symbol (None if not SCIP-enriched).
    pub fn get_meta_for_symbol(&self, symbol_id: SymbolId) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result: rusqlite::Result<Option<String>> = conn.query_row(
            "SELECT scip_symbol FROM symbols WHERE id = ?1",
            rusqlite::params![symbol_id as i64],
            |r| r.get(0),
        );
        match result {
            Ok(v)                                     => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e)                                    => Err(e.into()),
        }
    }

    // ── Index metadata ───────────────────────────────────────────────────────

    /// Return all files whose path starts with `prefix` (absolute) or contains `/<prefix>/`.
    pub fn files_in_prefix(&self, prefix: &str) -> anyhow::Result<Vec<FileRecord>> {
        let prefix = prefix.trim_end_matches('/');
        // An empty prefix (`""`, `"/"`) would match the whole index.
        if prefix.is_empty() {
            return Ok(vec![]);
        }
        // `%`/`_` are escaped so `path: "%"` is not "every file" and `_` in
        // `sales_order` is not a wildcard; `LIKE` is case-insensitive, so the
        // segment match is re-checked case-sensitively below.
        let escaped: String = prefix
            .chars()
            .flat_map(|c| match c {
                '%' | '_' | '\\' => vec!['\\', c],
                other => vec![other],
            })
            .collect();
        let abs_pat = format!("{escaped}/%");
        let rel_pat = format!("%/{escaped}/%");
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, checksum, language, indexed_at, byte_size FROM files \
             WHERE path LIKE ?1 ESCAPE '\\' OR path LIKE ?2 ESCAPE '\\'",
        )?;
        let rows = stmt.query_map(rusqlite::params![abs_pat, rel_pat], row_to_file)?;
        let abs_prefix = format!("{prefix}/");
        let segment = format!("/{prefix}/");
        let mut out = Vec::new();
        for row in rows {
            let f = row?;
            let p = f.path.to_string_lossy();
            if p.starts_with(&abs_prefix) || p.contains(&segment) {
                out.push(f);
            }
        }
        Ok(out)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO index_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT value FROM index_meta WHERE key = ?1",
            rusqlite::params![key],
            |r| r.get(0),
        );
        match result {
            Ok(v)                            => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e)                           => Err(e.into()),
        }
    }
}

// ── Row mapping helpers ──────────────────────────────────────────────────────

fn row_to_symbol(r: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    let id: i64                 = r.get(0)?;
    let file_id: i64            = r.get(1)?;
    let name: String            = r.get(2)?;
    let qualified_name: String  = r.get(3)?;
    let kind_str: String        = r.get(4)?;
    let start_byte: u32         = r.get(5)?;
    let end_byte: u32           = r.get(6)?;
    let start_line: u32         = r.get(7)?;
    let end_line: u32           = r.get(8)?;
    let docstring: Option<String> = r.get(9)?;
    let lang_str: String        = r.get(10)?;
    let file_path: String       = r.get(11)?;

    Ok(Symbol {
        id:             id as u64,
        file_id:        file_id as u64,
        name,
        qualified_name,
        kind:           SymbolKind::from_str(&kind_str)
            .unwrap_or(SymbolKind::Function),
        span: Span { start_byte, end_byte, start_line, end_line },
        docstring,
        language:       Language::from_str(&lang_str).unwrap_or(Language::Rust),
        file_path:      std::path::PathBuf::from(file_path),
    })
}

fn row_to_edge(r: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    let from_id: i64         = r.get(0)?;
    let to_id: Option<i64>   = r.get(1)?;
    let to_name: Option<String> = r.get(2)?;
    let kind_str: String     = r.get(3)?;
    let source_str: String   = r.get(4).unwrap_or_else(|_| "syntax".to_string());
    Ok(Edge {
        from_id: from_id as u64,
        to_id:   to_id.map(|id| id as u64),
        to_name,
        kind:    graphrelay_core::types::EdgeKind::from_str(&kind_str)
                     .unwrap_or(graphrelay_core::types::EdgeKind::Calls),
        source:  source_str.parse().unwrap_or(EdgeSource::Syntax),
    })
}

fn row_to_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileRecord> {
    let id: i64          = r.get(0)?;
    let path: String     = r.get(1)?;
    let checksum: i64    = r.get(2)?;
    let lang_str: String = r.get(3)?;
    let indexed_at: String = r.get(4)?;
    let byte_size: i64   = r.get(5)?;

    Ok(FileRecord {
        id:         id as u64,
        path:       std::path::PathBuf::from(path),
        checksum:   checksum as u64,
        language:   Language::from_str(&lang_str).unwrap_or(Language::Rust),
        indexed_at,
        byte_size:  byte_size as u64,
    })
}

// Implement GraphStore trait from graphrelay-core
impl graphrelay_core::retrieval::pipeline::GraphStore for GraphDb {
    fn symbol_by_id(&self, id: SymbolId) -> anyhow::Result<Option<Symbol>> {
        self.symbol_by_id(id)
    }
    fn callers_of(&self, id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        self.callers_of(id)
    }
    fn callees_of(&self, id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        self.callees_of(id)
    }
    fn neighbors(&self, id: SymbolId, hops: u8) -> anyhow::Result<Vec<Symbol>> {
        self.neighbors(id, hops)
    }
    fn in_degree(&self, id: SymbolId) -> anyhow::Result<u32> {
        self.in_degree(id)
    }
    fn symbols_by_ids(&self, ids: &[SymbolId]) -> anyhow::Result<Vec<Symbol>> {
        self.symbols_by_ids(ids)
    }
    fn callee_ids_of(&self, id: SymbolId) -> anyhow::Result<Vec<SymbolId>> {
        self.callee_ids_of(id)
    }
    fn lexical_search(&self, terms: &[String], synonym_terms: &[String], top_k: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        self.lexical_search(terms, synonym_terms, top_k)
    }
    fn resolve_pinned_files(&self, path_text: &str) -> anyhow::Result<Vec<String>> {
        self.resolve_pinned_files(path_text)
    }
    fn symbols_for_path_pin(&self, file_path: &str, remaining_terms: &[String], max_symbols: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        self.symbols_for_path_pin(file_path, remaining_terms, max_symbols)
    }
}

fn neighbor_names_sql(target_col: &str, other_col: &str) -> String {
    format!(
        "SELECT target, name FROM (
             SELECT e.{target_col} AS target, s.name AS name,
                    ROW_NUMBER() OVER (PARTITION BY e.{target_col} ORDER BY s.id) AS rn
             FROM symbols t
             CROSS JOIN edges e
             CROSS JOIN symbols s
             WHERE t.file_id = ?1 AND e.{target_col} = t.id
               AND +e.kind = 'Calls' AND s.id = e.{other_col}
         ) WHERE rn <= ?2
         ORDER BY target, rn"
    )
}

#[cfg(test)]
mod neighbor_names_plan_tests {
    use super::*;

    #[test]
    fn neighbor_names_query_starts_from_the_files_symbols() {
        let db = GraphDb::open_in_memory().unwrap();
        let conn = db.conn.lock().unwrap();
        for (target, other) in [("to_id", "from_id"), ("from_id", "to_id")] {
            let sql = format!("EXPLAIN QUERY PLAN {}", neighbor_names_sql(target, other));
            let mut stmt = conn.prepare(&sql).unwrap();
            let plan: Vec<String> = stmt
                .query_map(rusqlite::params![1i64, 5i64], |r| r.get::<_, String>(3))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            let plan = plan.join("\n");
            assert!(plan.contains("idx_symbols_file_id"), "{plan}");
            assert!(!plan.contains("idx_edges_kind"), "scans every Calls edge per file:\n{plan}");
        }
    }
}
