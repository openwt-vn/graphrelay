use graphrelay_core::types::{
    Edge, EdgeKind, FileRecord, IndexedChunk, Language, Span, Symbol, SymbolKind,
};
use graphrelay_store::GraphDb;
use std::path::PathBuf;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn file(id: u64, path: &str) -> FileRecord {
    FileRecord {
        id,
        path:       PathBuf::from(path),
        checksum:   99,
        language:   Language::Rust,
        indexed_at: "2026-01-01T00:00:00Z".to_string(),
        byte_size:  256,
    }
}

fn sym(id: u64, file_id: u64, name: &str, path: &str) -> Symbol {
    Symbol {
        id,
        file_id,
        name:           name.to_string(),
        qualified_name: format!("{path}::{name}"),
        kind:           SymbolKind::Function,
        span:           Span { start_byte: 0, end_byte: 50, start_line: 0, end_line: 3 },
        docstring:      None,
        language:       Language::Rust,
        file_path:      PathBuf::from(path),
    }
}

fn calls(from: u64, to: u64) -> Edge {
    Edge { from_id: from, to_id: Some(to), to_name: None, kind: EdgeKind::Calls, source: Default::default() }
}

fn edge_kind(from: u64, to: u64, kind: EdgeKind) -> Edge {
    Edge { from_id: from, to_id: Some(to), to_name: None, kind, source: Default::default() }
}

// ── File CRUD ─────────────────────────────────────────────────────────────────

#[test]
fn file_upsert_and_lookup() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(1, "src/main.rs");
    db.upsert_file(&f).unwrap();

    let got = db.file_by_path(&f.path).unwrap().expect("file not found");
    assert_eq!(got.id, 1);
    assert_eq!(got.language, Language::Rust);
}

#[test]
fn file_upsert_is_idempotent() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(2, "src/lib.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_file(&f).unwrap(); // second upsert must not error
    assert_eq!(db.total_file_count().unwrap(), 1);
}

#[test]
fn missing_file_returns_none() {
    let db = GraphDb::open_in_memory().unwrap();
    assert!(db.file_by_path(std::path::Path::new("nope.rs")).unwrap().is_none());
}

#[test]
fn files_by_suffix_matches_relative_path_against_absolute_index() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/erpnext/stock/doctype/item/item.py")).unwrap();

    let matches = db.files_by_suffix(std::path::Path::new("erpnext/stock/doctype/item/item.py")).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].id, 1);
}

#[test]
fn files_by_suffix_does_not_match_underscore_as_wildcard() {
    // '_' is a single-char LIKE wildcard — must be escaped, or "item.py" would
    // also spuriously match a hypothetical "itemXpy" (or similar) sibling.
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/delivery_note.py")).unwrap();
    db.upsert_file(&file(2, "/repo/deliveryXnote.py")).unwrap();

    let matches = db.files_by_suffix(std::path::Path::new("delivery_note.py")).unwrap();
    assert_eq!(matches.iter().map(|f| f.id).collect::<Vec<_>>(), vec![1]);
}

#[test]
fn files_by_suffix_reports_ambiguity_across_multiple_matches() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/erpnext/erpnext/utils.py")).unwrap();
    db.upsert_file(&file(2, "/repo/erpnext/hr/utils.py")).unwrap();

    let matches = db.files_by_suffix(std::path::Path::new("utils.py")).unwrap();
    assert_eq!(matches.len(), 2);
}

#[test]
fn files_by_suffix_returns_empty_for_no_match() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/src/main.rs")).unwrap();
    assert!(db.files_by_suffix(std::path::Path::new("nope.rs")).unwrap().is_empty());
}

// ── Symbol CRUD ───────────────────────────────────────────────────────────────

#[test]
fn symbol_upsert_and_lookup_by_id() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(10, "foo.rs");
    db.upsert_file(&f).unwrap();

    let s = sym(100, 10, "my_fn", "foo.rs");
    db.upsert_symbols(std::slice::from_ref(&s)).unwrap();

    let got = db.symbol_by_id(100).unwrap().expect("symbol not found");
    assert_eq!(got.name, "my_fn");
    assert_eq!(got.kind, SymbolKind::Function);
}

#[test]
fn symbol_lookup_by_qualified_name() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(11, "bar.rs");
    db.upsert_file(&f).unwrap();

    let s = sym(101, 11, "alpha", "bar.rs");
    db.upsert_symbols(&[s]).unwrap();

    let got = db.symbol_by_qualified("bar.rs::alpha").unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().id, 101);
}

#[test]
fn symbols_in_file_returns_all() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(20, "multi.rs");
    db.upsert_file(&f).unwrap();

    let syms = vec![sym(200, 20, "a", "multi.rs"), sym(201, 20, "b", "multi.rs")];
    db.upsert_symbols(&syms).unwrap();

    let got = db.symbols_in_file(20).unwrap();
    assert_eq!(got.len(), 2);
}

#[test]
fn symbol_count_matches_upserted() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(30, "count.rs");
    db.upsert_file(&f).unwrap();

    db.upsert_symbols(&[sym(300, 30, "x", "count.rs"), sym(301, 30, "y", "count.rs")]).unwrap();
    assert_eq!(db.total_symbol_count().unwrap(), 2);
}

// ── Edge operations ───────────────────────────────────────────────────────────

#[test]
fn callers_and_callees_roundtrip() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(40, "edges.rs");
    db.upsert_file(&f).unwrap();

    // caller (id=400) calls callee (id=401)
    db.upsert_symbols(&[sym(400, 40, "caller_fn", "edges.rs"), sym(401, 40, "callee_fn", "edges.rs")]).unwrap();
    db.upsert_edges(&[calls(400, 401)]).unwrap();

    let callers = db.callers_of(401).unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].id, 400);

    let callees = db.callees_of(400).unwrap();
    assert_eq!(callees.len(), 1);
    assert_eq!(callees[0].id, 401);
}

#[test]
fn call_neighbor_names_for_file_matches_per_symbol_queries() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(41, "a.rs")).unwrap();
    db.upsert_file(&file(42, "b.rs")).unwrap();
    let mut syms = vec![sym(410, 41, "target", "a.rs"), sym(411, 41, "helper", "a.rs")];
    for i in 0..8u64 {
        syms.push(sym(420 + i, 42, &format!("caller{i}"), "b.rs"));
    }
    db.upsert_symbols(&syms).unwrap();
    let mut edges: Vec<Edge> = (0..8u64).rev().map(|i| calls(420 + i, 410)).collect();
    edges.push(calls(410, 411));
    db.upsert_edges(&edges).unwrap();

    let names = db.call_neighbor_names_for_file(41, 5).unwrap();
    for id in [410u64, 411] {
        let expect_callers: Vec<String> = db.callers_of(id).unwrap().into_iter().take(5).map(|s| s.name).collect();
        let expect_callees: Vec<String> = db.callees_of(id).unwrap().into_iter().take(5).map(|s| s.name).collect();
        let (callers, callees) = names.get(&id).cloned().unwrap_or_default();
        assert_eq!(callers, expect_callers, "callers of {id}");
        assert_eq!(callees, expect_callees, "callees of {id}");
    }
    assert_eq!(names[&410].0.len(), 5);
    assert!(!names.contains_key(&420), "symbols of other files are not keyed");
}

#[test]
fn neighbors_includes_calls_inherits_references_and_implements_but_not_imports() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(41, "neighbors.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[
        sym(410, 41, "center", "neighbors.rs"),
        sym(411, 41, "callee", "neighbors.rs"),
        sym(412, 41, "base_class", "neighbors.rs"),
        sym(413, 41, "referenced_type", "neighbors.rs"),
        sym(414, 41, "unrelated_import_target", "neighbors.rs"),
        sym(415, 41, "implemented_trait", "neighbors.rs"),
    ]).unwrap();
    db.upsert_edges(&[
        edge_kind(410, 411, EdgeKind::Calls),
        edge_kind(410, 412, EdgeKind::Inherits),
        edge_kind(410, 413, EdgeKind::References),
        edge_kind(410, 414, EdgeKind::Imports),
        edge_kind(410, 415, EdgeKind::Implements),
    ]).unwrap();

    let neighbor_ids: std::collections::HashSet<u64> =
        db.neighbors(410, 1).unwrap().into_iter().map(|s| s.id).collect();

    assert!(neighbor_ids.contains(&411), "Calls neighbor missing");
    assert!(neighbor_ids.contains(&412), "Inherits neighbor missing — expand_graph widening regressed");
    assert!(neighbor_ids.contains(&413), "References neighbor missing — expand_graph widening regressed");
    assert!(neighbor_ids.contains(&415), "Implements neighbor missing — the edge kind this same changeset starts populating (Rust impl Trait for Type) must be walked too");
    assert!(!neighbor_ids.contains(&414), "Imports edges should not be walked by neighbors()");

    // callers_of/callees_of must stay Calls-only — trace_impact, explain_symbol,
    // personalized_pagerank, and the REST symbol-detail endpoint depend on this.
    assert_eq!(db.callees_of(410).unwrap().len(), 1);
    assert_eq!(db.callees_of(410).unwrap()[0].id, 411);
}

#[test]
fn neighbors_zero_hops_returns_empty() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(42, "zerohops.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[sym(420, 42, "a", "zerohops.rs"), sym(421, 42, "b", "zerohops.rs")]).unwrap();
    db.upsert_edges(&[calls(420, 421)]).unwrap();

    assert!(db.neighbors(420, 0).unwrap().is_empty());
}

#[test]
fn edges_between_returns_internal_edges() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(50, "between.rs");
    db.upsert_file(&f).unwrap();

    db.upsert_symbols(&[sym(500, 50, "p", "between.rs"), sym(501, 50, "q", "between.rs")]).unwrap();
    db.upsert_edges(&[calls(500, 501)]).unwrap();

    let edges = db.edges_between(&[500, 501]).unwrap();
    assert!(!edges.is_empty(), "expected at least one edge");
    assert!(edges.iter().any(|e| e.from_id == 500 && e.to_id == Some(501)));
}

#[test]
fn in_degree_reflects_incoming_calls() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(60, "degree.rs");
    db.upsert_file(&f).unwrap();

    db.upsert_symbols(&[sym(600, 60, "hot", "degree.rs"), sym(601, 60, "c1", "degree.rs"), sym(602, 60, "c2", "degree.rs")]).unwrap();
    db.upsert_edges(&[calls(601, 600), calls(602, 600)]).unwrap();

    assert_eq!(db.in_degree(600).unwrap(), 2);
}

// ── find_implementations (implementors_of) ────────────────────────────────────

/// Plan 21 item 4: `implementors_of` previously included `References` edges,
/// which are far more common than real `Inherits`/`Implements` edges (any
/// parameter/return-type mention counts) — measured on keycloak,
/// `RealmModel` had 2,483 `References` edges against only 4 real `Inherits`
/// edges, so real implementors were entirely drowned out by parameter-
/// mention noise. This asserts a pure `References` edge is excluded.
#[test]
fn implementors_of_excludes_references_edges() {
    let db = GraphDb::open_in_memory().unwrap();
    let f = file(70, "impl.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[
        sym(700, 70, "Base", "impl.rs"),
        sym(701, 70, "real_implementor", "impl.rs"),
        sym(702, 70, "just_mentions_it", "impl.rs"),
    ]).unwrap();
    db.upsert_edges(&[
        edge_kind(701, 700, EdgeKind::Implements),
        edge_kind(702, 700, EdgeKind::References),
    ]).unwrap();

    let ids: std::collections::HashSet<u64> =
        db.implementors_of(700).unwrap().into_iter().map(|s| s.id).collect();

    assert!(ids.contains(&701), "real Implements edge must be returned");
    assert!(!ids.contains(&702), "a mere References edge must not count as an implementation");
}

/// Plan 21 item 4: implementors must be found transitively over `Inherits`,
/// not just direct implementors — `RealmModel` in keycloak had implementors
/// reachable only via `SomeAdapter extends SomeInterface extends RealmModel`,
/// which were entirely invisible before this fix.
#[test]
fn implementors_of_follows_transitive_inherits_chain() {
    let db = GraphDb::open_in_memory().unwrap();
    let f = file(71, "chain.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[
        sym(710, 71, "RootInterface", "chain.rs"),
        sym(711, 71, "MiddleInterface", "chain.rs"),
        sym(712, 71, "ConcreteImpl", "chain.rs"),
    ]).unwrap();
    // ConcreteImpl implements MiddleInterface, which extends RootInterface —
    // ConcreteImpl is only an indirect (2-hop) implementor of RootInterface.
    db.upsert_edges(&[
        edge_kind(711, 710, EdgeKind::Inherits),
        edge_kind(712, 711, EdgeKind::Implements),
    ]).unwrap();

    let ids: std::collections::HashSet<u64> =
        db.implementors_of(710).unwrap().into_iter().map(|s| s.id).collect();

    assert!(ids.contains(&711), "direct implementor must be found");
    assert!(ids.contains(&712), "transitive (indirect) implementor must also be found");
}

/// A cyclic Inherits graph (malformed input, or unusual mixin semantics)
/// must not hang or error the recursive CTE — SQLite's recursive CTE
/// naturally terminates once no new rows are added, but this pins that
/// behavior against a real 2-cycle rather than trusting it un-tested.
#[test]
fn implementors_of_terminates_on_a_cycle() {
    let db = GraphDb::open_in_memory().unwrap();
    let f = file(72, "cycle.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[
        sym(720, 72, "A", "cycle.rs"),
        sym(721, 72, "B", "cycle.rs"),
    ]).unwrap();
    db.upsert_edges(&[
        edge_kind(720, 721, EdgeKind::Inherits),
        edge_kind(721, 720, EdgeKind::Inherits),
    ]).unwrap();

    let ids: std::collections::HashSet<u64> =
        db.implementors_of(720).unwrap().into_iter().map(|s| s.id).collect();
    assert!(ids.contains(&721), "the direct cyclic neighbor should still be found");
}

// ── Top symbols & language counts ─────────────────────────────────────────────

#[test]
fn top_symbols_by_in_degree_orders_correctly() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(70, "top.rs");
    db.upsert_file(&f).unwrap();

    let syms = vec![sym(700, 70, "rare", "top.rs"), sym(701, 70, "popular", "top.rs")];
    db.upsert_symbols(&syms).unwrap();
    // popular gets 3 callers, rare gets 0
    db.upsert_symbols(&[sym(710, 70, "c1", "top.rs"), sym(711, 70, "c2", "top.rs"), sym(712, 70, "c3", "top.rs")]).unwrap();
    db.upsert_edges(&[calls(710, 701), calls(711, 701), calls(712, 701)]).unwrap();

    let top = db.top_symbols_by_in_degree(10, None).unwrap();
    assert!(!top.is_empty());
    // popular should appear before rare
    let popular_pos = top.iter().position(|s| s.id == 701);
    let rare_pos    = top.iter().position(|s| s.id == 700);
    if let (Some(pp), Some(rp)) = (popular_pos, rare_pos) {
        assert!(pp < rp, "popular should rank higher than rare");
    }
}

#[test]
fn language_counts_groups_by_language() {
    let db = GraphDb::open_in_memory().unwrap();

    let f1 = FileRecord { id: 81, path: PathBuf::from("a.rs"), checksum: 1, language: Language::Rust,       indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 10 };
    let f2 = FileRecord { id: 82, path: PathBuf::from("b.rs"), checksum: 2, language: Language::Rust,       indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 10 };
    let f3 = FileRecord { id: 83, path: PathBuf::from("c.py"), checksum: 3, language: Language::Python,     indexed_at: "2026-01-01T00:00:00Z".to_string(), byte_size: 10 };
    db.upsert_file(&f1).unwrap();
    db.upsert_file(&f2).unwrap();
    db.upsert_file(&f3).unwrap();

    let counts = db.language_counts().unwrap();
    let rust_count = counts.iter().find(|(l, _)| l == "rust").map(|(_, c)| *c);
    let py_count   = counts.iter().find(|(l, _)| l == "python").map(|(_, c)| *c);
    assert_eq!(rust_count, Some(2));
    assert_eq!(py_count,   Some(1));
}

// ── Deletion ──────────────────────────────────────────────────────────────────

#[test]
fn delete_file_cascades_symbols() {
    let db = GraphDb::open_in_memory().unwrap();
    let f  = file(90, "del.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[sym(900, 90, "gone", "del.rs")]).unwrap();

    db.delete_symbols_for_file(90).unwrap();
    db.delete_file(90).unwrap();

    assert!(db.file_by_path(std::path::Path::new("del.rs")).unwrap().is_none());
    assert!(db.symbol_by_id(900).unwrap().is_none());
}

// ── Meta key-value ───────────────────────────────────────────────────────────

#[test]
fn meta_roundtrip() {
    let db = GraphDb::open_in_memory().unwrap();
    db.set_meta("version", "42").unwrap();
    let got = db.get_meta("version").unwrap();
    assert_eq!(got, Some("42".to_string()));
}

#[test]
fn meta_missing_key_returns_none() {
    let db = GraphDb::open_in_memory().unwrap();
    assert!(db.get_meta("no-such-key").unwrap().is_none());
}

// ── Edge resolution ───────────────────────────────────────────────────────────

fn unresolved(from: u64, to_name: &str) -> Edge {
    Edge { from_id: from, to_id: None, to_name: Some(to_name.to_string()), kind: EdgeKind::Calls, source: Default::default() }
}

#[test]
fn resolve_edges_fills_in_to_id_by_name() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "a.rs")).unwrap();
    db.upsert_symbols(&[sym(1, 1, "caller", "a.rs"), sym(2, 1, "callee", "a.rs")]).unwrap();
    db.upsert_edges(&[unresolved(1, "callee")]).unwrap();

    let resolved_count = db.resolve_edges().unwrap();
    assert_eq!(resolved_count, 1);

    let edges = db.edges_between(&[1, 2]).unwrap();
    assert!(edges.iter().any(|e| e.from_id == 1 && e.to_id == Some(2)));
}

/// Plan 21 item 3: `resolve_edges` must never resolve a `Calls` edge to a
/// symbol in a different language — measured on erpnext before this fix,
/// 23% of resolved cross-file `Calls` edges were cross-language nonsense
/// (e.g. a Python call resolving to a same-named JS symbol). Here, a Python
/// caller's unresolved call to `"helper"` must resolve to the Python
/// `helper` even though a same-named JS `helper` has far higher in-degree —
/// same-language eligibility is checked *before* the in-degree tie-break,
/// not as a tie-break input.
#[test]
fn resolve_edges_never_crosses_a_language_boundary() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "a.py")).unwrap();
    db.upsert_file(&file(2, "b.js")).unwrap();
    db.upsert_file(&file(3, "c.py")).unwrap();

    let mut py_helper = sym(1, 1, "helper", "a.py");
    py_helper.language = Language::Python;
    let mut js_helper = sym(2, 2, "helper", "b.js");
    js_helper.language = Language::JavaScript;
    let mut py_caller = sym(3, 3, "caller", "c.py");
    py_caller.language = Language::Python;
    // Callers that only exist to give the JS helper a much higher in-degree
    // than the Python one, so a same-language-blind resolver would pick it.
    let mut js_caller_1 = sym(4, 2, "js_caller_1", "b.js");
    js_caller_1.language = Language::JavaScript;
    let mut js_caller_2 = sym(5, 2, "js_caller_2", "b.js");
    js_caller_2.language = Language::JavaScript;

    db.upsert_symbols(&[py_helper, js_helper, py_caller, js_caller_1, js_caller_2]).unwrap();
    db.upsert_edges(&[calls(4, 2), calls(5, 2)]).unwrap(); // js_helper in-degree = 2
    db.upsert_edges(&[unresolved(3, "helper")]).unwrap();  // py_caller -> "helper", ambiguous

    db.resolve_edges().unwrap();

    let edges = db.edges_between(&[3, 1, 2]).unwrap();
    assert!(
        edges.iter().any(|e| e.from_id == 3 && e.to_id == Some(1)),
        "Python caller must resolve to the Python helper despite its lower in-degree"
    );
    assert!(
        !edges.iter().any(|e| e.from_id == 3 && e.to_id == Some(2)),
        "Python caller must never resolve to the higher-in-degree JS helper"
    );
}

/// Regression test for a real large-repo fix (see
/// `bench-reports/large-repo-indexing-2026-08-04.md`): a real
/// `apple/swift` full-index run showed `resolve_edges()` scaling far worse
/// than linearly with corpus size (2+ hours and 81.6GB of accumulated
/// compressed memory on a corpus only ~20x larger than a subset that
/// finished the whole pipeline in 23 seconds) — root-caused to a `GROUP BY`
/// aggregation re-run once per unresolved edge inside a correlated
/// subquery instead of being computed once. This still asserts the fix's
/// original point (materializing in-degree once, not per-edge) doesn't
/// regress performance — see `resolve_edges_scales_past_a_few_thousand_edges_quickly`
/// below for the actual timing guard.
///
/// **Plan 21 item 10 changed the ambiguity behavior this test used to
/// assert**: blindly picking the highest-in-degree candidate for a
/// genuinely ambiguous name was the exact mechanism that let one keycloak
/// `getId()` overload absorb 4,270 calls belonging to 1,004 other methods.
/// With no import signal to disambiguate, `resolve_edges` now leaves the
/// edge unresolved (a visible gap) instead of guessing (a silent lie) — see
/// `resolve_edges_import_signal_disambiguates_over_in_degree` below for the
/// case where a real import signal *is* present and used correctly.
#[test]
fn resolve_edges_ambiguous_name_with_no_import_signal_stays_unresolved() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "a.rs")).unwrap();
    db.upsert_file(&file(2, "b.rs")).unwrap();
    // Two distinct symbols named "helper" in different files/qualified paths.
    db.upsert_symbols(&[
        sym(1, 1, "helper", "a.rs"),
        sym(2, 2, "helper", "b.rs"),
        sym(3, 1, "other_caller_1", "a.rs"),
        sym(4, 1, "other_caller_2", "a.rs"),
        sym(5, 1, "other_caller_3", "a.rs"),
    ]).unwrap();
    // Give symbol 2 ("b.rs::helper") a higher resolved in-degree than symbol 1
    // — this must NOT be enough to resolve the ambiguity below, since there's
    // no import signal pointing at either candidate.
    db.upsert_edges(&[
        calls(3, 2),
        calls(4, 2),
        calls(5, 2),
        calls(3, 1),
    ]).unwrap();
    db.upsert_edges(&[unresolved(3, "helper")]).unwrap();

    let resolved_count = db.resolve_edges().unwrap();
    assert_eq!(resolved_count, 0, "an ambiguous name with no import signal must not be resolved");

    let edges = db.edges_between(&[3, 1, 2]).unwrap();
    let ambiguous = edges.iter().filter(|e| e.from_id == 3 && e.to_name.as_deref() == Some("helper"));
    assert!(
        ambiguous.clone().all(|e| e.to_id.is_none()),
        "expected the ambiguous 'helper' edge to stay unresolved rather than guess by in-degree, got: {:?}",
        ambiguous.collect::<Vec<_>>(),
    );
}

fn import_edge(from: u64, to_name: &str) -> Edge {
    Edge { from_id: from, to_id: None, to_name: Some(to_name.to_string()), kind: EdgeKind::Imports, source: Default::default() }
}

/// Plan 21 item 10: when a genuinely ambiguous name (same name, same
/// language, multiple candidates) has exactly one candidate whose enclosing
/// type/file matches something the caller's file imports, that import
/// signal wins over a blind in-degree guess — even when the *other*,
/// non-imported candidate has a much higher in-degree. This is the
/// resolution the previous test's in-degree-based version couldn't tell
/// apart from a real fix, since it never provided any import edges at all.
#[test]
fn resolve_edges_import_signal_disambiguates_over_in_degree() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "caller.rs")).unwrap();
    db.upsert_file(&file(2, "target_a.rs")).unwrap();
    db.upsert_file(&file(3, "target_b.rs")).unwrap();

    db.upsert_symbols(&[
        sym(10, 1, "do_call", "caller.rs"),
        sym(20, 2, "helper", "target_a.rs"), // NOT imported, but higher in-degree
        sym(21, 3, "helper", "target_b.rs"), // imported by caller.rs, lower in-degree
        // Decoy callers to inflate target_a::helper's in-degree. `edges.from_id`
        // has a real `REFERENCES symbols(id)` foreign key, so these need to be
        // actual symbol rows, not just arbitrary ids.
        sym(100, 1, "decoy_caller_1", "caller.rs"),
        sym(101, 1, "decoy_caller_2", "caller.rs"),
        sym(102, 1, "decoy_caller_3", "caller.rs"),
    ]).unwrap();

    // Inflate target_a::helper's in-degree well above target_b::helper's —
    // in-degree is counted directly off the `edges` table.
    db.upsert_edges(&[calls(100, 20), calls(101, 20), calls(102, 20)]).unwrap();

    // caller.rs imports "target_b" — the only signal that should decide this.
    db.upsert_edges(&[import_edge(10, "target_b")]).unwrap();
    db.upsert_edges(&[unresolved(10, "helper")]).unwrap();

    let resolved_count = db.resolve_edges().unwrap();
    assert_eq!(resolved_count, 1);

    let edges = db.edges_between(&[10, 20, 21]).unwrap();
    let ambiguous = edges.iter().filter(|e| e.from_id == 10 && e.to_name.as_deref() == Some("helper"));
    assert!(
        ambiguous.clone().any(|e| e.to_id == Some(21)),
        "expected the import-matched target_b::helper (id 21) to win despite lower in-degree, got: {:?}",
        ambiguous.collect::<Vec<_>>(),
    );
    assert!(
        !edges.iter().any(|e| e.from_id == 10 && e.to_id == Some(20)),
        "must not fall back to the higher-in-degree, non-imported candidate"
    );
}

/// Performance regression guard: with the pre-fix query, this many
/// unresolved edges against this many total edges would take far longer
/// than a test suite should tolerate (the O(unresolved × total) pattern
/// scales quadratically). A generous wall-clock bound here fails loudly if
/// the quadratic pattern is ever reintroduced, without needing a real
/// multi-hour large-repo run to notice.
#[test]
fn resolve_edges_scales_past_a_few_thousand_edges_quickly() {
    let db = GraphDb::open_in_memory().unwrap();
    const N: u64 = 3000;

    db.upsert_file(&file(1, "big.rs")).unwrap();
    let symbols: Vec<Symbol> = (0..N).map(|i| sym(i, 1, &format!("fn_{i}"), "big.rs")).collect();
    db.upsert_symbols(&symbols).unwrap();

    // A resolved edge into every symbol (so the in-degree aggregation has N
    // rows to group), plus one unresolved edge per symbol needing a name
    // lookup — the exact shape that made the old query O(N * N).
    let mut edges: Vec<Edge> = Vec::with_capacity((N * 2) as usize);
    for i in 0..N {
        edges.push(calls((i + 1) % N, i));
        edges.push(unresolved((i + 1) % N, &format!("fn_{i}")));
    }
    db.upsert_edges(&edges).unwrap();

    let start = std::time::Instant::now();
    let resolved_count = db.resolve_edges().unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resolved_count as u64, N);
    assert!(
        elapsed.as_secs() < 5,
        "resolve_edges() took {elapsed:?} for {N} unresolved edges against {N} total \
         edges — expected well under 5s; this is the exact shape that was quadratic \
         before the fix (a real ~20x-larger corpus took 2+ hours)",
    );
}

/// Regression test for a code-review finding on the temp-table-based fix
/// above: `resolve_edges()` creates a `CREATE TEMP TABLE` scoped to the
/// long-lived connection `GraphDb` holds for its whole process lifetime, so
/// it must never fail on a later call because an earlier call's temp table
/// wasn't cleaned up (a real risk if a `CREATE INDEX` ever failed between an
/// already-succeeded `CREATE TABLE` and the cleanup `DROP`). Calling it
/// repeatedly on the same `GraphDb`, including with nothing left to resolve
/// on the later calls, must keep succeeding.
#[test]
fn resolve_edges_can_be_called_repeatedly_without_error() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "a.rs")).unwrap();
    db.upsert_symbols(&[sym(1, 1, "caller", "a.rs"), sym(2, 1, "callee", "a.rs")]).unwrap();
    db.upsert_edges(&[unresolved(1, "callee")]).unwrap();

    assert_eq!(db.resolve_edges().unwrap(), 1, "first call should resolve the one pending edge");
    for _ in 0..4 {
        assert_eq!(db.resolve_edges().unwrap(), 0, "nothing left to resolve on subsequent calls");
    }
}

// ── Lexical (FTS5) candidate generation ──────────────────────────────────────

fn chunk(symbol_id: u64, qualified_name: &str, file_path: &str, text: &str) -> IndexedChunk {
    IndexedChunk {
        symbol_id,
        qualified_name: qualified_name.to_string(),
        chunk_text:     text.to_string(),
        file_path:      file_path.to_string(),
        start_line:     0,
        language:       "rust".to_string(),
    }
}

#[test]
fn lexical_search_finds_matching_chunk_text() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_fts_batch(&[
        chunk(1, "a::createTextNode", "a.rs", "fn createTextNode() { /* builds a new text node */ }"),
        chunk(2, "a::deleteEditor", "a.rs", "fn deleteEditor() { /* tears down an editor instance */ }"),
    ]).unwrap();

    let hits = db.lexical_search(&["text".to_string(), "node".to_string()], &[], 10).unwrap();
    assert_eq!(hits.len(), 1, "only the createTextNode chunk mentions text/node");
    assert_eq!(hits[0].0, 1);
    assert!(hits[0].1 > 0.0, "normalized score should be positive for a match");
}

/// `chunk_fts` is a plain FTS5 virtual table, so `symbols.file_id ON DELETE
/// CASCADE` never reached it — a real, pre-existing gap found while removing
/// vector search (Plan 20; `GraphDb::delete_file` now explicitly deletes the
/// matching `chunk_fts` rows in the same transaction). Without this, a
/// deleted file's chunk text would live in the FTS5 index forever, since
/// there's no other retrieval channel left to notice the staleness.
#[test]
fn delete_file_also_removes_fts_rows() {
    let db = GraphDb::open_in_memory().unwrap();
    let f = file(91, "gone.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[sym(910, 91, "willBeDeleted", "gone.rs")]).unwrap();
    db.upsert_fts_batch(&[
        chunk(910, "gone.rs::willBeDeleted", "gone.rs", "fn willBeDeleted() { /* about to go */ }"),
    ]).unwrap();

    assert!(
        !db.lexical_search(&["delet".to_string()], &[], 10).unwrap().is_empty(),
        "sanity check: the FTS row should be findable before delete"
    );

    db.delete_file(91).unwrap();

    assert!(
        db.lexical_search(&["delet".to_string()], &[], 10).unwrap().is_empty(),
        "chunk_fts row must not survive its file's deletion"
    );
}

/// Plan 21 item 5: `delete_symbols_for_file` (the live-watcher's per-file
/// re-index path, used on every edit-and-save, distinct from whole-file
/// `delete_file`) must also clean up `chunk_fts` rows for the symbols it
/// deletes — previously only `delete_file` did this, so editing a file to
/// remove/rename a symbol left that symbol's old FTS row behind forever
/// (it kept matching queries and silently occupied a `top_k` slot, since
/// `enrich()` can no longer find the now-gone `symbols` row for it).
#[test]
fn delete_symbols_for_file_also_removes_fts_rows() {
    let db = GraphDb::open_in_memory().unwrap();
    let f = file(92, "edited.rs");
    db.upsert_file(&f).unwrap();
    db.upsert_symbols(&[sym(920, 92, "willBeRenamed", "edited.rs")]).unwrap();
    db.upsert_fts_batch(&[
        chunk(920, "edited.rs::willBeRenamed", "edited.rs", "fn willBeRenamed() { /* about to go */ }"),
    ]).unwrap();

    // Matches via the decomposed+stemmed *identifier* tokens (`tokenize_name`
    // stems "Renamed" -> "renam"), same mechanism `delete_file_also_removes_fts_rows`
    // above relies on — chunk body text itself isn't stemmed (Plan 21 item 12,
    // not yet fixed), so a body-text-only query term wouldn't match here.
    assert!(
        !db.lexical_search(&["renam".to_string()], &[], 10).unwrap().is_empty(),
        "sanity check: the FTS row should be findable before the symbol is removed"
    );

    // Simulate an edit-and-save that removes this symbol (e.g. it was
    // renamed) — the live-watcher path calls delete_symbols_for_file, then
    // re-inserts whatever symbols the new parse actually found.
    db.delete_symbols_for_file(92).unwrap();

    assert!(
        db.lexical_search(&["renam".to_string()], &[], 10).unwrap().is_empty(),
        "chunk_fts row for a symbol removed by a same-file re-index must not survive"
    );
}

#[test]
fn lexical_search_matches_camelcase_identifier_via_decomposed_tokens() {
    let db = GraphDb::open_in_memory().unwrap();
    // "reset" and "editor" appear NOWHERE as their own words — not in the
    // qualified name's raw text (FTS5's `unicode61` treats `resetEditor` as
    // one opaque token), not in the file path (deliberately abbreviated to
    // "em.rs", not "editor_module.rs" — FTS5 does split on `_` by default,
    // which would otherwise give "editor" its own token there and defeat the
    // isolation this test is going for), not in the body. The only way
    // either query term can match is via `upsert_fts_batch`'s
    // `tokenize_name`-decomposed form of the qualified name — this is the
    // regression test for checklist.md Plan 16's camelCase-tokenization fix.
    db.upsert_fts_batch(&[
        chunk(1, "ed::resetEditor", "src/em.rs", "pub fn clears_state() { wipe() }"),
        chunk(2, "utils::unrelatedThing", "src/misc.rs", "pub fn does_other_stuff() { compute() }"),
    ]).unwrap();

    let by_reset = db.lexical_search(&["reset".to_string()], &[], 10).unwrap();
    assert_eq!(by_reset.len(), 1, "'reset' should only match via resetEditor's decomposed name tokens");
    assert_eq!(by_reset[0].0, 1);

    let by_editor = db.lexical_search(&["editor".to_string()], &[], 10).unwrap();
    assert_eq!(by_editor.len(), 1, "'editor' should only match via resetEditor's decomposed name tokens");
    assert_eq!(by_editor[0].0, 1);
}

/// Regression test for a real bug caught by directly querying a live server
/// during Plan 16's camelCase-tokenization fix (not by unit tests alone):
/// decomposing the *whole* qualified name — including the relative file
/// path — floods the index with generic tokens whenever many files share a
/// camelCase-ish naming prefix (`LexicalEditor.ts`, `LexicalSelection.ts`,
/// `LexicalUtils.ts`, …, all containing "editor"/"selection"/"utils" once
/// decomposed). `upsert_fts_batch` must only decompose the bare identifier —
/// the part after the qualified name's one and only `::` — never the path.
#[test]
fn lexical_search_does_not_pollute_matches_from_decomposing_the_file_path() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_fts_batch(&[
        // The actual target: "editor" only appears via `resetEditor`'s own
        // decomposed name, not anywhere in its (deliberately boring) path.
        chunk(1, "src/em.rs::resetEditor", "src/em.rs", "pub fn clears_state() { wipe() }"),
        // A decoy whose *file path* contains a camelCase word that would
        // decompose to "editor" if the whole qualified name were tokenized —
        // its own identifier and body have nothing to do with "editor" at all.
        chunk(2, "src/LexicalEditor.ts::unrelatedHelper", "src/LexicalEditor.ts", "pub fn does_other_stuff() { compute() }"),
    ]).unwrap();

    let hits = db.lexical_search(&["editor".to_string()], &[], 10).unwrap();
    assert_eq!(hits.len(), 1, "only the symbol whose OWN name decomposes to 'editor' should match — \
        not every symbol that merely lives in a file whose path contains 'Editor'");
    assert_eq!(hits[0].0, 1);
}

#[test]
fn lexical_search_empty_terms_returns_empty() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_fts_batch(&[chunk(1, "a::f", "a.rs", "fn f() {}")]).unwrap();
    assert_eq!(db.lexical_search(&[], &[], 10).unwrap(), vec![]);
}

#[test]
fn lexical_search_no_match_returns_empty() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_fts_batch(&[chunk(1, "a::f", "a.rs", "fn f() { compute() }")]).unwrap();
    let hits = db.lexical_search(&["nonexistentterm".to_string()], &[], 10).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn upsert_fts_batch_is_idempotent_by_symbol_id() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_fts_batch(&[chunk(1, "a::old", "a.rs", "fn old() { legacyBehavior() }")]).unwrap();
    // Re-index the same symbol id with new content — must replace, not duplicate.
    db.upsert_fts_batch(&[chunk(1, "a::renamed", "a.rs", "fn renamed() { newBehavior() }")]).unwrap();

    assert!(db.lexical_search(&["legacybehavior".to_string()], &[], 10).unwrap().is_empty(),
        "stale content must not still be findable after a re-index");
    let hits = db.lexical_search(&["newbehavior".to_string()], &[], 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, 1);
}

#[test]
fn lexical_search_weights_name_over_path_over_body() {
    let db = GraphDb::open_in_memory().unwrap();
    // "router" appears as its own FTS5 token in exactly one column per chunk,
    // isolating each of `lexical_search`'s three per-column `bm25()` weights
    // (qualified_name=3.0, file_path=2.0, chunk_text=1.0). Each name uses
    // "::" (not camelCase) to put "router" on its own — `unicode61` does not
    // split camelCase, so a name like `createResumeRouter` would NOT give
    // "router" its own token; that's a real, separate limitation of the
    // lexical channel, documented in checklist.md Plan 16.
    db.upsert_fts_batch(&[
        chunk(1, "router::resumeHandler", "src/handlers.rs", "fn resumeHandler() { build() }"),   // name match only
        chunk(2, "utils::helperOne",      "src/router/utils.rs", "fn helperOne() { compute() }"), // path match only
        chunk(3, "utils::helperTwo",      "src/misc.rs", "fn helperTwo() { /* not a router at all */ }"), // body match only
    ]).unwrap();

    let hits = db.lexical_search(&["router".to_string()], &[], 10).unwrap();
    let order: Vec<u64> = hits.iter().map(|(id, _)| *id).collect();
    assert_eq!(order, vec![1, 2, 3],
        "qualified_name match (weight 3.0) should outrank file_path match (2.0), which should outrank a chunk_text match (1.0)");
}

#[test]
fn fts_format_migration_resets_existing_files_to_pending() {
    // Re-applying an FTS-format migration to a project that already has files
    // must reset them to the pending sentinel so the next index rebuilds their
    // full chunk text (the old reduced-text backfill is gone).
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    graphrelay_store::migrations::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size) VALUES (1,'a.rs',5,'rust','t',0)",
        [],
    ).unwrap();
    conn.execute("DELETE FROM _migrations WHERE name = '009_fts_acronym_digit_tokenization'", []).unwrap();
    graphrelay_store::migrations::run_migrations(&conn).unwrap();
    let cs: i64 = conn.query_row("SELECT checksum FROM files WHERE id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(cs, 0);
}

#[test]
fn legacy_backfill_flag_is_honoured_once() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    graphrelay_store::migrations::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size) VALUES (1,'a.rs',5,'rust','t',0)",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO index_meta (key, value) VALUES ('fts_backfill_needed','true')
         ON CONFLICT(key) DO UPDATE SET value = 'true'", [],
    ).unwrap();
    graphrelay_store::migrations::run_migrations(&conn).unwrap();
    let cs: i64 = conn.query_row("SELECT checksum FROM files WHERE id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(cs, 0);
    conn.execute("UPDATE files SET checksum = 7", []).unwrap();
    graphrelay_store::migrations::run_migrations(&conn).unwrap();
    let cs: i64 = conn.query_row("SELECT checksum FROM files WHERE id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(cs, 7, "the flag must be cleared after it is honoured once");
}

#[test]
fn lexical_search_scores_stay_in_unit_range_across_multiple_matches() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_fts_batch(&[
        chunk(1, "a::parseConfigFile", "config/parse.rs", "parse config file settings, parse again"),
        chunk(2, "a::loadConfig", "config/load.rs", "load config file once"),
        chunk(3, "a::unrelated", "misc.rs", "totally unrelated logic, no config here"),
    ]).unwrap();

    let hits = db.lexical_search(&["config".to_string(), "file".to_string()], &[], 10).unwrap();
    assert!(hits.len() >= 2, "at least the two config-heavy chunks should match");
    for (_, score) in &hits {
        assert!((0.0..=1.0).contains(score), "normalized lexical score {score} out of [0,1] range");
    }
    // Best match (most term occurrences / name+path overlap) gets exactly 1.0.
    assert_eq!(hits.iter().map(|(_, s)| *s).fold(0f32, f32::max), 1.0);
}

// ── Plan 22 item 3: file-path pinning ────────────────────────────────────────

#[test]
fn resolve_pinned_files_exact_match_takes_precedence_over_suffix() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/src/status.py")).unwrap();

    let matches = db.resolve_pinned_files("/repo/src/status.py").unwrap();
    assert_eq!(matches, vec!["/repo/src/status.py".to_string()]);
}

#[test]
fn resolve_pinned_files_falls_back_to_suffix_match() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/src/billing/status.py")).unwrap();

    let matches = db.resolve_pinned_files("billing/status.py").unwrap();
    assert_eq!(matches, vec!["/repo/src/billing/status.py".to_string()]);
}

/// Regression guard for a real correctness hazard flagged during review: a
/// naive unanchored `LIKE '%' || name` would also match `billing_status.py`
/// when resolving a bare `status.py` mention, corrupting the ambiguity-policy
/// count upstream in `RetrievalPipeline::resolve_path_pins`.
/// `files_by_suffix`'s `%/`-prefixed pattern is already anchored at a path
/// separator — this pins that `resolve_pinned_files` inherits that guarantee.
#[test]
fn resolve_pinned_files_does_not_match_a_filename_that_merely_ends_with_the_same_letters() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/src/billing_status.py")).unwrap();

    let matches = db.resolve_pinned_files("status.py").unwrap();
    assert!(
        matches.is_empty(),
        "'billing_status.py' must not match a bare 'status.py' mention: {matches:?}"
    );
}

#[test]
fn resolve_pinned_files_reports_multiple_matches_for_ambiguity_policy() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/pkg_a/status.py")).unwrap();
    db.upsert_file(&file(2, "/repo/pkg_b/status.py")).unwrap();

    let matches = db.resolve_pinned_files("status.py").unwrap();
    assert_eq!(matches.len(), 2);
}

#[test]
fn resolve_pinned_files_returns_empty_for_a_url_like_string() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "/repo/src/foo.py")).unwrap();

    let matches = db.resolve_pinned_files("https://example.com/foo.py").unwrap();
    assert!(matches.is_empty(), "a URL-like mention must not resolve: {matches:?}");
}

/// `symbols_for_path_pin`'s primary path: FTS-scored by `remaining_terms`
/// restricted to the pinned file's own symbols — a term that also appears in
/// a *different* file's chunk must not leak in, and the matching symbol
/// within the pinned file must outrank a non-matching one in the same file.
#[test]
fn symbols_for_path_pin_scores_by_remaining_terms_restricted_to_the_file() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "billing/status.py")).unwrap();
    db.upsert_file(&file(2, "other/unrelated.py")).unwrap();
    db.upsert_symbols(&[
        sym(10, 1, "computeBillingTotal", "billing/status.py"),
        sym(11, 1, "unrelatedHelper", "billing/status.py"),
        sym(12, 2, "computeBillingTotal", "other/unrelated.py"),
    ]).unwrap();
    db.upsert_fts_batch(&[
        chunk(10, "billing/status.py::computeBillingTotal", "billing/status.py", "fn computeBillingTotal() { total() }"),
        chunk(11, "billing/status.py::unrelatedHelper", "billing/status.py", "fn unrelatedHelper() { noop() }"),
        chunk(12, "other/unrelated.py::computeBillingTotal", "other/unrelated.py", "fn computeBillingTotal() { total() }"),
    ]).unwrap();

    let hits = db.symbols_for_path_pin("billing/status.py", &["total".to_string()], 20).unwrap();
    let ids: Vec<_> = hits.iter().map(|(id, _)| *id).collect();

    assert!(ids.contains(&10), "the matching symbol in the pinned file must be selected: {ids:?}");
    assert!(!ids.contains(&12), "a same-named symbol in a DIFFERENT file must not leak in: {ids:?}");
}

/// Fallback: when no remaining term matches anything in the file (or there
/// are no remaining terms at all — e.g. the query was just the path), fall
/// back to a stable span-order slice with score 0.0, instead of admitting
/// nothing.
#[test]
fn symbols_for_path_pin_falls_back_to_span_order_when_nothing_matches() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "billing/status.py")).unwrap();
    db.upsert_symbols(&[
        sym(10, 1, "secondFn", "billing/status.py"),
        sym(11, 1, "firstFn", "billing/status.py"),
    ]).unwrap();
    db.upsert_fts_batch(&[
        chunk(10, "billing/status.py::secondFn", "billing/status.py", "fn secondFn() {}"),
        chunk(11, "billing/status.py::firstFn", "billing/status.py", "fn firstFn() {}"),
    ]).unwrap();

    // No remaining terms at all — the query was just the path itself.
    let hits = db.symbols_for_path_pin("billing/status.py", &[], 20).unwrap();
    assert_eq!(hits.len(), 2, "fallback must still admit the file's symbols: {hits:?}");
    for (_, score) in &hits {
        assert_eq!(*score, 0.0, "fallback-admitted symbols carry no FTS score");
    }

    // A remaining term that matches nothing anywhere in the corpus — same
    // fallback behavior as no terms at all.
    let hits2 = db.symbols_for_path_pin("billing/status.py", &["zzz_no_match_zzz".to_string()], 20).unwrap();
    assert_eq!(hits2.len(), 2, "fallback must fire when no remaining term matches anything: {hits2:?}");
}

/// Per-file cap: a file with more symbols than `max_symbols` must be
/// truncated, not returned in full.
#[test]
fn symbols_for_path_pin_respects_the_per_file_cap() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(1, "big.py")).unwrap();
    let syms: Vec<_> = (0..10u64).map(|i| sym(100 + i, 1, &format!("fn{i}"), "big.py")).collect();
    db.upsert_symbols(&syms).unwrap();
    let chunks: Vec<_> = (0..10u64)
        .map(|i| chunk(100 + i, &format!("big.py::fn{i}"), "big.py", &format!("fn fn{i}() {{}}")))
        .collect();
    db.upsert_fts_batch(&chunks).unwrap();

    // No remaining terms -> span-order fallback path; cap must still apply.
    let hits = db.symbols_for_path_pin("big.py", &[], 3).unwrap();
    assert_eq!(hits.len(), 3, "fallback path must respect the cap");
}

#[test]
fn symbols_for_path_pin_returns_empty_for_an_unindexed_file() {
    let db = GraphDb::open_in_memory().unwrap();
    let hits = db.symbols_for_path_pin("nope.py", &["anything".to_string()], 20).unwrap();
    assert!(hits.is_empty());
}

// ── Plan 24 Phase 1: migration 010 ───────────────────────────────────────────

mod migration_010 {
    use rusqlite::Connection;

    const SQL: &str = include_str!("../src/migrations/010_scip_heal_and_dedupe.sql");

    fn seeded() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        graphrelay_store::migrations::run_migrations(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO files (id, path, checksum, language, indexed_at, byte_size)
                 VALUES (1, 'a.py', 42, 'python', 't', 0);
             INSERT INTO symbols (id, file_id, name, qualified_name, kind, start_byte, end_byte,
                                  start_line, end_line, language, scip_symbol)
                 VALUES (1, 1, 'f', 'a.py::f', 'Function', 0, 1, 0, 1, 'python', 'local 3'),
                        (2, 1, 'g', 'a.py::g', 'Function', 0, 1, 2, 3, 'python', 'pkg/g().');
             INSERT INTO edges (from_id, to_id, to_name, kind, weight, source)
                 VALUES (1, 2, 'g', 'Calls', 1.0, 'semantic'),
                        (1, 2, 'g', 'Calls', 1.0, 'syntax'),
                        (1, 2, 'g', 'Calls', 1.0, 'syntax');
             INSERT INTO index_meta (key, value) VALUES ('scip_ingested_at', 'x');",
        ).unwrap();
        conn
    }

    #[test]
    fn heals_scip_layer_only_when_scip_was_ingested() {
        let conn = seeded();
        conn.execute_batch(SQL).unwrap();
        let count = |q: &str| -> i64 { conn.query_row(q, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT checksum FROM files WHERE id = 1"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM edges WHERE source = 'semantic'"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM edges WHERE source = 'syntax'"), 2,
            "syntax edges (incl. legitimate per-call-site repeats) must be untouched");
        assert_eq!(count("SELECT COUNT(*) FROM symbols WHERE scip_symbol LIKE 'local %'"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM symbols WHERE scip_symbol = 'pkg/g().'"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM index_meta WHERE key = 'scip_ingested_at'"), 0);
    }

    #[test]
    fn no_scip_ingest_means_files_are_not_reset() {
        let conn = seeded();
        conn.execute("DELETE FROM index_meta WHERE key = 'scip_ingested_at'", []).unwrap();
        conn.execute_batch(SQL).unwrap();
        let checksum: i64 = conn.query_row("SELECT checksum FROM files WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(checksum, 42);
    }

    #[test]
    fn semantic_uniqueness_is_enforced_including_null_to_id() {
        let conn = seeded();
        conn.execute_batch(SQL).unwrap();
        for _ in 0..2 {
            conn.execute(
                "INSERT OR IGNORE INTO edges (from_id, to_id, to_name, kind, weight, source)
                 VALUES (1, NULL, 'x', 'Calls', 1.0, 'semantic')", []).unwrap();
        }
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edges WHERE source = 'semantic'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "NULL to_id must not defeat the unique index");
    }
}

// ── Plan 24 Phase 2: atomic per-file replace, token-guarded commits, hygiene ──

mod plan24_indexing {
    use graphrelay_core::types::{
        Edge, EdgeKind, EdgeSource, FileRecord, IndexedChunk, Language, Span, Symbol, SymbolKind,
    };
    use graphrelay_store::{ChunkCommit, GraphDb};
    use std::path::PathBuf;

    fn file(id: u64, path: &str, checksum: u64) -> FileRecord {
        FileRecord { id, path: path.into(), checksum, language: Language::Rust,
                     indexed_at: "2026-01-01T00:00:00Z".into(), byte_size: 0 }
    }
    fn sym(id: u64, file_id: u64, path: &str, name: &str, start: u32) -> Symbol {
        Symbol {
            id, file_id, name: name.into(), qualified_name: format!("{path}::{name}"),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 10, start_line: start, end_line: start + 1 },
            docstring: None, language: Language::Rust, file_path: path.into(),
        }
    }
    fn edge(from: u64, to: Option<u64>, name: &str) -> Edge {
        Edge { from_id: from, to_id: to, to_name: Some(name.into()), kind: EdgeKind::Calls, source: EdgeSource::Syntax }
    }
    fn chunk(id: u64, qn: &str, text: &str) -> IndexedChunk {
        IndexedChunk { symbol_id: id, qualified_name: qn.into(), chunk_text: text.into(),
                       file_path: "f".into(), start_line: 0, language: String::new() }
    }
    fn fts_hits(db: &GraphDb, term: &str) -> Vec<u64> {
        db.lexical_search(&[term.to_string()], &[], 10).unwrap().into_iter().map(|h| h.0).collect()
    }

    #[test]
    fn replace_keeps_surviving_symbols_edges_and_scip_data_and_drops_stale() {
        let db = GraphDb::open_in_memory().unwrap();
        db.upsert_file(&file(2, "b.rs", 1)).unwrap();
        db.upsert_symbols(&[sym(20, 2, "b.rs", "caller", 0)]).unwrap();
        db.replace_file_graph(
            &file(1, "a.rs", 0),
            &[sym(10, 1, "a.rs", "keep", 0), sym(11, 1, "a.rs", "gone", 5)],
            &[],
        ).unwrap();
        db.upsert_edges(&[edge(20, Some(10), "keep"), edge(20, Some(11), "gone")]).unwrap();
        db.upsert_fts_batch(&[chunk(10, "a.rs::keep", "alpha"), chunk(11, "a.rs::gone", "alpha")]).unwrap();
        // SCIP enrichment on the surviving symbol.
        let conn_meta = graphrelay_store::ScipEnrichment {
            file_id: 1, line: 0, name: "keep".into(), scip_symbol: "pkg/keep().".into(),
            kind: Some(SymbolKind::Method), docstring: None,
        };
        db.apply_scip_merge(&[conn_meta], &[]).unwrap();

        db.replace_file_graph(&file(1, "a.rs", 0), &[sym(10, 1, "a.rs", "keep", 0)], &[]).unwrap();

        assert!(db.symbol_by_id(10).unwrap().is_some());
        assert!(db.symbol_by_id(11).unwrap().is_none(), "removed symbol must be deleted");
        assert_eq!(fts_hits(&db, "alpha"), vec![10], "removed symbol's FTS row must go with it");
        assert_eq!(db.callers_of(10).unwrap().len(), 1, "incoming edge to a surviving symbol must keep its to_id");
        assert_eq!(db.get_meta_for_symbol(10).unwrap().as_deref(), Some("pkg/keep()."), "SCIP id preserved");
        assert_eq!(db.symbol_by_id(10).unwrap().unwrap().kind, SymbolKind::Method, "SCIP kind preserved");
    }

    #[test]
    fn replace_rewrites_outgoing_syntax_edges_without_duplicating() {
        let db = GraphDb::open_in_memory().unwrap();
        let edges = vec![edge(10, None, "helper")];
        for _ in 0..3 {
            db.replace_file_graph(&file(1, "a.rs", 0), &[sym(10, 1, "a.rs", "f", 0)], &edges).unwrap();
        }
        let n = db.edge_count_from(10).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn stale_token_commit_is_rejected_and_current_token_commits_atomically() {
        let db = GraphDb::open_in_memory().unwrap();
        let s = [sym(10, 1, "a.rs", "f", 0)];
        let t1 = db.replace_file_graph(&file(1, "a.rs", 0), &s, &[]).unwrap();
        let t2 = db.replace_file_graph(&file(1, "a.rs", 0), &s, &[]).unwrap();
        assert_ne!(t1, t2);
        let path = PathBuf::from("a.rs");

        let n = db.commit_chunk_batch(&[ChunkCommit {
            path: path.clone(), token: t1, checksum: 111, chunks: vec![chunk(10, "a.rs::f", "stalebody")],
        }]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(db.file_checksum(&path).unwrap(), Some(0), "stale builder must not commit a checksum");
        assert!(fts_hits(&db, "stalebody").is_empty(), "stale builder must not write FTS rows");

        let n = db.commit_chunk_batch(&[ChunkCommit {
            path: path.clone(), token: t2, checksum: 222, chunks: vec![chunk(10, "a.rs::f", "freshbody")],
        }]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(db.file_checksum(&path).unwrap(), Some(222));
        assert_eq!(fts_hits(&db, "freshbody"), vec![10]);

        let n = db.commit_chunk_batch(&[ChunkCommit {
            path, token: t2, checksum: 333, chunks: vec![],
        }]).unwrap();
        assert_eq!(n, 0, "a token is single-use once committed");
    }

    #[test]
    fn upsert_batch_returns_one_token_per_file() {
        let db = GraphDb::open_in_memory().unwrap();
        let tokens = db.upsert_batch(&[
            (file(1, "a.rs", 0), vec![sym(10, 1, "a.rs", "f", 0)], vec![]),
            (file(2, "b.rs", 0), vec![sym(20, 2, "b.rs", "g", 0)], vec![]),
        ]).unwrap();
        assert_eq!(tokens.len(), 2);
        assert_ne!(tokens[0], tokens[1]);
    }

    #[test]
    fn gc_removes_only_orphaned_fts_rows() {
        let db = GraphDb::open_in_memory().unwrap();
        db.replace_file_graph(&file(1, "a.rs", 0), &[sym(10, 1, "a.rs", "f", 0)], &[]).unwrap();
        db.upsert_fts_batch(&[chunk(10, "a.rs::f", "beta"), chunk(999, "ghost::g", "beta")]).unwrap();
        assert_eq!(db.gc_orphan_fts().unwrap(), 1);
        assert_eq!(fts_hits(&db, "beta"), vec![10]);
    }

    #[test]
    fn delete_files_under_removes_a_whole_directory() {
        let db = GraphDb::open_in_memory().unwrap();
        db.replace_file_graph(&file(1, "/r/dir/a.rs", 1), &[sym(10, 1, "/r/dir/a.rs", "f", 0)], &[]).unwrap();
        db.replace_file_graph(&file(2, "/r/dir/sub/b.rs", 1), &[sym(20, 2, "/r/dir/sub/b.rs", "g", 0)], &[]).unwrap();
        db.replace_file_graph(&file(3, "/r/dirx/c.rs", 1), &[sym(30, 3, "/r/dirx/c.rs", "h", 0)], &[]).unwrap();
        db.upsert_fts_batch(&[chunk(10, "a::f", "gamma"), chunk(20, "b::g", "gamma"), chunk(30, "c::h", "gamma")]).unwrap();
        let removed = db.delete_files_under(std::path::Path::new("/r/dir")).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(db.symbol_by_id(30).unwrap().is_some(), "a sibling with a shared name prefix must survive");
        assert_eq!(fts_hits(&db, "gamma"), vec![30]);
    }

    fn corpus(n: usize) -> GraphDb {
        let db = GraphDb::open_in_memory().unwrap();
        let chunks: Vec<IndexedChunk> = (0..n as u64)
            .map(|i| {
                let text = if i == 0 { "commonword rareword".to_string() } else { "commonword filler".to_string() };
                chunk(i + 1, &format!("m{i}::f"), &text)
            })
            .collect();
        db.upsert_fts_batch(&chunks).unwrap();
        db
    }

    #[test]
    fn very_common_terms_are_skipped_in_a_large_index() {
        let db = corpus(1500);
        let terms = ["commonword".to_string(), "rareword".to_string()];
        let hits = db.lexical_search(&terms, &[], 10).unwrap();
        assert_eq!(hits.len(), 1, "commonword is in ~100% of rows and must not widen the match set");
        assert_eq!(hits[0].0, 1);
        // A lone term is never dropped: there would be nothing left to match on.
        assert_eq!(db.lexical_search(&["commonword".to_string()], &[], 10).unwrap().len(), 10);
    }

    #[test]
    fn small_indexes_never_skip_terms() {
        let db = corpus(50);
        let terms = ["commonword".to_string(), "rareword".to_string()];
        assert_eq!(db.lexical_search(&terms, &[], 10).unwrap().len(), 10);
    }

    #[test]
    fn scoped_resolution_matches_full_resolution() {
        fn build() -> GraphDb {
            let db = GraphDb::open_in_memory().unwrap();
            db.upsert_batch(&[
                (file(1, "/r/a.rs", 0), vec![sym(10, 1, "/r/a.rs", "caller", 0)],
                    vec![edge(10, None, "target"), edge(10, None, "unknown")]),
                (file(2, "/r/b.rs", 0), vec![sym(20, 2, "/r/b.rs", "target", 0)], vec![]),
                (file(3, "/r/c.rs", 0), vec![sym(30, 3, "/r/c.rs", "other", 0)], vec![edge(30, None, "target")]),
            ]).unwrap();
            db
        }
        let full = build();
        let n_full = full.resolve_edges().unwrap();
        let scoped = build();
        // b.rs changed: callers elsewhere (a.rs, c.rs) that name `target` must resolve
        // even though neither of their files is in scope.
        let n_scoped = scoped.resolve_edges_for_files(&[2]).unwrap();
        assert_eq!(n_full, n_scoped);
        assert_eq!(full.callers_of(20).unwrap().len(), scoped.callers_of(20).unwrap().len());
        assert_eq!(scoped.callers_of(20).unwrap().len(), 2);
        // Scoping to an unrelated file leaves them untouched.
        let other = build();
        assert_eq!(other.resolve_edges_for_files(&[999]).unwrap(), 0);
    }
}

#[test]
fn files_in_prefix_treats_like_wildcards_literally() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(60, "/repo/sales_order/a.py")).unwrap();
    db.upsert_file(&file(61, "/repo/salesXorder/b.py")).unwrap();
    db.upsert_file(&file(62, "/repo/Src/c.py")).unwrap();

    let hits: Vec<u64> = db.files_in_prefix("sales_order").unwrap().into_iter().map(|f| f.id).collect();
    assert_eq!(hits, vec![60], "`_` must not match any character");
    assert!(db.files_in_prefix("%").unwrap().is_empty(), "`%` must not match the whole index");
    assert!(db.files_in_prefix("/").unwrap().is_empty());
    assert!(db.files_in_prefix("src").unwrap().is_empty(), "segment match is case-sensitive");
}

#[test]
fn delete_files_under_is_case_sensitive() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(70, "/repo/src/Foo/a.rs")).unwrap();
    db.upsert_file(&file(71, "/repo/src/foo/b.rs")).unwrap();
    db.upsert_file(&file(72, "/repo/src/Foo.rs")).unwrap();

    let removed = db.delete_files_under(std::path::Path::new("/repo/src/Foo")).unwrap();
    assert_eq!(removed, vec![PathBuf::from("/repo/src/Foo/a.rs")]);
    assert!(db.file_by_path(std::path::Path::new("/repo/src/foo/b.rs")).unwrap().is_some());
    assert!(db.file_by_path(std::path::Path::new("/repo/src/Foo.rs")).unwrap().is_some());
}

#[test]
fn impact_graph_reports_the_parent_each_node_was_reached_through() {
    let db = GraphDb::open_in_memory().unwrap();
    db.upsert_file(&file(80, "chain.rs")).unwrap();
    // root <- direct <- far ; root <- other_direct ; far also calls other_direct.
    db.upsert_symbols(&[
        sym(800, 80, "root", "chain.rs"),
        sym(801, 80, "direct", "chain.rs"),
        sym(802, 80, "far", "chain.rs"),
        sym(803, 80, "other_direct", "chain.rs"),
    ]).unwrap();
    db.upsert_edges(&[calls(801, 800), calls(803, 800), calls(802, 801), calls(802, 803)]).unwrap();

    let impact = db.impact_graph(800, 2).unwrap();
    let by_name = |n: &str| impact.affected.iter().find(|a| a.symbol.name == n).unwrap();
    assert_eq!(by_name("direct").via, None, "hop 1 hangs off the root itself");
    assert_eq!(by_name("other_direct").via, None);
    let far = by_name("far");
    assert_eq!(far.hop_depth, 2);
    // Reached through the lower-id parent, deterministically.
    assert_eq!(far.via.as_deref(), Some("chain.rs::direct"));
}
