//! Integration tests for graphrelay-store.
use graphrelay_core::types::{
    Edge, EdgeKind, FileRecord, Language, Span, Symbol, SymbolId, SymbolKind,
};
use graphrelay_store::GraphDb;
use std::path::PathBuf;

// ── GraphDb helpers ───────────────────────────────────────────────────────────

fn make_symbol(file_id: u64, name: &str, id: SymbolId) -> Symbol {
    Symbol {
        id,
        file_id,
        name: name.to_string(),
        qualified_name: format!("test.rs::{name}"),
        kind: SymbolKind::Function,
        span: Span { start_byte: 0, end_byte: 100, start_line: 0, end_line: 5 },
        docstring: None,
        language: Language::Rust,
        file_path: PathBuf::from("test.rs"),
    }
}

fn make_file_record(id: u64) -> FileRecord {
    FileRecord {
        id,
        path: PathBuf::from("test.rs"),
        checksum: 12345,
        language: Language::Rust,
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        byte_size: 1024,
    }
}

fn make_edge(from: SymbolId, to: SymbolId) -> Edge {
    Edge { from_id: from, to_id: Some(to), to_name: None, kind: EdgeKind::Calls, source: Default::default() }
}

// ── GraphDb: additional coverage ─────────────────────────────────────────────

#[test]
fn graphdb_upsert_and_count_symbols() {
    let db = GraphDb::open_in_memory().unwrap();
    let file_rec = make_file_record(1);
    db.upsert_file(&file_rec).unwrap();

    let syms = vec![
        make_symbol(1, "alpha", 1100),
        make_symbol(1, "beta",  1200),
        make_symbol(1, "gamma", 1300),
    ];
    db.upsert_symbols(&syms).unwrap();

    let s = db.symbol_by_id(1100).unwrap().expect("alpha not found");
    assert_eq!(s.name, "alpha");
    let s = db.symbol_by_id(1200).unwrap().expect("beta not found");
    assert_eq!(s.name, "beta");
    let s = db.symbol_by_id(1300).unwrap().expect("gamma not found");
    assert_eq!(s.name, "gamma");

    assert_eq!(db.in_degree(1300).unwrap(), 0);
}

#[test]
fn graphdb_callers_callees_via_edges() {
    let db = GraphDb::open_in_memory().unwrap();
    let file_rec = make_file_record(1);
    db.upsert_file(&file_rec).unwrap();

    let syms = vec![make_symbol(1, "caller", 2001), make_symbol(1, "callee", 2002)];
    db.upsert_symbols(&syms).unwrap();

    let edges = vec![make_edge(2001, 2002)];
    db.upsert_edges(&edges).unwrap();

    let callees = db.callees_of(2001).unwrap();
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0].name, "callee");

    let callers = db.callers_of(2002).unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].name, "caller");

    assert_eq!(db.in_degree(2002).unwrap(), 1);
}

#[test]
fn graphdb_delete_file_cascades_symbols_and_edges() {
    let db = GraphDb::open_in_memory().unwrap();
    let file_rec = make_file_record(42);
    db.upsert_file(&file_rec).unwrap();

    let syms = vec![make_symbol(42, "a", 3010), make_symbol(42, "b", 3020)];
    db.upsert_symbols(&syms).unwrap();
    let edges = vec![make_edge(3010, 3020)];
    db.upsert_edges(&edges).unwrap();

    assert!(db.symbol_by_id(3010).unwrap().is_some());

    db.delete_edges_for_file(42).unwrap();
    db.delete_symbols_for_file(42).unwrap();

    assert!(db.symbol_by_id(3010).unwrap().is_none(), "symbol should be removed after file delete");
    assert!(db.symbol_by_id(3020).unwrap().is_none());

    let callers = db.callers_of(3020).unwrap();
    assert!(callers.is_empty(), "edge should be removed after file delete");
}

#[test]
fn graphdb_search_by_name() {
    let db = GraphDb::open_in_memory().unwrap();
    // Use a unique path to avoid conflict with other tests
    let file_rec = FileRecord {
        id: 4001,
        path: PathBuf::from("search_test.rs"),
        checksum: 12345,
        language: Language::Rust,
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        byte_size: 1024,
    };
    db.upsert_file(&file_rec).unwrap();

    let syms = vec![
        Symbol {
            id: 4001,
            file_id: 4001,
            name: "process_data".to_string(),
            qualified_name: "search_test.rs::process_data".to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 100, start_line: 0, end_line: 5 },
            docstring: None,
            language: Language::Rust,
            file_path: PathBuf::from("search_test.rs"),
        },
        Symbol {
            id: 4002,
            file_id: 4001,
            name: "process_batch".to_string(),
            qualified_name: "search_test.rs::process_batch".to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 100, start_line: 0, end_line: 5 },
            docstring: None,
            language: Language::Rust,
            file_path: PathBuf::from("search_test.rs"),
        },
        Symbol {
            id: 4003,
            file_id: 4001,
            name: "validate_input".to_string(),
            qualified_name: "search_test.rs::validate_input".to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 100, start_line: 0, end_line: 5 },
            docstring: None,
            language: Language::Rust,
            file_path: PathBuf::from("search_test.rs"),
        },
    ];
    db.upsert_symbols(&syms).unwrap();

    let results = db.search_symbols_by_name("process", 10).unwrap();
    assert_eq!(results.len(), 2, "search should find both process_* symbols");

    let results = db.search_symbols_by_name("validate", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "validate_input");
}

#[test]
fn graphdb_upsert_is_idempotent() {
    let db = GraphDb::open_in_memory().unwrap();
    let file_rec = make_file_record(1);
    db.upsert_file(&file_rec).unwrap();

    let sym = make_symbol(1, "my_fn", 5999);
    db.upsert_symbols(std::slice::from_ref(&sym)).unwrap();
    db.upsert_symbols(&[sym]).unwrap();

    let found = db.symbol_by_id(5999).unwrap().expect("symbol not found");
    assert_eq!(found.name, "my_fn");
}
