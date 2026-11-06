use crate::language::{LanguageRegistry, LanguageSupport};
use streaming_iterator::StreamingIterator;
use graphrelay_core::types::{
    Edge, EdgeKind, EdgeSource, FileExtraction, FileId, FileRecord, Language, Span, Symbol,
    SymbolId, SymbolKind,
};
use dashmap::DashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Generate a content-addressed SymbolId from a qualified name.
pub fn generate_symbol_id(qualified_name: &str) -> SymbolId {
    xxhash_rust::xxh64::xxh64(qualified_name.as_bytes(), 0)
}

/// Generate a FileId from a file path.
pub fn generate_file_id(path: &Path) -> FileId {
    xxhash_rust::xxh64::xxh64(path.to_string_lossy().as_bytes(), 0)
}

/// Diff two full-buffer contents into the single edited byte range
/// tree-sitter needs (`tree_sitter::Tree::edit`) — the standard common-
/// prefix/common-suffix trim used whenever only before/after full text is
/// available (not a live keystroke stream with its own precise edit
/// position). `None` means the two buffers are byte-identical.
fn compute_input_edit(old: &[u8], new: &[u8]) -> Option<tree_sitter::InputEdit> {
    if old == new {
        return None;
    }
    let min_len = old.len().min(new.len());
    let mut prefix = 0;
    while prefix < min_len && old[prefix] == new[prefix] {
        prefix += 1;
    }
    // Bounded by what's left after the prefix so the prefix/suffix regions
    // never overlap (e.g. old="aaa", new="aaaa" — prefix already consumes
    // everything old has).
    let max_suffix = min_len - prefix;
    let mut suffix = 0;
    while suffix < max_suffix && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix] {
        suffix += 1;
    }

    let start_byte = prefix;
    let old_end_byte = old.len() - suffix;
    let new_end_byte = new.len() - suffix;

    Some(tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: byte_to_point(old, start_byte),
        old_end_position: byte_to_point(old, old_end_byte),
        new_end_position: byte_to_point(new, new_end_byte),
    })
}

/// Row/column (both 0-indexed, column in bytes per tree-sitter convention)
/// of a byte offset within `bytes`.
fn byte_to_point(bytes: &[u8], offset: usize) -> tree_sitter::Point {
    let offset = offset.min(bytes.len());
    let mut row = 0usize;
    let mut col = 0usize;
    for &b in &bytes[..offset] {
        if b == b'\n' {
            row += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    tree_sitter::Point { row, column: col }
}

pub struct Extractor {
    registry: Arc<LanguageRegistry>,
    parsers:  DashMap<Language, Mutex<tree_sitter::Parser>>,
    // Keyed by path: the source bytes and tree from this file's last
    // `extract_incremental` call, so the next call can diff against them and
    // hand tree-sitter a real edited tree instead of parsing from scratch.
    incremental_cache: DashMap<std::path::PathBuf, (Vec<u8>, tree_sitter::Tree)>,
    /// Project root. When set, qualified names use the path relative to it, so
    /// `SymbolId = xxhash64(qualified_name)` no longer depends on where the repo
    /// happens to be checked out (Plan 24 Phase 4). `None` keeps absolute paths.
    root: Option<std::path::PathBuf>,
}

impl Extractor {
    pub fn new(registry: LanguageRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            parsers:  DashMap::new(),
            incremental_cache: DashMap::new(),
            root: None,
        }
    }

    /// Make qualified names (and therefore symbol ids) relative to `root`.
    pub fn with_root(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Like `extract`, but on a file this `Extractor` has already parsed via
    /// this method, diffs the new source against the cached previous source
    /// and hands tree-sitter a real edited tree — a genuine incremental
    /// re-parse, not a full one — for files whose content mostly repeats
    /// (the common case for a single-file save in the live watcher).
    pub fn extract_incremental(&self, path: &Path, source: &[u8]) -> anyhow::Result<FileExtraction> {
        let lang_support = self
            .registry
            .for_path(path)
            .ok_or_else(|| anyhow::anyhow!("unsupported language: {:?}", path))?;
        let language = lang_support.language_kind();

        // SFC (.vue/.svelte) delegation has no meaningful "cached outer tree"
        // to track edits against — the tree that matters is the delegated
        // script-block parse, rebuilt fresh every call. See `try_sfc_delegate`.
        // Drop any stale cache entry too, in case this path was previously
        // parsed as a plain (non-delegated) language.
        if let Some(extraction) = self.try_sfc_delegate(path, source, language)? {
            self.incremental_cache.remove(path);
            return Ok(extraction);
        }

        let cached = self.incremental_cache.remove(path).map(|(_, v)| v);
        let tree = match cached {
            Some((old_source, mut old_tree)) => match compute_input_edit(&old_source, source) {
                // A real edit: mark the old tree's affected range so
                // tree-sitter only re-parses what actually changed.
                Some(edit) => {
                    old_tree.edit(&edit);
                    self.parse_incremental(lang_support, source, Some(&old_tree))?
                }
                // Byte-identical to last time — nothing to do.
                None => old_tree,
            },
            // First time seeing this path (or it was evicted) — no old tree to diff against.
            None => self.parse(lang_support, source)?,
        };
        self.incremental_cache.insert(path.to_owned(), (source.to_vec(), tree.clone()));

        self.build_extraction(path, source, lang_support, &tree)
    }

    /// Drop a path's cached source/tree — call when a file is deleted so the
    /// cache doesn't grow unbounded with entries for files that no longer
    /// exist, and so a later file recreated at the same path starts fresh
    /// rather than diffing against unrelated old content.
    pub fn invalidate_incremental_cache(&self, path: &Path) {
        self.incremental_cache.remove(path);
    }

    pub fn extract(&self, path: &Path, source: &[u8]) -> anyhow::Result<FileExtraction> {
        let lang_support = self
            .registry
            .for_path(path)
            .ok_or_else(|| anyhow::anyhow!("unsupported language: {:?}", path))?;
        let language = lang_support.language_kind();

        if let Some(extraction) = self.try_sfc_delegate(path, source, language)? {
            return Ok(extraction);
        }

        let tree = self.parse(lang_support, source)?;
        self.build_extraction(path, source, lang_support, &tree)
    }

    /// Parse and extract many files in parallel via rayon. Each call uses a
    /// freshly-constructed `tree_sitter::Parser` rather than the shared
    /// per-language cache `extract`/`extract_incremental` use — tree-sitter
    /// parser construction is cheap (no grammar compilation at this point,
    /// that already happened at build time), so this avoids every concurrent
    /// thread parsing the same language contending on one `Mutex<Parser>`,
    /// which would otherwise serialize the exact work this method exists to
    /// parallelize. Intended for the full-index path; the live single-file
    /// watcher hot path still uses `extract`/`extract_incremental` above.
    pub fn extract_many(
        &self,
        files: &[(std::path::PathBuf, Vec<u8>)],
    ) -> Vec<(std::path::PathBuf, anyhow::Result<FileExtraction>)> {
        use rayon::prelude::*;
        files
            .par_iter()
            .map(|(path, source)| {
                let result = self.extract_with_fresh_parser(path, source);
                (path.clone(), result)
            })
            .collect()
    }

    fn extract_with_fresh_parser(&self, path: &Path, source: &[u8]) -> anyhow::Result<FileExtraction> {
        let lang_support = self
            .registry
            .for_path(path)
            .ok_or_else(|| anyhow::anyhow!("unsupported language: {:?}", path))?;
        let language = lang_support.language_kind();

        if let Some(extraction) = self.try_sfc_delegate(path, source, language)? {
            return Ok(extraction);
        }

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&lang_support.ts_language())
            .map_err(|e| {
                anyhow::anyhow!(
                    "tree-sitter ABI mismatch for {:?}: {e:?}",
                    lang_support.language_kind()
                )
            })?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("parse returned None"))?;

        self.build_extraction(path, source, lang_support, &tree)
    }

    fn build_extraction(
        &self,
        path: &Path,
        source: &[u8],
        lang_support: &dyn LanguageSupport,
        tree: &tree_sitter::Tree,
    ) -> anyhow::Result<FileExtraction> {
        let file_id   = generate_file_id(path);
        let checksum  = xxhash_rust::xxh64::xxh64(source, 0);
        let language  = lang_support.language_kind();
        let indexed_at = chrono::Utc::now().to_rfc3339();

        let file_record = FileRecord {
            id:         file_id,
            path:       path.to_owned(),
            checksum,
            language,
            indexed_at,
            byte_size:  source.len() as u64,
        };

        let symbols = self.extract_symbols(tree, source, lang_support, path, file_id, language)?;
        let edges   = self.extract_edges(tree, source, lang_support, &symbols, path, file_id)?;

        Ok(FileExtraction {
            file_record,
            symbols,
            edges,
            source: source.to_vec(),
        })
    }

    /// Returns `Ok(Some(..))` for `Language::Vue`/`Language::Svelte`, `Ok(None)`
    /// for everything else — callers intercept with this *before* attempting
    /// to parse `source` with `lang_support.ts_language()`, since Vue/Svelte
    /// files are not themselves valid input to any single tree-sitter grammar
    /// (they're a template format wrapping an embedded `<script>` block).
    ///
    /// Delegated symbols/edges are tagged with the *embedded* language
    /// (`TypeScript`/`JavaScript`, whichever the `<script lang="...">`
    /// attribute or file convention indicates), not `Vue`/`Svelte` — this is
    /// deliberate: it makes docstring extraction (which dispatches on
    /// `Language`) work correctly for free, and means Initiative 9's JS/TS
    /// callback-dispatch synthesis picks up Vue/Svelte-embedded code via the
    /// existing `symbols_by_language("typescript"/"javascript")` calls with
    /// no changes needed there. `FileRecord.language` (the file-level tag)
    /// stays `Vue`/`Svelte`, correctly reflecting what kind of *file* this is.
    fn try_sfc_delegate(
        &self,
        path: &Path,
        source: &[u8],
        language: Language,
    ) -> anyhow::Result<Option<FileExtraction>> {
        if !matches!(language, Language::Vue | Language::Svelte) {
            return Ok(None);
        }

        let file_id    = generate_file_id(path);
        let checksum   = xxhash_rust::xxh64::xxh64(source, 0);
        let indexed_at = chrono::Utc::now().to_rfc3339();
        let file_record = FileRecord {
            id: file_id,
            path: path.to_owned(),
            checksum,
            language,
            indexed_at,
            byte_size: source.len() as u64,
        };

        // Not valid UTF-8 (rare for source files) — degrade to "no script
        // block found" rather than risk a byte-offset mismatch between the
        // lossy `String` and the original `&[u8]` this file's spans must be
        // reported against.
        let Ok(source_str) = std::str::from_utf8(source) else {
            return Ok(Some(FileExtraction {
                file_record, symbols: vec![], edges: vec![], source: source.to_vec(),
            }));
        };

        let Some((script_start_byte, script_text, is_typescript)) = find_script_block(source_str) else {
            // Template-only file (no <script> block) — a valid, empty extraction.
            return Ok(Some(FileExtraction {
                file_record, symbols: vec![], edges: vec![], source: source.to_vec(),
            }));
        };

        let delegate_lang = if is_typescript { Language::TypeScript } else { Language::JavaScript };
        let delegate_support = self
            .registry
            .for_language(delegate_lang)
            .ok_or_else(|| anyhow::anyhow!("no {delegate_lang:?} support registered for SFC delegation"))?;

        let script_bytes = script_text.as_bytes();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&delegate_support.ts_language())
            .map_err(|e| anyhow::anyhow!("tree-sitter ABI mismatch for {delegate_lang:?}: {e:?}"))?;
        let tree = parser
            .parse(script_bytes, None)
            .ok_or_else(|| anyhow::anyhow!("SFC script-block parse returned None"))?;

        // Extract from the script slice using slice-relative byte positions
        // throughout — `extract_edges` locates a call site's enclosing symbol
        // by byte-range containment, which only works if both call sites and
        // symbol spans are in the same (slice-relative) coordinate space.
        // Symbol spans are shifted into the original file's coordinates only
        // after both calls complete.
        let mut symbols = self.extract_symbols(
            &tree, script_bytes, delegate_support, path, file_id, delegate_lang,
        )?;
        let edges = self.extract_edges(&tree, script_bytes, delegate_support, &symbols, path, file_id)?;

        let line_offset = source_str.as_bytes()[..script_start_byte]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32;
        for sym in &mut symbols {
            sym.span.start_byte += script_start_byte as u32;
            sym.span.end_byte   += script_start_byte as u32;
            sym.span.start_line += line_offset;
            sym.span.end_line   += line_offset;
        }

        Ok(Some(FileExtraction {
            file_record,
            symbols,
            edges,
            source: source.to_vec(),
        }))
    }

    fn parse(
        &self,
        lang_support: &dyn LanguageSupport,
        source: &[u8],
    ) -> anyhow::Result<tree_sitter::Tree> {
        self.parse_incremental(lang_support, source, None)
    }

    fn parse_incremental(
        &self,
        lang_support: &dyn LanguageSupport,
        source: &[u8],
        old_tree: Option<&tree_sitter::Tree>,
    ) -> anyhow::Result<tree_sitter::Tree> {
        let language = lang_support.language_kind();

        if !self.parsers.contains_key(&language) {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&lang_support.ts_language())
                .map_err(|e| anyhow::anyhow!("tree-sitter ABI mismatch for {:?}: {e:?}", language))?;
            self.parsers.insert(language, Mutex::new(p));
        }

        let entry     = self.parsers.get(&language).unwrap();
        let mut guard = entry.lock().map_err(|_| anyhow::anyhow!("parser mutex poisoned"))?;
        guard
            .parse(source, old_tree)
            .ok_or_else(|| anyhow::anyhow!("parse returned None"))
    }

    /// Path used as the qualified-name prefix: relative to the project root (with
    /// `/` separators) when one is configured and `path` is under it, else as given.
    fn qualified_path(&self, path: &Path) -> String {
        if let Some(root) = &self.root {
            if let Ok(rel) = path.strip_prefix(root) {
                return rel.to_string_lossy().replace('\\', "/");
            }
        }
        path.to_string_lossy().into_owned()
    }

    fn extract_symbols(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        lang: &dyn LanguageSupport,
        path: &Path,
        file_id: FileId,
        language: Language,
    ) -> anyhow::Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        // Parallel to `symbols`: the receiver type of a method declared outside
        // its type's span (Go), used below as the enclosing type.
        let mut receivers: Vec<Option<String>> = Vec::new();
        let rel_path = self.qualified_path(path);

        for query_def in lang.symbol_queries() {
            let query = match tree_sitter::Query::new(&lang.ts_language(), query_def.pattern) {
                Ok(q)  => q,
                Err(e) => {
                    tracing::warn!("Query parse error for {:?}: {e}", query_def.kind);
                    continue;
                }
            };

            let name_idx = match query.capture_index_for_name(query_def.name_capture) {
                Some(i) => i,
                None    => continue,
            };
            let def_idx = query.capture_index_for_name(query_def.def_capture);

            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source);
            while let Some(m) = matches.next() {
                let name_node = m
                    .captures
                    .iter()
                    .find(|c| c.index == name_idx)
                    .map(|c| c.node);

                let def_node = def_idx.and_then(|idx| {
                    m.captures.iter().find(|c| c.index == idx).map(|c| c.node)
                });

                let Some(name_node) = name_node else { continue };

                let name = match name_node.utf8_text(source) {
                    Ok(n)  => n.to_string(),
                    Err(_) => continue,
                };
                if name.is_empty() { continue; }

                // Use def_node span for the definition extent, name_node for line info
                let (start_byte, end_byte) = if let Some(def) = def_node {
                    (def.start_byte(), def.end_byte())
                } else {
                    (name_node.start_byte(), name_node.end_byte())
                };
                let start_line = name_node.start_position().row as u32;
                let end_line   = if let Some(def) = def_node {
                    def.end_position().row as u32
                } else {
                    name_node.end_position().row as u32
                };

                let docstring = extract_docstring(tree, &name_node, source, language);
                receivers.push(def_node.and_then(|d| lang.receiver_type(d, source)));

                // id/qualified_name are computed in a second pass below (Plan
                // 21 item 9) once every symbol in this file is known — the
                // enclosing-type lookup and overload disambiguation both need
                // to see the whole set, not just symbols seen so far.
                symbols.push(Symbol {
                    id: 0,
                    file_id,
                    name,
                    qualified_name: String::new(),
                    kind: query_def.kind,
                    span: Span {
                        start_byte: start_byte as u32,
                        end_byte:   end_byte as u32,
                        start_line,
                        end_line,
                    },
                    docstring,
                    language,
                    file_path: path.to_owned(),
                });
            }
        }

        // Plan 21 item 9: qualified_name is `{rel_path}::{EnclosingType}::{name}`
        // when a symbol is nested inside a class/struct/enum/trait/interface,
        // or plain `{rel_path}::{name}` at top level — the originally-specified
        // Plan 02 §8 design, never actually implemented until now. Before this
        // fix every symbol's id was `xxhash({rel_path}::{name})` alone, so a
        // constructor and its enclosing class collided whenever the
        // constructor's name equaled the class name (Java/C#) and the
        // later-extracted one silently won the old id-keyed dedup — measured
        // on keycloak: ~36% of Java classes were stored as bare `Function`
        // (constructor only, class definition lost).
        const TYPE_KINDS: [SymbolKind; 5] = [
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::Interface,
        ];
        let mut base_names = vec![String::new(); symbols.len()];
        for i in 0..symbols.len() {
            let this_span = symbols[i].span.clone();
            // A receiver is explicit in the source, so it wins over inferring
            // the enclosing type from spans.
            let enclosing_name = receivers[i].clone().or_else(|| symbols
                .iter()
                .enumerate()
                .filter(|(j, s)| {
                    *j != i
                        && TYPE_KINDS.contains(&s.kind)
                        && s.span.start_byte <= this_span.start_byte
                        && this_span.end_byte <= s.span.end_byte
                        && (s.span.start_byte, s.span.end_byte)
                            != (this_span.start_byte, this_span.end_byte)
                })
                .min_by_key(|(_, s)| s.span.end_byte - s.span.start_byte)
                .map(|(_, s)| s.name.clone()));
            base_names[i] = match enclosing_name {
                Some(enc) => format!("{}::{}::{}", rel_path, enc, symbols[i].name),
                None => format!("{}::{}", rel_path, symbols[i].name),
            };
        }

        // Group by base qualified_name. Two entries at the IDENTICAL span are
        // the pre-existing "same node matched by two queries" case (e.g. a Go
        // struct matching both the generic type_spec pattern and the specific
        // struct_type pattern) — keep only the LAST one, same precedence rule
        // the old id-keyed dedup used. Two entries with the SAME base name but
        // DIFFERENT spans are genuine overloads/same-named siblings — keep
        // both, disambiguated with a stable `#N` suffix ordered by source
        // position (not a counter over discovery order, so it doesn't shift
        // when an unrelated symbol elsewhere in the file is added/removed).
        let mut by_base: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
        for (i, name) in base_names.iter().enumerate() {
            by_base.entry(name.as_str()).or_default().push(i);
        }

        let mut final_names = vec![String::new(); symbols.len()];
        for (base, idxs) in &by_base {
            // Drop exact-span duplicates, keeping the last (highest index).
            let mut by_span: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
            for &i in idxs {
                // A decorated Python function matches both its bare
                // `function_definition` and the wrapping `decorated_definition`;
                // the two spans end together. Keep the wrapper so the body's
                // calls and the decorator share one symbol, not `name#1`/`name#2`.
                // Python only: migration 017 re-indexes only Python files, so
                // changing ids for another language would leave stale ones.
                let (start, end) = (symbols[i].span.start_byte, symbols[i].span.end_byte);
                if symbols[i].language == Language::Python
                    && idxs.iter().any(|&j| {
                        symbols[j].span.end_byte == end && symbols[j].span.start_byte < start
                    })
                {
                    continue;
                }
                let key = (symbols[i].span.start_byte, symbols[i].span.end_byte);
                by_span.insert(key, i); // later index overwrites earlier
            }
            let mut kept: Vec<usize> = by_span.into_values().collect();
            kept.sort_by_key(|&i| symbols[i].span.start_byte);

            if kept.len() == 1 {
                final_names[kept[0]] = base.to_string();
            } else {
                for (ordinal, &i) in kept.iter().enumerate() {
                    final_names[i] = format!("{base}#{}", ordinal + 1);
                }
            }
        }

        let mut keep = vec![false; symbols.len()];
        for (i, sym) in symbols.iter_mut().enumerate() {
            if final_names[i].is_empty() {
                continue; // dropped as an exact-span duplicate above
            }
            sym.qualified_name = std::mem::take(&mut final_names[i]);
            sym.id = generate_symbol_id(&sym.qualified_name);
            keep[i] = true;
        }
        let mut iter = keep.into_iter();
        symbols.retain(|_| iter.next().unwrap());

        Ok(symbols)
    }

    fn extract_edges(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        lang: &dyn LanguageSupport,
        symbols: &[Symbol],
        _path: &Path,
        _file_id: FileId,
    ) -> anyhow::Result<Vec<Edge>> {
        let call_pattern = lang.call_query();
        let query = match tree_sitter::Query::new(&lang.ts_language(), call_pattern) {
            Ok(q)  => q,
            Err(_) => return Ok(vec![]),
        };

        let call_name_idx = match query.capture_index_for_name("call.name") {
            Some(i) => i,
            None    => return Ok(vec![]),
        };

        // Build a lookup: name → SymbolId for same-file resolution
        let local_names: std::collections::HashMap<&str, SymbolId> =
            symbols.iter().map(|s| (s.name.as_str(), s.id)).collect();

        let mut edges = Vec::new();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source);
        while let Some(m) = matches.next() {
            let call_node = m
                .captures
                .iter()
                .find(|c| c.index == call_name_idx)
                .map(|c| c.node);

            let Some(call_node) = call_node else { continue };
            let call_name = match call_node.utf8_text(source) {
                Ok(n)  => n.to_string(),
                Err(_) => continue,
            };

            // Find enclosing symbol (the symbol that contains this call site).
            // Must be the NARROWEST containing span, not the first one found:
            // `symbols` is typically ordered class/module-before-method (Java,
            // C#, Dart, Groovy, ObjC, Elixir all register the outer symbol
            // first), so `.find()` here would credit every call inside a
            // method to its enclosing class/module instead of the method
            // itself — measured on keycloak: 140 TS/JS class methods with
            // zero outgoing Calls edges before this fix, because every call
            // inside them was attributed to the class.
            let call_byte = call_node.start_byte() as u32;
            let from_sym = symbols
                .iter()
                .filter(|s| s.span.start_byte <= call_byte && call_byte <= s.span.end_byte)
                .min_by_key(|s| s.span.end_byte - s.span.start_byte);

            let Some(from_sym) = from_sym else { continue };

            // Try to resolve to a local symbol
            let to_id = local_names.get(call_name.as_str()).copied();

            edges.push(Edge {
                from_id: from_sym.id,
                to_id,
                to_name: if to_id.is_none() {
                    Some(call_name)
                } else {
                    None
                },
                kind:   EdgeKind::Calls,
                source: EdgeSource::Syntax,
            });
        }

        // Plan 12 A3: wire import_query() into EdgeKind::Imports edges.
        // Imports get to_id = None and to_name = raw import path string.
        // Cross-file resolution happens later (SCIP semantic layer or store resolve_edges).
        let import_pattern = lang.import_query();
        if let Ok(import_query) = tree_sitter::Query::new(&lang.ts_language(), import_pattern) {
            if let Some(path_idx) = import_query.capture_index_for_name("import.path") {
                let mut icursor = tree_sitter::QueryCursor::new();
                let mut imatches = icursor.matches(&import_query, tree.root_node(), source);
                while let Some(m) = imatches.next() {
                    let path_node = m.captures.iter().find(|c| c.index == path_idx);
                    let Some(path_node) = path_node else { continue };
                    let import_path = match path_node.node.utf8_text(source) {
                        Ok(p)  => p.trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string(),
                        Err(_) => continue,
                    };
                    if import_path.is_empty() { continue; }
                    // Find enclosing symbol (the module/file-level symbol, or first symbol)
                    let import_byte = path_node.node.start_byte() as u32;
                    let from_id = symbols.iter()
                        .find(|s| s.span.start_byte <= import_byte && import_byte <= s.span.end_byte)
                        .or_else(|| symbols.first())
                        .map(|s| s.id);
                    let Some(from_id) = from_id else { continue };
                    edges.push(Edge {
                        from_id,
                        to_id:   None,
                        to_name: Some(import_path),
                        kind:    EdgeKind::Imports,
                        source:  EdgeSource::Syntax,
                    });
                }
            }
        }

        // `inherits_query()` into `EdgeKind::Inherits` edges, and
        // `references_query()` into `EdgeKind::References` edges (type
        // annotations only — a function/method/field's parameter, return,
        // or declared type mentioning another known symbol; deliberately
        // narrower than some competitors' "any known-symbol-name token
        // mentioned anywhere in this body" heuristic, which also fires on
        // unrelated string-literal/variable-name coincidences). Unlike
        // calls/imports (whose `from` is found by byte-range containment),
        // both query shapes capture their own "from" explicitly —
        // `@inherits.from`/`@references.from` (the enclosing symbol's own
        // name) and `@inherits.to`/`@references.to` (the referenced type's
        // name) — so a target declared later in the same file, or whose
        // own span doesn't happen to contain the reference site, still
        // resolves correctly.
        push_captured_edges(lang.inherits_query(), "inherits", EdgeKind::Inherits, lang, tree, source, symbols, &local_names, &mut edges);
        push_captured_edges(lang.references_query(), "references", EdgeKind::References, lang, tree, source, symbols, &local_names, &mut edges);
        push_captured_edges(lang.implements_query(), "implements", EdgeKind::Implements, lang, tree, source, symbols, &local_names, &mut edges);

        // Deduplicate edges
        edges.sort_by(|a, b| {
            a.from_id.cmp(&b.from_id)
                .then(a.to_id.cmp(&b.to_id))
                .then(a.to_name.cmp(&b.to_name))
                .then(a.kind.as_str().cmp(b.kind.as_str()))
        });
        edges.dedup_by(|a, b| {
            a.from_id == b.from_id
                && a.to_id   == b.to_id
                && a.to_name == b.to_name
                && a.kind.as_str() == b.kind.as_str()
        });

        Ok(edges)
    }
}

/// Shared implementation for query shapes that capture both edge ends
/// explicitly (`@<prefix>.from`/`@<prefix>.to`) rather than resolving
/// `from` by byte-range containment — used by `inherits_query()` and
/// `references_query()`. A no-op if `pattern` is empty (a language that
/// doesn't implement this query) or fails to compile.
#[allow(clippy::too_many_arguments)]
fn push_captured_edges(
    pattern: &str,
    prefix: &str,
    kind: EdgeKind,
    lang: &dyn LanguageSupport,
    tree: &tree_sitter::Tree,
    source: &[u8],
    symbols: &[Symbol],
    local_names: &std::collections::HashMap<&str, SymbolId>,
    edges: &mut Vec<Edge>,
) {
    if pattern.is_empty() {
        return;
    }
    let Ok(query) = tree_sitter::Query::new(&lang.ts_language(), pattern) else { return };
    let from_idx = query.capture_index_for_name(&format!("{prefix}.from"));
    let to_idx   = query.capture_index_for_name(&format!("{prefix}.to"));
    let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) else { return };

    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        let from_node = m.captures.iter().find(|c| c.index == from_idx);
        let to_node   = m.captures.iter().find(|c| c.index == to_idx);
        let (Some(from_node), Some(to_node)) = (from_node, to_node) else { continue };
        let Ok(from_name) = from_node.node.utf8_text(source) else { continue };
        let Ok(to_name) = to_node.node.utf8_text(source) else { continue };
        if to_name.is_empty() { continue; }
        let Some(from_sym) = symbols.iter().find(|s| s.name == from_name) else { continue };
        let to_id = local_names.get(to_name).copied();
        edges.push(Edge {
            from_id: from_sym.id,
            to_id,
            to_name: if to_id.is_none() { Some(to_name.to_string()) } else { None },
            kind,
            source: EdgeSource::Syntax,
        });
    }
}

// ── SFC (.vue/.svelte) script-block delegation ───────────────────────────────

fn script_block_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"(?s)<script([^>]*)>(.*?)</script>"#).unwrap()
    })
}

/// Find the first `<script>` block in an SFC source, returning
/// `(byte_offset_of_content, content_text, is_typescript)`. `byte_offset` is
/// the offset of the captured content (not the opening tag) into `source`,
/// so `source[byte_offset..byte_offset + content.len()] == content`.
///
/// Known limitation, documented rather than handled: a Vue file with *both*
/// a `<script>` and a separate `<script setup>` block (a valid, if uncommon,
/// pattern for defining module-scope exports alongside setup-scope code)
/// only has its first matching block delegated — this is a text-pattern
/// heuristic, not an SFC compiler, and matches the scoping of every other
/// heuristic in this codebase (see `synthesis::c_fnptr`'s own header comment
/// for the same posture).
fn find_script_block(source: &str) -> Option<(usize, &str, bool)> {
    let caps = script_block_re().captures(source)?;
    let attrs = caps.get(1)?.as_str();
    let content = caps.get(2)?;
    let is_typescript = attrs.contains("lang=\"ts\"") || attrs.contains("lang='ts'")
        || attrs.contains("lang=\"tsx\"") || attrs.contains("lang='tsx'");
    Some((content.start(), content.as_str(), is_typescript))
}

// ── Docstring extraction ─────────────────────────────────────────────────────

fn extract_docstring(
    _tree: &tree_sitter::Tree,
    name_node: &tree_sitter::Node<'_>,
    source: &[u8],
    language: Language,
) -> Option<String> {
    // Walk back through preceding siblings to find a doc comment
    let node = name_node.parent()?;

    // Look at the node immediately before this definition in the source
    let prev = node.prev_named_sibling();

    match language {
        Language::Rust => {
            let prev = prev?;
            if prev.kind() == "line_comment" || prev.kind() == "block_comment" {
                let text = prev.utf8_text(source).ok()?.to_string();
                let cleaned = clean_rust_doc(text);
                if cleaned.is_empty() { None } else { Some(cleaned) }
            } else {
                None
            }
        }
        Language::Python => {
            // First statement in function/class body being a string literal
            let body = node.child_by_field_name("body")?;
            let first = body.named_child(0)?;
            if first.kind() == "expression_statement" {
                let expr = first.named_child(0)?;
                if expr.kind() == "string" {
                    let text = expr.utf8_text(source).ok()?.to_string();
                    let cleaned = clean_python_docstring(text);
                    if cleaned.is_empty() { None } else { Some(cleaned) }
                } else {
                    None
                }
            } else {
                None
            }
        }
        Language::TypeScript | Language::JavaScript => {
            // JSDoc comment before the node
            let prev = prev?;
            if prev.kind() == "comment" {
                let text = prev.utf8_text(source).ok()?.to_string();
                let cleaned = clean_jsdoc(text);
                if cleaned.is_empty() { None } else { Some(cleaned) }
            } else {
                None
            }
        }
        Language::Go => {
            // Comment block ending on the line before the declaration
            let prev = prev?;
            if prev.kind() == "comment" {
                let text = prev.utf8_text(source).ok()?.to_string();
                let cleaned = clean_go_comment(text);
                if cleaned.is_empty() { None } else { Some(cleaned) }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn clean_rust_doc(s: String) -> String {
    let mut out = String::new();
    for line in s.lines() {
        let line = line.trim();
        let line = if let Some(rest) = line.strip_prefix("///") {
            rest.trim()
        } else if let Some(rest) = line.strip_prefix("//!") {
            rest.trim()
        } else if let Some(rest) = line.strip_prefix("/**") {
            rest.trim().trim_end_matches("*/").trim()
        } else if let Some(rest) = line.strip_prefix('*') {
            rest.trim()
        } else {
            line
        };
        out.push_str(line);
        out.push('\n');
    }
    out.trim().chars().take(500).collect()
}

fn clean_python_docstring(s: String) -> String {
    let s = s.trim();
    let s = s.trim_matches('"').trim_matches('\'');
    let s = s.trim_start_matches("\"\"\"").trim_end_matches("\"\"\"");
    let s = s.trim_start_matches("'''").trim_end_matches("'''");
    s.trim().chars().take(500).collect()
}

fn clean_jsdoc(s: String) -> String {
    let mut out = String::new();
    for line in s.lines() {
        let line = line.trim();
        let line = if let Some(rest) = line.strip_prefix("/**") {
            rest.trim()
        } else if line.starts_with("*/") {
            continue;
        } else if let Some(rest) = line.strip_prefix('*') {
            rest.trim()
        } else if let Some(rest) = line.strip_prefix("//") {
            rest.trim()
        } else {
            line
        };
        out.push_str(line);
        out.push('\n');
    }
    out.trim().chars().take(500).collect()
}

fn clean_go_comment(s: String) -> String {
    let mut out = String::new();
    for line in s.lines() {
        let line = line.trim();
        let line = if let Some(rest) = line.strip_prefix("//") {
            rest.trim()
        } else {
            line
        };
        out.push_str(line);
        out.push('\n');
    }
    out.trim().chars().take(500).collect()
}

// Helper for extract_body_text used by the watcher
pub fn extract_body_text(symbol: &Symbol, source: &[u8]) -> String {
    let start = symbol.span.start_byte as usize;
    let end   = symbol.span.end_byte as usize;
    let end   = end.min(source.len());
    if start >= source.len() {
        return String::new();
    }
    String::from_utf8_lossy(&source[start..end]).into_owned()
}
