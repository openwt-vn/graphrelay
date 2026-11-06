// Correctness tests for `Extractor::extract_many`:
// the parallel full-index parse path must produce results identical to the
// sequential `extract()` path, and must not corrupt results when many files
// of the *same* language are parsed concurrently (the failure mode a shared
// `Mutex<Parser>` would produce under contention).

use graphrelay_core::types::SymbolKind;
use graphrelay_parser::{Extractor, LanguageRegistry};
use std::path::PathBuf;

fn extractor() -> Extractor {
    Extractor::new(LanguageRegistry::default())
}

#[test]
fn extract_many_matches_sequential_extract_per_file() {
    let files: Vec<(PathBuf, Vec<u8>)> = vec![
        (PathBuf::from("a.rs"), b"pub fn alpha() {}".to_vec()),
        (PathBuf::from("b.py"), b"def beta():\n    pass\n".to_vec()),
        (PathBuf::from("c.ts"), b"function gamma(): void {}".to_vec()),
        (PathBuf::from("d.go"), b"package main\nfunc delta() {}\n".to_vec()),
    ];

    let ext = extractor();
    let parallel_results = ext.extract_many(&files);
    assert_eq!(parallel_results.len(), files.len());

    for (path, source) in &files {
        let sequential = ext.extract(path, source).expect("sequential extract failed");
        let (parallel_path, parallel_result) = parallel_results
            .iter()
            .find(|(p, _)| p == path)
            .expect("extract_many missing a file present in the input");
        assert_eq!(parallel_path, path);
        let parallel = parallel_result
            .as_ref()
            .unwrap_or_else(|e| panic!("extract_many failed for {path:?}: {e}"));

        assert_eq!(
            parallel.symbols.len(), sequential.symbols.len(),
            "symbol count mismatch for {path:?}",
        );
        let mut seq_names: Vec<&str> = sequential.symbols.iter().map(|s| s.name.as_str()).collect();
        let mut par_names: Vec<&str> = parallel.symbols.iter().map(|s| s.name.as_str()).collect();
        seq_names.sort_unstable();
        par_names.sort_unstable();
        assert_eq!(par_names, seq_names, "symbol names mismatch for {path:?}");

        assert_eq!(
            parallel.edges.len(), sequential.edges.len(),
            "edge count mismatch for {path:?}",
        );
        assert_eq!(
            parallel.file_record.checksum, sequential.file_record.checksum,
            "checksum mismatch for {path:?}",
        );
    }
}

#[test]
fn extract_many_handles_unsupported_language_without_failing_the_batch() {
    let files: Vec<(PathBuf, Vec<u8>)> = vec![
        (PathBuf::from("ok.rs"), b"pub fn works() {}".to_vec()),
        (PathBuf::from("nope.unsupported_ext"), b"garbage".to_vec()),
    ];
    let results = extractor().extract_many(&files);
    assert_eq!(results.len(), 2);

    let ok = results.iter().find(|(p, _)| p.ends_with("ok.rs")).unwrap();
    assert!(ok.1.is_ok(), "supported file should still succeed");

    let bad = results.iter().find(|(p, _)| p.ends_with("nope.unsupported_ext")).unwrap();
    assert!(bad.1.is_err(), "unsupported file should error, not panic");
}

/// Many files of the *same* language, each with a distinct function name, run
/// through `extract_many` concurrently. `extract_many` gives each rayon task
/// its own freshly-constructed `tree_sitter::Parser` instead of reusing the
/// shared per-language cache `extract()` uses — this test would catch the
/// classic failure mode of accidentally routing back through a shared,
/// contended parser: wrong symbol landing in the wrong file, or a parse
/// silently producing zero symbols under concurrent load.
#[test]
fn extract_many_same_language_high_concurrency_no_cross_contamination() {
    let n = 200;
    let files: Vec<(PathBuf, Vec<u8>)> = (0..n)
        .map(|i| {
            (
                PathBuf::from(format!("file_{i}.rs")),
                format!("pub fn func_{i}() {{ let _ = {i}; }}").into_bytes(),
            )
        })
        .collect();

    let results = extractor().extract_many(&files);
    assert_eq!(results.len(), n);

    for (i, (path, result)) in results.iter().enumerate() {
        let extraction = result
            .as_ref()
            .unwrap_or_else(|e| panic!("extraction failed for {path:?}: {e}"));
        assert!(
            path.to_string_lossy().contains(&format!("file_{i}.rs")),
            "result order/path mismatch at index {i}: {path:?}",
        );
        let expected_name = format!("func_{i}");
        let sym = extraction
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function)
            .unwrap_or_else(|| panic!("no function symbol found for {path:?}"));
        assert_eq!(
            sym.name, expected_name,
            "wrong function name extracted for {path:?} — possible cross-file contamination",
        );
    }
}

#[test]
fn extract_many_empty_input_returns_empty() {
    let results = extractor().extract_many(&[]);
    assert!(results.is_empty());
}
