//! Tests for Vue/Svelte Single-File-Component script-block delegation:
//! `.vue`/`.svelte` files delegate their
//! `<script>` block to the existing TypeScript/JavaScript `LanguageSupport`
//! impls rather than requiring a new tree-sitter grammar, then offset the
//! resulting symbol spans back into the original file's coordinates. The
//! offset math is the single most likely source of bugs in this kind of
//! delegation, so these tests specifically verify spans, not just presence.

use graphrelay_core::types::{EdgeKind, Language, SymbolKind};
use graphrelay_parser::{Extractor, LanguageRegistry};
use std::path::Path;

fn ex() -> Extractor {
    Extractor::new(LanguageRegistry::with_defaults())
}

const VUE_SFC: &str = r#"<template>
  <div>{{ greeting }}</div>
</template>

<script setup lang="ts">
function greet(name: string): string {
    return hello(name);
}

function hello(name: string): string {
    return "Hello " + name;
}
</script>

<style scoped>
div { color: red; }
</style>
"#;

#[test]
fn vue_extracts_functions_with_spans_matching_original_file() {
    let result = ex().extract(Path::new("Greeter.vue"), VUE_SFC.as_bytes()).unwrap();

    assert_eq!(result.symbols.len(), 2, "expected exactly 2 functions, got: {:?}", result.symbols);
    assert!(result.symbols.iter().all(|s| s.kind == SymbolKind::Function));

    // Delegated symbols are tagged with the *embedded* language (see
    // `try_sfc_delegate`'s doc comment) — TypeScript here since `lang="ts"`.
    assert!(
        result.symbols.iter().all(|s| s.language == Language::TypeScript),
        "delegated .vue symbols should be tagged TypeScript, got: {:?}",
        result.symbols.iter().map(|s| s.language).collect::<Vec<_>>(),
    );

    // The critical check: each symbol's reported span, sliced out of the
    // *original* .vue file (not the extracted script fragment), must equal
    // that function's real source text. A wrong offset would slice garbage
    // or the wrong lines entirely.
    for sym in &result.symbols {
        let start = sym.span.start_byte as usize;
        let end = sym.span.end_byte as usize;
        let sliced = &VUE_SFC[start..end];
        assert!(
            sliced.starts_with("function") && sliced.contains(&sym.name),
            "span for {:?} sliced out of the original file as {:?} — offset math is wrong",
            sym.name, sliced,
        );
    }

    let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
    let hello = result.symbols.iter().find(|s| s.name == "hello").unwrap();
    // "function greet" is the 6th line of VUE_SFC (1-indexed) -> 0-indexed line 5.
    assert_eq!(greet.span.start_line, 5, "greet() should be on line 5 (0-indexed) of the original file");
    assert!(hello.span.start_line > greet.span.start_line, "hello() should come after greet() in the file");
}

#[test]
fn vue_call_edge_resolves_across_delegated_functions() {
    let result = ex().extract(Path::new("Greeter.vue"), VUE_SFC.as_bytes()).unwrap();
    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Calls),
        "expected a Calls edge from greet() to hello(), got: {:?}", result.edges,
    );
}

#[test]
fn vue_file_with_no_script_block_produces_zero_symbols_without_erroring() {
    let template_only = "<template>\n  <div>static content</div>\n</template>\n";
    let result = ex().extract(Path::new("Static.vue"), template_only.as_bytes()).unwrap();
    assert!(result.symbols.is_empty());
    assert!(result.edges.is_empty());
}

const SVELTE_SFC: &str = r#"<script>
function double(x) {
    return x * 2;
}
</script>

<div>{double(2)}</div>
"#;

#[test]
fn svelte_extracts_function_with_correct_span_and_javascript_language() {
    let result = ex().extract(Path::new("Doubler.svelte"), SVELTE_SFC.as_bytes()).unwrap();
    assert_eq!(result.symbols.len(), 1);
    let sym = &result.symbols[0];
    assert_eq!(sym.name, "double");
    assert_eq!(sym.language, Language::JavaScript, "plain <script> (no lang=\"ts\") should delegate to JavaScript");

    let start = sym.span.start_byte as usize;
    let end = sym.span.end_byte as usize;
    assert_eq!(&SVELTE_SFC[start..end], "function double(x) {\n    return x * 2;\n}");
}

#[test]
fn vue_and_svelte_are_watched_by_default() {
    // Regression guard for the exact drift bug this session's Initiative 3
    // work found and fixed (see language_config_sync.rs in graphrelay-watcher):
    // every registered language must appear in default_languages().
    let cfg = graphrelay_core::Config::default();
    assert!(cfg.index.languages.iter().any(|l| l == "vue"));
    assert!(cfg.index.languages.iter().any(|l| l == "svelte"));
}
