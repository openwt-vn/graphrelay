//! Heuristic C/C++ function-pointer dispatch synthesis.
//!
//! Recovers `dispatcher -> handler` call edges for the common
//! "struct-of-function-pointers" idiom:
//!
//! ```c
//! struct cmd { const char *name; void (*fn)(int argc, char **argv); };
//! static struct cmd cmds[] = { {"add", cmd_add}, {"remove", cmd_remove} };
//! ...
//! cmds[i].fn(argc, argv);   // or: p->fn(argv)
//! ```
//!
//! A plain tree-sitter call-expression query never sees an edge here —
//! there is no literal `cmd_add(...)` call site, only a call through a
//! struct field. This pass is a text-pattern heuristic, not a type checker:
//! it extracts (a) which fields of which `struct` definitions are
//! function-pointer-typed, (b) which functions get assigned to those fields
//! in initializers — both designated (`.field = fn`) and positional (using
//! declaration order) — then (c) links every call site through that field
//! name to every function ever assigned to it anywhere in the corpus.
//!
//! Known imprecision, by design: if two unrelated structs happen to share a
//! function-pointer field name, a call through either is linked to handlers
//! registered on both — an over-approximation, not a false structural miss.
//! Nested/complex initializers, macros, and multi-statement field
//! declarations are not specially handled; this targets the common,
//! textually-simple form of the idiom.

use graphrelay_core::types::{Edge, EdgeKind, EdgeSource, Language, Symbol, SymbolId};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

fn struct_def_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)struct\s+(\w+)\s*\{([^{}]*)\}").unwrap())
}

fn fnptr_field_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\(\s*\*\s*(\w+)\s*\)\s*\(").unwrap())
}

fn last_identifier_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z_]\w*").unwrap())
}

fn designated_init_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.(\w+)\s*=\s*(\w+)").unwrap())
}

fn typed_init_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // e.g. `struct cmd cmds[] = { ... };` or `struct cmd single = { ... };`
    RE.get_or_init(|| Regex::new(r"(?s)struct\s+(\w+)\s+\w+(?:\[\s*\])?\s*=\s*\{(.*?)\}\s*;").unwrap())
}

fn dispatch_call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // `.field(` or `->field(` — a call through a struct/pointer field.
    RE.get_or_init(|| Regex::new(r"(?:->|\.)\s*(\w+)\s*\(").unwrap())
}

/// One field per `;`-terminated declaration statement in a struct body, in
/// declaration order, tagged with whether it's a function-pointer field.
fn parse_struct_fields(body: &str) -> Vec<(String, bool)> {
    let mut fields = Vec::new();
    for stmt in body.split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        if let Some(cap) = fnptr_field_re().captures(stmt) {
            fields.push((cap[1].to_string(), true));
        } else {
            // Strip a trailing array suffix like "[8]" before taking the
            // last identifier as the field name (skips a leading type name
            // like "int" only in the common single-word-type case; this is
            // an intentionally scoped heuristic, not a C parser).
            let stmt = stmt.split('[').next().unwrap_or(stmt);
            if let Some(m) = last_identifier_re().find_iter(stmt).last() {
                fields.push((m.as_str().to_string(), false));
            }
        }
    }
    fields
}

/// Split a brace-delimited initializer list's body into each top-level
/// `{...}` entry's inner text — e.g. ` {"add", cmd_add}, {"remove", cmd_remove} `
/// becomes `["\"add\", cmd_add", "\"remove\", cmd_remove"]`.
fn split_brace_entries(body: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut capturing = false;
    for c in body.chars() {
        match c {
            '{' => {
                depth += 1;
                if depth == 1 {
                    capturing = true;
                    current.clear();
                    continue;
                }
                current.push(c);
            }
            '}' => {
                depth -= 1;
                if depth == 0 && capturing {
                    entries.push(std::mem::take(&mut current));
                    capturing = false;
                    continue;
                }
                current.push(c);
            }
            _ => {
                if capturing {
                    current.push(c);
                }
            }
        }
    }
    entries
}

/// Split one initializer entry's contents on top-level commas, respecting
/// simple `"..."` string literals and one level of bracket nesting so a
/// comma inside a string or nested initializer doesn't split the entry.
fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut current = String::new();
    for c in s.chars() {
        match c {
            '"' => {
                in_string = !in_string;
                current.push(c);
            }
            '(' | '{' | '[' if !in_string => {
                depth += 1;
                current.push(c);
            }
            ')' | '}' | ']' if !in_string => {
                depth -= 1;
                current.push(c);
            }
            ',' if !in_string && depth == 0 => {
                parts.push(std::mem::take(&mut current).trim().to_string());
            }
            _ => current.push(c),
        }
    }
    let last = current.trim();
    if !last.is_empty() {
        parts.push(last.to_string());
    }
    parts
}

fn is_c_like(lang: Language) -> bool {
    matches!(lang, Language::C | Language::Cpp)
}

/// `name -> matching C/C++ symbols` index. Cheap (no source text touched) —
/// computed once from the corpus's full symbol set and reused across both
/// phases, and across every chunk when the caller processes files in bounded
/// batches (see `build_field_handlers`/`scan_call_sites`).
pub fn c_functions_by_name(symbols: &[Symbol]) -> HashMap<&str, Vec<&Symbol>> {
    let mut functions_by_name: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    for sym in symbols {
        if is_c_like(sym.language) {
            functions_by_name.entry(sym.name.as_str()).or_default().push(sym);
        }
    }
    functions_by_name
}

/// Phase 1: scan `sources`' text for dispatch-table field-pointer
/// assignments, resolving handler names against the full-corpus
/// `functions_by_name` index. `sources` may be any subset of the corpus's
/// C/C++ files (e.g. one bounded chunk) — callers processing a large real
/// repo in chunks call this once per chunk and merge the returned maps,
/// rather than materializing every file's text in one `HashMap` at once (the
/// latter was observed to reach double-digit-GB resident memory on
/// `apple/swift`'s ~5k-file C++ corpus — see `run_dispatch_synthesis`).
pub fn build_field_handlers(
    functions_by_name: &HashMap<&str, Vec<&Symbol>>,
    sources: &HashMap<PathBuf, String>,
) -> HashMap<String, HashSet<SymbolId>> {
    let mut field_handlers: HashMap<String, HashSet<SymbolId>> = HashMap::new();

    for source in sources.values() {
        let mut struct_fields: HashMap<String, Vec<(String, bool)>> = HashMap::new();
        for cap in struct_def_re().captures_iter(source) {
            struct_fields.insert(cap[1].to_string(), parse_struct_fields(&cap[2]));
        }

        // Designated initializers: `.field = identifier`, anywhere in the file.
        for cap in designated_init_re().captures_iter(source) {
            let field = &cap[1];
            let value = &cap[2];
            if let Some(candidates) = functions_by_name.get(value) {
                let entry = field_handlers.entry(field.to_string()).or_default();
                for sym in candidates {
                    entry.insert(sym.id);
                }
            }
        }

        // Positional initializers for a known struct type: `struct T x[] = { {v0, v1}, ... };`.
        for cap in typed_init_re().captures_iter(source) {
            let type_name = &cap[1];
            let body = &cap[2];
            let Some(fields) = struct_fields.get(type_name) else { continue };
            let Some(fn_field_pos) = fields.iter().position(|(_, is_fn)| *is_fn) else { continue };
            let fn_field_name = &fields[fn_field_pos].0;

            for entry in split_brace_entries(body) {
                let values = split_top_level_commas(&entry);
                let Some(value) = values.get(fn_field_pos) else { continue };
                if let Some(candidates) = functions_by_name.get(value.as_str()) {
                    let slot = field_handlers.entry(fn_field_name.clone()).or_default();
                    for sym in candidates {
                        slot.insert(sym.id);
                    }
                }
            }
        }
    }

    field_handlers
}

/// Phase 2: call sites through any known dispatch field, scoped to each
/// symbol's own byte span so the edge's `from_id` is the enclosing function.
/// `field_handlers` must already be complete across the whole corpus (built
/// via one or more `build_field_handlers` calls) — a handler registered in
/// one chunk can be the target of a call site scanned from a different
/// chunk, so this phase must not start until every chunk's phase 1 has run.
pub fn scan_call_sites(
    symbols: &[Symbol],
    sources: &HashMap<PathBuf, String>,
    field_handlers: &HashMap<String, HashSet<SymbolId>>,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    if field_handlers.is_empty() {
        return edges;
    }

    for sym in symbols {
        if !is_c_like(sym.language) {
            continue;
        }
        let Some(source) = sources.get(&sym.file_path) else { continue };
        let start = sym.span.start_byte as usize;
        let end = (sym.span.end_byte as usize).min(source.len());
        if start >= source.len() || start >= end {
            continue;
        }
        let Some(body_text) = source.get(start..end) else { continue };

        for cap in dispatch_call_re().captures_iter(body_text) {
            let field = &cap[1];
            let Some(handlers) = field_handlers.get(field) else { continue };
            for &to_id in handlers {
                if to_id == sym.id {
                    continue; // skip trivial self-loop noise
                }
                edges.push(Edge {
                    from_id: sym.id,
                    to_id: Some(to_id),
                    to_name: None,
                    kind: EdgeKind::Calls,
                    source: EdgeSource::Synthesized,
                });
            }
        }
    }

    edges
}

/// Convenience single-shot API for small/synthetic inputs (tests, small
/// repos) where materializing every file's text in one `HashMap` is
/// harmless. Real large-repo callers should use `build_field_handlers` /
/// `scan_call_sites` directly in bounded chunks instead — see their doc
/// comments.
pub fn synthesize(symbols: &[Symbol], sources: &HashMap<PathBuf, String>) -> Vec<Edge> {
    let functions_by_name = c_functions_by_name(symbols);
    if functions_by_name.is_empty() {
        return Vec::new();
    }
    let field_handlers = build_field_handlers(&functions_by_name, sources);
    if field_handlers.is_empty() {
        return Vec::new();
    }
    scan_call_sites(symbols, sources, &field_handlers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphrelay_core::types::{Span, SymbolKind};
    use std::path::PathBuf;

    fn make_symbol(id: SymbolId, name: &str, source: &str, needle: &str) -> Symbol {
        let start = source.find(needle).expect("needle not found in fixture source");
        Symbol {
            id,
            file_id: 1,
            name: name.to_string(),
            qualified_name: format!("dispatch.c::{name}"),
            kind: SymbolKind::Function,
            span: Span {
                start_byte: start as u32,
                end_byte: (start + needle.len()) as u32,
                start_line: 0,
                end_line: 0,
            },
            docstring: None,
            language: Language::C,
            file_path: PathBuf::from("dispatch.c"),
        }
    }

    /// The exact scenario from this module's original success criterion:
    /// a positional `static struct cmd cmds[] = {{"add", cmd_add}}`-style
    /// dispatch table, plus a `p->fn(argv)` call site inside `dispatch()`.
    #[test]
    fn positional_dispatch_table_produces_synthesized_edge() {
        let source = r#"
struct cmd {
    const char *name;
    void (*fn)(int argc, char **argv);
};

static struct cmd cmds[] = {
    {"add", cmd_add},
    {"remove", cmd_remove},
};

void cmd_add(int argc, char **argv) { }
void cmd_remove(int argc, char **argv) { }

void dispatch(struct cmd *p, int argc, char **argv) {
    p->fn(argv);
}
"#
        .to_string();

        let dispatch_needle = "void dispatch(struct cmd *p, int argc, char **argv) {\n    p->fn(argv);\n}";
        let cmd_add_needle = "void cmd_add(int argc, char **argv) { }";
        let cmd_remove_needle = "void cmd_remove(int argc, char **argv) { }";

        let dispatch_sym = make_symbol(1, "dispatch", &source, dispatch_needle);
        let cmd_add_sym = make_symbol(2, "cmd_add", &source, cmd_add_needle);
        let cmd_remove_sym = make_symbol(3, "cmd_remove", &source, cmd_remove_needle);
        let symbols = vec![dispatch_sym.clone(), cmd_add_sym.clone(), cmd_remove_sym.clone()];

        let mut sources = HashMap::new();
        sources.insert(PathBuf::from("dispatch.c"), source);

        // Absence check: a plain 3-stage resolver (name-matching on literal
        // call expressions only) would never produce this edge — `p->fn(...)`
        // has no callee named "cmd_add" or "cmd_remove" anywhere in its text,
        // it calls through the field name "fn". Confirm there is genuinely no
        // textual "cmd_add(" / "cmd_remove(" call inside dispatch()'s span.
        assert!(
            !dispatch_needle.contains("cmd_add(") && !dispatch_needle.contains("cmd_remove("),
            "fixture invalid: dispatch() must not contain a literal call to either handler",
        );

        // Presence check: the synthesizer recovers both edges.
        let edges = synthesize(&symbols, &sources);
        let targets: HashSet<SymbolId> = edges
            .iter()
            .filter(|e| e.from_id == dispatch_sym.id)
            .filter_map(|e| e.to_id)
            .collect();
        assert!(
            targets.contains(&cmd_add_sym.id),
            "expected dispatch->cmd_add synthesized edge, got: {edges:?}",
        );
        assert!(
            targets.contains(&cmd_remove_sym.id),
            "expected dispatch->cmd_remove synthesized edge, got: {edges:?}",
        );
        assert!(
            edges.iter().all(|e| e.source == EdgeSource::Synthesized),
            "all edges from this pass must be tagged Synthesized",
        );
    }

    #[test]
    fn designated_initializer_dispatch_produces_synthesized_edge() {
        let source = r#"
struct handler_entry {
    const char *event;
    void (*handler)(void);
};

void on_click(void) { }

static struct handler_entry handlers[] = {
    { .event = "click", .handler = on_click },
};

void wire_up(struct handler_entry *e) {
    e->handler();
}
"#
        .to_string();

        let wire_up_needle = "void wire_up(struct handler_entry *e) {\n    e->handler();\n}";
        let on_click_needle = "void on_click(void) { }";

        let wire_up_sym = make_symbol(10, "wire_up", &source, wire_up_needle);
        let on_click_sym = make_symbol(11, "on_click", &source, on_click_needle);
        let symbols = vec![wire_up_sym.clone(), on_click_sym.clone()];

        let mut sources = HashMap::new();
        sources.insert(PathBuf::from("dispatch.c"), source);

        let edges = synthesize(&symbols, &sources);
        assert!(
            edges.iter().any(|e| e.from_id == wire_up_sym.id && e.to_id == Some(on_click_sym.id)),
            "expected wire_up->on_click synthesized edge from a designated initializer, got: {edges:?}",
        );
    }

    #[test]
    fn non_c_language_symbols_produce_no_edges() {
        let source = "fn dispatch() { p.fn(); }".to_string();
        let sym = Symbol {
            id: 1,
            file_id: 1,
            name: "dispatch".to_string(),
            qualified_name: "dispatch.rs::dispatch".to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: source.len() as u32, start_line: 0, end_line: 0 },
            docstring: None,
            language: Language::Rust,
            file_path: PathBuf::from("dispatch.rs"),
        };
        let mut sources = HashMap::new();
        sources.insert(PathBuf::from("dispatch.rs"), source);
        let edges = synthesize(&[sym], &sources);
        assert!(edges.is_empty(), "synthesis pass must be a no-op for non-C/C++ symbols");
    }

    #[test]
    fn empty_input_returns_no_edges() {
        assert!(synthesize(&[], &HashMap::new()).is_empty());
    }

    /// Regression test for the large-repo memory fix (see
    /// `bench-reports/large-repo-indexing-2026-08-04.md`): real
    /// large-repo callers (`graphrelay_watcher::worker::run_dispatch_synthesis`)
    /// no longer materialize every file's text in one `HashMap` — instead
    /// they call `build_field_handlers` once per bounded chunk of files and
    /// merge the results before running `scan_call_sites`. This simulates
    /// that two-chunk merge directly: the dispatch table lives in one file
    /// (its own `build_field_handlers` call/"chunk"), the call site through
    /// it lives in a different file (a separate `build_field_handlers`
    /// call/"chunk"), and the two chunks' `field_handlers` maps are merged
    /// exactly like `run_dispatch_synthesis` does — proving a handler
    /// registered in one chunk is still resolved for a call site scanned
    /// from a different chunk, not silently dropped by the chunking.
    #[test]
    fn cross_chunk_field_handlers_merge_resolves_dispatch_across_files() {
        let handlers_source = r#"
struct cmd {
    const char *name;
    void (*fn)(int argc, char **argv);
};

void cmd_add(int argc, char **argv) { }

static struct cmd cmds[] = {
    {"add", cmd_add},
};
"#
        .to_string();

        let dispatcher_source = r#"
void dispatch(struct cmd *p, int argc, char **argv) {
    p->fn(argv);
}
"#
        .to_string();

        let cmd_add_sym = make_symbol(1, "cmd_add", &handlers_source, "void cmd_add(int argc, char **argv) { }");
        let dispatch_needle = "void dispatch(struct cmd *p, int argc, char **argv) {\n    p->fn(argv);\n}";
        let mut dispatch_sym = make_symbol(2, "dispatch", &dispatcher_source, dispatch_needle);
        dispatch_sym.file_path = PathBuf::from("dispatcher.c");

        let symbols = vec![cmd_add_sym.clone(), dispatch_sym.clone()];
        let functions_by_name = c_functions_by_name(&symbols);

        // Chunk A: only the file containing the dispatch table/handler.
        let mut chunk_a_sources = HashMap::new();
        chunk_a_sources.insert(PathBuf::from("dispatch.c"), handlers_source);
        let handlers_a = build_field_handlers(&functions_by_name, &chunk_a_sources);

        // Chunk B: only the file containing the call site — no struct
        // definition or initializer visible in this chunk at all.
        let mut chunk_b_sources = HashMap::new();
        chunk_b_sources.insert(PathBuf::from("dispatcher.c"), dispatcher_source);
        let handlers_b = build_field_handlers(&functions_by_name, &chunk_b_sources);
        assert!(
            handlers_b.is_empty(),
            "chunk B alone (call-site file only) must find no handlers — proves the test isn't trivially passing",
        );

        // Merge exactly as `run_dispatch_synthesis` does.
        let mut field_handlers: HashMap<String, HashSet<SymbolId>> = HashMap::new();
        for (field, ids) in handlers_a {
            field_handlers.entry(field).or_default().extend(ids);
        }
        for (field, ids) in handlers_b {
            field_handlers.entry(field).or_default().extend(ids);
        }

        // Phase 2 scan against chunk B's symbols/sources only — the merged
        // map must still resolve `dispatch`'s call site to `cmd_add`, whose
        // registration was only ever visible in chunk A.
        let edges = scan_call_sites(&[dispatch_sym.clone()], &chunk_b_sources, &field_handlers);
        assert!(
            edges.iter().any(|e| e.from_id == dispatch_sym.id && e.to_id == Some(cmd_add_sym.id)),
            "expected dispatch->cmd_add edge resolved across chunks, got: {edges:?}",
        );
    }
}
