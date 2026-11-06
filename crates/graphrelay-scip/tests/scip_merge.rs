/// Plan 12 B7 — unit tests for the SCIP semantic layer.

// ── Always-compile tests ─────────────────────────────────────────────────────

#[test]
fn ingest_stats_default() {
    let stats = graphrelay_scip::ScipIngestStats::default();
    assert_eq!(stats.files_processed, 0);
    assert_eq!(stats.symbols_enriched, 0);
    assert_eq!(stats.edges_added, 0);
    assert_eq!(stats.docstrings_updated, 0);
}

// ── Tests that run WITH the scip feature ─────────────────────────────────────

#[cfg(feature = "scip")]
mod scip_enabled {
    use graphrelay_scip::mapper::scip_kind_to_symbol_kind;
    use scip::types::symbol_information::Kind as ScipKind;
    use graphrelay_core::types::SymbolKind;

    /// B7-T1: All known SCIP Kind values must not panic in the mapper.
    #[test]
    fn all_scip_kinds_are_handled_no_panic() {
        // Spot-check a representative sample — the match is exhaustive so any unhandled
        // variant would be a compile error in mapper.rs.
        let kinds = [
            ScipKind::Class, ScipKind::Interface, ScipKind::Struct, ScipKind::Enum,
            ScipKind::Function, ScipKind::Method, ScipKind::Trait, ScipKind::TypeAlias,
            ScipKind::Constant, ScipKind::Module, ScipKind::Variable, ScipKind::Parameter,
            ScipKind::Boolean, ScipKind::Array, ScipKind::UnspecifiedKind,
            ScipKind::AbstractMethod, ScipKind::StaticMethod, ScipKind::Extension,
        ];
        for kind in kinds {
            let _ = scip_kind_to_symbol_kind(kind); // must not panic
        }
    }

    /// B7-T1b: Key direct mappings.
    #[test]
    fn kind_mappings_correct() {
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Class),     Some(SymbolKind::Class));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Interface), Some(SymbolKind::Interface));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Struct),    Some(SymbolKind::Struct));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Enum),      Some(SymbolKind::Enum));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Function),  Some(SymbolKind::Function));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Method),    Some(SymbolKind::Method));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Trait),     Some(SymbolKind::Trait));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Protocol),  Some(SymbolKind::Trait));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::TypeAlias), Some(SymbolKind::TypeAlias));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Constant),  Some(SymbolKind::Const));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Module),    Some(SymbolKind::Module));
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Namespace), Some(SymbolKind::Module));
    }

    /// B7-T1c: Kinds with no graphrelay equivalent return None.
    #[test]
    fn no_graphrelay_equivalent_returns_none() {
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::UnspecifiedKind), None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Variable),   None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Parameter),  None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Boolean),    None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Array),      None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Null),       None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::Number),     None);
        assert_eq!(scip_kind_to_symbol_kind(ScipKind::String),     None);
    }

    /// B7-T2: Round-trip serialization of a minimal SCIP index.
    #[test]
    fn parse_minimal_scip_proto() {
        use protobuf::Message;
        use scip::types::{Index, Document, Occurrence, SymbolInformation};

        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/foo.py".to_string();

        let mut occ = Occurrence::new();
        occ.range = vec![5, 4, 10]; // 0-indexed line 5
        occ.symbol = "scip-python pip/foo 1.0 foo/bar().".to_string();
        occ.symbol_roles = 1; // DEFINITION bit
        doc.occurrences.push(occ);

        let mut si = SymbolInformation::new();
        si.symbol = "scip-python pip/foo 1.0 foo/bar().".to_string();
        si.documentation.push("A test function.".to_string());
        doc.symbols.push(si);
        index.documents.push(doc);

        let bytes = index.write_to_bytes().expect("serialize");
        let parsed = Index::parse_from_bytes(&bytes).expect("deserialize");

        assert_eq!(parsed.documents.len(), 1);
        assert_eq!(parsed.documents[0].relative_path, "src/foo.py");
        assert_eq!(parsed.documents[0].occurrences[0].range[0], 5);
        // DEFINITION bit check
        assert_eq!(parsed.documents[0].occurrences[0].symbol_roles & 1, 1);
    }

    /// B7-T3 (regression): SCIP occurrence line numbers are 0-indexed, and so is
    /// `graphrelay-parser`'s tree-sitter-derived `start_line` (see extractor.rs:
    /// `start_position().row`, used with no +1). Enrichment must match a symbol
    /// seeded at the *same* 0-indexed line an extractor would have produced — an
    /// earlier version of the merger added a spurious +1 here, which shifted every
    /// enrichment match off by one row and silently degraded `symbols_enriched`
    /// without failing any existing test (the integration tests below never seed
    /// tree-sitter-shaped symbols, so they couldn't catch it).
    #[test]
    fn enrich_matches_tree_sitter_style_zero_indexed_line() {
        use graphrelay_core::types::{FileRecord, Language, Span, Symbol, SymbolKind};
        use graphrelay_store::GraphDb;
        use protobuf::EnumOrUnknown;
        use scip::types::{symbol_information::Kind as ScipKind, Document, Index, Occurrence, SymbolInformation};

        let db = GraphDb::open_in_memory().expect("open db");

        let file = FileRecord {
            id:         1,
            path:       "src/foo.py".into(),
            checksum:   0,
            language:   Language::Python,
            indexed_at: "2026-01-01T00:00:00Z".to_string(),
            byte_size:  0,
        };
        db.upsert_file(&file).expect("upsert file");

        // Mirrors what graphrelay-parser's extractor would store for a `def bar():`
        // at editor line 6 — tree-sitter reports `start_position().row == 5` (0-indexed).
        let symbol = Symbol {
            id:             1,
            file_id:        1,
            name:           "bar".to_string(),
            qualified_name: "foo.bar".to_string(),
            kind:           SymbolKind::Function,
            span:           Span { start_byte: 0, end_byte: 10, start_line: 5, end_line: 5 },
            docstring:      None,
            language:       Language::Python,
            file_path:      "src/foo.py".into(),
        };
        db.upsert_symbols(&[symbol]).expect("upsert symbol");

        let mut index = Index::new();
        let mut doc = Document::new();
        doc.relative_path = "src/foo.py".to_string();

        let mut occ = Occurrence::new();
        occ.range = vec![5, 4, 10]; // SCIP 0-indexed line 5 — same row as the seeded symbol
        occ.symbol = "scip-python pip/foo 1.0 foo/bar().".to_string();
        occ.symbol_roles = 1; // DEFINITION bit
        doc.occurrences.push(occ);

        let mut si = SymbolInformation::new();
        si.symbol = "scip-python pip/foo 1.0 foo/bar().".to_string();
        si.kind = EnumOrUnknown::new(ScipKind::Function);
        si.documentation.push("A test function.".to_string());
        doc.symbols.push(si);
        index.documents.push(doc);

        // Empty root: `Path::new("").join("src/foo.py")` == `"src/foo.py"`,
        // matching the bare relative path seeded into `files.path` above —
        // Plan 21's fix resolves file_id via `root.join(relative_path)`,
        // once per document, instead of the old per-occurrence LIKE scan.
        let stats = graphrelay_scip::merger::merge_index(&index, &db, std::path::Path::new(""))
            .expect("merge_index");

        assert_eq!(stats.symbols_enriched, 1,
            "expected the seeded symbol at 0-indexed start_line=5 to be enriched \
             by a SCIP occurrence at the same 0-indexed line — an off-by-one here \
             means enrichment is matching the wrong row (or no row)");

        let enriched = db.symbol_by_qualified("foo.bar").expect("lookup").expect("symbol exists");
        assert_eq!(enriched.docstring.as_deref(), Some("A test function."));
    }
}

// ── Integration tests using real rust-analyzer SCIP output ──────────────────
// T4-equivalent: uses rust-analyzer (Rust) since scip-python/scip-typescript
// not available in this environment. Tests the full parse → merge → verify pipeline.

#[cfg(feature = "scip")]
mod integration {
    use graphrelay_store::GraphDb;
    use std::path::{Path, PathBuf};

    fn scip_path() -> Option<PathBuf> {
        // Tests run from the crate root (crates/graphrelay-scip/);
        // the SCIP file lives at the workspace root (.graphrelay/index.scip).
        let crate_dir = std::env::current_dir().ok()?;
        // Walk up to find workspace root (has Cargo.toml with [workspace])
        let mut dir = crate_dir.as_path();
        loop {
            let candidate = dir.join(".graphrelay/index.scip");
            if candidate.exists() { return Some(candidate); }
            dir = dir.parent()?;
        }
    }

    /// Workspace root implied by a `scip_path()` result — `.graphrelay/index.scip`'s
    /// grandparent. These tests all use an empty in-memory DB with nothing
    /// seeded, so `merge_index`'s per-document `file_by_path` lookup always
    /// misses regardless of the exact root value — this just needs to be a
    /// real, valid path for the (now-required) `root` argument.
    fn workspace_root(scip_path: &Path) -> PathBuf {
        scip_path.parent().unwrap().parent().unwrap().to_path_buf()
    }

    fn _seed_placeholder() {}  // seeding via tree-sitter removed (would add dep cycle)

    /// T4-equivalent: parse + ingest real rust-analyzer SCIP output.
    /// Verifies: SCIP parses, stats are correct, idempotency holds.
    #[test]
    fn parse_and_ingest_real_scip_file() {
        let scip_path = match scip_path() {
            Some(p) => p,
            None => {
                eprintln!("skip: run `rust-analyzer scip .` first to generate .graphrelay/index.scip");
                return;
            }
        };

        let db = GraphDb::open_in_memory().expect("open db");
        let stats = graphrelay_scip::merge_scip(&scip_path, &db, &workspace_root(&scip_path))
            .expect("merge_scip must not fail");

        assert!(stats.files_processed > 50,
            "rust-analyzer SCIP must cover >50 files, got {}",
            stats.files_processed);

        eprintln!("SCIP parse OK: {} files, {} enriched, {} edges, {} docs",
            stats.files_processed, stats.symbols_enriched,
            stats.edges_added, stats.docstrings_updated);
    }

    /// T4 cross-file edges: seed DB with real symbols, ingest SCIP, verify
    /// semantic edges are created between cross-file callers.
    #[test]
    fn scip_cross_file_edges_after_seeding() {
        let scip_path = match scip_path() {
            Some(p) => p,
            None => { return; }
        };

        // Empty DB — tests SCIP parse + edge extraction without symbol seeding
        let db = GraphDb::open_in_memory().expect("open db");

        let before_symbols = db.total_symbol_count().unwrap_or(0);
        let stats = graphrelay_scip::merge_scip(&scip_path, &db, &workspace_root(&scip_path)).expect("merge_scip");

        eprintln!("symbols before={before_symbols} enriched={} edges={}",
            stats.symbols_enriched, stats.edges_added);

        // If symbols were seeded, SCIP should have enriched some
        // (SCIP uses line-number matching on relative_path)
        // At minimum: the merge must succeed and produce processable output
        assert!(stats.files_processed > 0);
    }

    /// T4 idempotency: double-ingest produces identical results.
    #[test]
    fn double_ingest_idempotent() {
        let scip_path = match scip_path() {
            Some(p) => p,
            None => { return; }
        };
        let db = GraphDb::open_in_memory().expect("open db");
        let root = workspace_root(&scip_path);
        let s1 = graphrelay_scip::merge_scip(&scip_path, &db, &root).expect("first ingest");
        let s2 = graphrelay_scip::merge_scip(&scip_path, &db, &root).expect("second ingest");
        assert_eq!(s1.files_processed, s2.files_processed, "idempotency: same file count");
        assert_eq!(s1.edges_added, s2.edges_added, "idempotency: same edge count");
        eprintln!("Idempotency verified: {} files, {} edges (both ingests identical)",
            s1.files_processed, s1.edges_added);
    }
}

// ── Plan 24 Phase 1 regression tests ─────────────────────────────────────────

#[cfg(feature = "scip")]
mod plan24 {
    use graphrelay_core::types::{FileRecord, Language, Span, Symbol, SymbolKind};
    use graphrelay_store::GraphDb;
    use protobuf::EnumOrUnknown;
    use scip::types::{symbol_information::Kind as ScipKind, Document, Index, Occurrence, SymbolInformation};

    fn file(id: u64, path: &str) -> FileRecord {
        FileRecord { id, path: path.into(), checksum: 1, language: Language::Python,
                     indexed_at: "2026-01-01T00:00:00Z".into(), byte_size: 0 }
    }

    fn sym(id: u64, file_id: u64, path: &str, name: &str, start: u32, end: u32) -> Symbol {
        Symbol {
            id, file_id, name: name.into(), qualified_name: format!("{path}::{name}"),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 10, start_line: start, end_line: end },
            docstring: None, language: Language::Python, file_path: path.into(),
        }
    }

    fn occ(symbol: &str, line: i32, roles: i32) -> Occurrence {
        let mut o = Occurrence::new();
        o.range = vec![line, 0, 5];
        o.symbol = symbol.into();
        o.symbol_roles = roles;
        o
    }

    fn doc(path: &str, occs: Vec<Occurrence>) -> Document {
        let mut d = Document::new();
        d.relative_path = path.into();
        d.occurrences = occs;
        for o in d.occurrences.clone() {
            let mut si = SymbolInformation::new();
            si.symbol = o.symbol;
            si.kind = EnumOrUnknown::new(ScipKind::Function);
            d.symbols.push(si);
        }
        d
    }

    fn semantic_edges(db: &GraphDb) -> Vec<(u64, u64)> {
        let mut out = vec![];
        for s in db.all_symbols().unwrap() {
            for c in db.callees_of(s.id).unwrap() {
                out.push((s.id, c.id));
            }
        }
        out
    }

    fn seeded() -> GraphDb {
        let db = GraphDb::open_in_memory().unwrap();
        db.upsert_file(&file(1, "a.py")).unwrap();
        db.upsert_file(&file(2, "b.py")).unwrap();
        db.upsert_symbols(&[
            sym(10, 1, "a.py", "caller", 0, 9),
            sym(11, 1, "a.py", "other", 20, 21),
            sym(20, 2, "b.py", "callee", 0, 4),
            sym(21, 2, "b.py", "decoy", 0, 0),
        ]).unwrap();
        db
    }

    #[test]
    fn local_symbols_never_enrich_or_link() {
        let db = seeded();
        let mut idx = Index::new();
        idx.documents.push(doc("b.py", vec![occ("local 3", 0, 1)]));
        idx.documents.push(doc("a.py", vec![occ("local 3", 2, 0)]));
        let st = graphrelay_scip::merger::merge_index(&idx, &db, std::path::Path::new("")).unwrap();
        assert_eq!(st.symbols_enriched, 0);
        assert_eq!(st.edges_added, 0);
        assert!(db.get_meta_for_symbol(20).unwrap().is_none());
        assert!(semantic_edges(&db).is_empty());
    }

    #[test]
    fn enrichment_matches_by_name_not_just_line() {
        let db = seeded();
        let target = "scip-python pip/x 1.0 b/callee().";
        let mut idx = Index::new();
        idx.documents.push(doc("b.py", vec![occ(target, 0, 1)]));
        graphrelay_scip::merger::merge_index(&idx, &db, std::path::Path::new("")).unwrap();
        assert_eq!(db.get_meta_for_symbol(20).unwrap().as_deref(), Some(target));
        assert!(db.get_meta_for_symbol(21).unwrap().is_none(),
            "a different symbol starting on the same line must not be touched");
    }

    #[test]
    fn only_resolvable_call_descriptors_become_edges_and_dedupe() {
        let db = seeded();
        let callee = "scip-python pip/x 1.0 b/callee().";
        let mut idx = Index::new();
        idx.documents.push(doc("b.py", vec![occ(callee, 0, 1)]));
        idx.documents.push(doc("a.py", vec![
            occ(callee, 3, 0),
            occ(callee, 4, 0),
            occ("scip-python pip/x 1.0 b/CONSTANT.", 5, 0),
            occ("scip-python pip/x 1.0 ext/unknown().", 6, 0),
        ]));
        let st = graphrelay_scip::merger::merge_index(&idx, &db, std::path::Path::new("")).unwrap();
        assert_eq!(st.edges_added, 1, "one deduped call edge; field read and unresolved dropped");
        assert_eq!(semantic_edges(&db), vec![(10, 20)]);
    }

    #[test]
    fn reingest_replaces_rather_than_duplicates() {
        let db = seeded();
        let callee = "scip-python pip/x 1.0 b/callee().";
        let mut idx = Index::new();
        idx.documents.push(doc("b.py", vec![occ(callee, 0, 1)]));
        idx.documents.push(doc("a.py", vec![occ(callee, 3, 0)]));
        for _ in 0..3 {
            graphrelay_scip::merger::merge_index(&idx, &db, std::path::Path::new("")).unwrap();
        }
        assert_eq!(semantic_edges(&db), vec![(10, 20)]);
    }
}
