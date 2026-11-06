use graphrelay_core::types::{EdgeKind, SymbolKind};
use graphrelay_parser::{Extractor, LanguageRegistry};
use std::path::Path;

fn extractor() -> Extractor {
    Extractor::new(LanguageRegistry::default())
}

// ── Rust ─────────────────────────────────────────────────────────────────────

#[test]
fn rust_extracts_function() {
    let src = b"pub fn greet(name: &str) -> String { format!(\"Hello {}\", name) }";
    let result = extractor().extract(Path::new("sample.rs"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "greet").expect("greet not found");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert_eq!(sym.language, graphrelay_core::types::Language::Rust);
}

#[test]
fn rust_extracts_struct() {
    let src = b"pub struct Foo { pub x: i32 }";
    let result = extractor().extract(Path::new("sample.rs"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Foo").expect("Foo not found");
    assert_eq!(sym.kind, SymbolKind::Struct);
}

#[test]
fn rust_extracts_enum() {
    let src = b"pub enum Color { Red, Green, Blue }";
    let result = extractor().extract(Path::new("sample.rs"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Color").expect("Color not found");
    assert_eq!(sym.kind, SymbolKind::Enum);
}

#[test]
fn rust_extracts_call_edge() {
    let src = b"fn foo() {}\nfn bar() { foo(); }";
    let result = extractor().extract(Path::new("sample.rs"), src).unwrap();
    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Calls),
        "expected at least one Calls edge"
    );
}

#[test]
fn rust_symbol_span_is_valid() {
    let src = b"fn alpha() {}\nfn beta() {}";
    let result = extractor().extract(Path::new("span.rs"), src).unwrap();
    for sym in &result.symbols {
        assert!(sym.span.start_line <= sym.span.end_line, "inverted span for {}", sym.name);
        assert!(sym.span.start_byte <= sym.span.end_byte, "inverted bytes for {}", sym.name);
    }
}

#[test]
fn rust_qualified_name_contains_path() {
    let src = b"fn my_fn() {}";
    let result = extractor().extract(Path::new("src/lib.rs"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "my_fn").unwrap();
    assert!(
        sym.qualified_name.contains("src/lib.rs"),
        "qualified_name should contain file path, got: {}",
        sym.qualified_name
    );
}

// ── Python ───────────────────────────────────────────────────────────────────

#[test]
fn python_extracts_function() {
    let src = b"def hello(name):\n    return f'Hello {name}'\n";
    let result = extractor().extract(Path::new("sample.py"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "hello").expect("hello not found");
    assert_eq!(sym.kind, SymbolKind::Function);
}

#[test]
fn python_extracts_class() {
    let src = b"class Greeter:\n    def greet(self):\n        pass\n";
    let result = extractor().extract(Path::new("sample.py"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Greeter").expect("Greeter not found");
    assert_eq!(sym.kind, SymbolKind::Class);
}

#[test]
fn python_extracts_docstring() {
    let src = b"def described():\n    \"\"\"This is a docstring.\"\"\"\n    pass\n";
    let result = extractor().extract(Path::new("doc.py"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "described").unwrap();
    assert!(sym.docstring.is_some(), "expected docstring to be extracted");
}

// ── TypeScript ───────────────────────────────────────────────────────────────

#[test]
fn typescript_extracts_function() {
    let src = b"function add(a: number, b: number): number { return a + b; }";
    let result = extractor().extract(Path::new("sample.ts"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "add").expect("add not found");
    assert_eq!(sym.kind, SymbolKind::Function);
}

#[test]
fn typescript_extracts_interface() {
    let src = b"interface Animal { name: string; sound(): string; }";
    let result = extractor().extract(Path::new("sample.ts"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Animal");
    assert!(sym.is_some(), "expected Animal interface to be extracted");
}

/// Plan 21 item 2: a call site must be attributed to the NARROWEST
/// containing symbol, not the first one found — `extract_edges` previously
/// used `.find()` over `symbols` (ordered class-before-method for
/// TypeScript/JavaScript), so every call inside a class method was credited
/// to the enclosing class instead of the method itself. Measured on
/// keycloak: 140 TS/JS class methods had zero outgoing Calls edges before
/// this fix, because every call inside them was mis-attributed this way.
#[test]
fn typescript_class_method_call_attributed_to_method_not_enclosing_class() {
    let src = br#"
function helper() {}
class Widget {
    render() {
        helper();
    }
}
"#;
    let result = extractor().extract(Path::new("widget.ts"), src).unwrap();
    let method = result.symbols.iter().find(|s| s.name == "render").expect("render method not found");
    let class = result.symbols.iter().find(|s| s.name == "Widget").expect("Widget class not found");
    let call_edge = result.edges.iter().find(|e| e.kind == EdgeKind::Calls)
        .expect("expected a Calls edge for helper()");
    assert_eq!(
        call_edge.from_id, method.id,
        "a call inside a method must be attributed to the method itself"
    );
    assert_ne!(
        call_edge.from_id, class.id,
        "a call inside a method must NOT be attributed to its enclosing class"
    );
}

// ── JavaScript: object-literal colon-form methods (Plan 18, Finding 3) ────────
// `{ key(){} }` shorthand was already covered by `method_definition`; these
// cover the previously-unindexed `{ key: function(){} }` / `{ key: () => {} }`
// forms, motivated by ERPNext's frappe.ui.form.on("Doctype", {...}) pattern.

#[test]
fn javascript_extracts_object_literal_colon_function_method() {
    // The actual shape from erpnext/stock/doctype/item/item.js — a
    // frappe.ui.form.on(...) handler block using `key: function(frm) {}`.
    let src = br#"
frappe.ui.form.on("Item", {
    render_item_prices: function (frm) {
        if (!frappe.model.can_read("Item Price")) {
            frm.toggle_display("prices_html", false);
        }
    },
});
"#;
    let result = extractor().extract(Path::new("item.js"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "render_item_prices")
        .expect("render_item_prices not found");
    assert_eq!(sym.kind, SymbolKind::Method);
}

#[test]
fn javascript_extracts_object_literal_arrow_function_method() {
    let src = b"const handlers = { onClick: (event) => { doSomething(event); } };";
    let result = extractor().extract(Path::new("handlers.js"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "onClick")
        .expect("onClick not found");
    assert_eq!(sym.kind, SymbolKind::Method);
}

#[test]
fn javascript_object_literal_shorthand_method_still_works() {
    // Regression guard: the ES6 shorthand form must keep working via the
    // pre-existing method_definition pattern, unaffected by the new patterns.
    let src = b"const obj = { refresh(frm) { console.log(frm); } };";
    let result = extractor().extract(Path::new("shorthand.js"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "refresh");
    assert!(sym.is_some(), "shorthand method 'refresh' should still be extracted");
}

#[test]
fn javascript_object_literal_plain_data_pair_is_not_a_symbol() {
    // False-positive guard: a `{ key: "value" }` / `{ key: 42 }` pair must
    // NOT become a symbol — the new query patterns only match a function
    // value, not any pair.
    let src = b"const config = { title: \"Item\", max_count: 42, enabled: true };";
    let result = extractor().extract(Path::new("config.js"), src).unwrap();
    for name in ["title", "max_count", "enabled"] {
        assert!(
            !result.symbols.iter().any(|s| s.name == name),
            "'{name}' is a plain data field and must not be extracted as a symbol"
        );
    }
}

#[test]
fn javascript_object_literal_identifier_reference_is_not_a_new_symbol() {
    // False-positive guard: `{ key: someFn }` is a *reference* to an
    // existing function, not a new definition — must not create a second
    // symbol for `handler`.
    let src = b"function someFn() {}\nconst obj = { handler: someFn };";
    let result = extractor().extract(Path::new("ref.js"), src).unwrap();
    assert!(
        !result.symbols.iter().any(|s| s.name == "handler"),
        "an identifier-reference pair value must not be extracted as a symbol"
    );
    assert!(result.symbols.iter().any(|s| s.name == "someFn"), "someFn itself should still be extracted");
}

// ── TypeScript: object-literal colon-form methods (Plan 18, Finding 3) ────────

#[test]
fn typescript_extracts_object_literal_colon_function_method() {
    let src = b"const handlers = { onSave: function (doc) { return doc; } };";
    let result = extractor().extract(Path::new("handlers.ts"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "onSave").expect("onSave not found");
    assert_eq!(sym.kind, SymbolKind::Method);
}

#[test]
fn typescript_object_literal_plain_data_pair_is_not_a_symbol() {
    let src = b"const config = { title: \"Item\", maxCount: 42 };";
    let result = extractor().extract(Path::new("config.ts"), src).unwrap();
    assert!(!result.symbols.iter().any(|s| s.name == "title" || s.name == "maxCount"));
}

// ── Go ───────────────────────────────────────────────────────────────────────

#[test]
fn go_extracts_function() {
    let src = b"package main\n\nfunc Hello() string { return \"hello\" }\n";
    let result = extractor().extract(Path::new("main.go"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Hello").expect("Hello not found");
    assert_eq!(sym.kind, SymbolKind::Function);
}

/// Go methods are top-level declarations, so the receiver type is the only
/// "enclosing type". Without it every `New` in a file was `path::New` (or
/// `path::New#2`), and `trace_impact` could not say which factory it meant.
#[test]
fn go_methods_are_qualified_by_their_receiver_type() {
    let src = b"package migrations\n\
type GiteaFactory struct{}\n\
type GogsFactory struct{}\n\
type Set[K comparable] struct{}\n\
func (f *GiteaFactory) New() int { return 1 }\n\
func (f GogsFactory) New() int { return 2 }\n\
func (GogsFactory) Close() {}\n\
func (s *Set[K]) Add(k K) {}\n\
func New() int { return 0 }\n";
    let result = extractor().extract(Path::new("services/factory.go"), src).unwrap();
    let names: Vec<&str> = result.symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    for expected in [
        "services/factory.go::GiteaFactory::New",
        "services/factory.go::GogsFactory::New",
        "services/factory.go::GogsFactory::Close",
        "services/factory.go::Set::Add",
        "services/factory.go::New",
        "services/factory.go::GiteaFactory",
    ] {
        assert!(names.contains(&expected), "missing {expected}; got {names:?}");
    }
    assert!(!names.iter().any(|n| n.contains('#')), "no overload suffix needed once receivers differ: {names:?}");
}

/// Same-named methods on the same receiver in one file still disambiguate.
#[test]
fn go_receiver_qualified_names_stay_unique_and_stable() {
    let src = b"package p\ntype T struct{}\nfunc (t T) M() {}\n";
    let a = extractor().extract(Path::new("p/t.go"), src).unwrap();
    let b = extractor().extract(Path::new("p/t.go"), src).unwrap();
    let id = |r: &graphrelay_core::types::FileExtraction| r.symbols.iter().find(|s| s.name == "M").unwrap().id;
    assert_eq!(id(&a), id(&b));
    let ids: std::collections::HashSet<_> = a.symbols.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), a.symbols.len());
}

// ── Java ─────────────────────────────────────────────────────────────────────

#[test]
fn java_extracts_class() {
    let src = b"public class Greeter { public void greet() {} }";
    let result = extractor().extract(Path::new("Greeter.java"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Greeter").expect("Greeter not found");
    assert_eq!(sym.kind, SymbolKind::Class);
}

#[test]
fn java_extracts_method() {
    let src = b"public class Foo { public void bar() {} }";
    let result = extractor().extract(Path::new("Foo.java"), src).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "bar"), "bar method not found");
}

// ── C ────────────────────────────────────────────────────────────────────────

#[test]
fn c_extracts_function() {
    let src = b"int add(int a, int b) { return a + b; }";
    let result = extractor().extract(Path::new("math.c"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "add").expect("add not found");
    assert_eq!(sym.kind, SymbolKind::Function);
}

#[test]
fn c_extracts_struct() {
    let src = b"struct Point { int x; int y; };";
    let result = extractor().extract(Path::new("geo.c"), src).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "Point"), "Point struct not found");
}

// ── C++ ──────────────────────────────────────────────────────────────────────

#[test]
fn cpp_extracts_class() {
    let src = b"class Vector { public: int size(); };";
    let result = extractor().extract(Path::new("vec.cpp"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Vector").expect("Vector not found");
    assert_eq!(sym.kind, SymbolKind::Class);
}

// ── C# ───────────────────────────────────────────────────────────────────────

#[test]
fn csharp_extracts_class() {
    let src = b"public class Greeter { public void Greet() {} }";
    let result = extractor().extract(Path::new("Greeter.cs"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "Greeter").expect("Greeter not found");
    assert_eq!(sym.kind, SymbolKind::Class);
}

// ── Ruby ─────────────────────────────────────────────────────────────────────

#[test]
fn ruby_extracts_method() {
    let src = b"class Greeter\n  def hello\n    puts 'hi'\n  end\nend\n";
    let result = extractor().extract(Path::new("greeter.rb"), src).unwrap();
    assert!(result.symbols.iter().any(|s| s.name == "hello"), "hello method not found");
    assert!(result.symbols.iter().any(|s| s.name == "Greeter"), "Greeter class not found");
}

// ── Kotlin ───────────────────────────────────────────────────────────────────

#[test]
#[ignore = "Kotlin grammar disabled: no crates.io release compatible with tree-sitter 0.24"]
fn kotlin_extracts_function() {
    let src = b"fun greet(name: String): String = \"Hello $name\"";
    let result = extractor().extract(Path::new("greet.kt"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "greet").expect("greet not found");
    assert_eq!(sym.kind, SymbolKind::Function);
}

// ── Swift ────────────────────────────────────────────────────────────────────

#[test]
fn swift_extracts_function() {
    let src = b"func greet(name: String) -> String { return \"Hello \\(name)\" }";
    let result = extractor().extract(Path::new("greet.swift"), src).unwrap();
    let sym = result.symbols.iter().find(|s| s.name == "greet").expect("greet not found");
    assert_eq!(sym.kind, SymbolKind::Function);
}

// ── Incremental ──────────────────────────────────────────────────────────────

#[test]
fn incremental_parse_produces_same_symbols_as_full() {
    let src = b"fn alpha() {}\nfn beta() {}";
    let path = Path::new("inc.rs");

    let ex = extractor();
    let incremental = ex.extract_incremental(path, src).unwrap();
    let full = ex.extract(path, src).unwrap();

    let mut inc_names: Vec<_> = incremental.symbols.iter().map(|s| &s.name).collect();
    let mut full_names: Vec<_> = full.symbols.iter().map(|s| &s.name).collect();
    inc_names.sort();
    full_names.sort();
    assert_eq!(inc_names, full_names, "incremental and full must find the same symbols");
}

#[test]
fn incremental_reuse_updates_cache() {
    let src_v1 = b"fn foo() {}";
    let src_v2 = b"fn foo() {}\nfn bar() {}";
    let path = Path::new("cache.rs");

    let ex = extractor();
    ex.extract_incremental(path, src_v1).unwrap();
    let result = ex.extract_incremental(path, src_v2).unwrap();

    assert!(result.symbols.iter().any(|s| s.name == "bar"), "bar should appear after re-parse");
}

/// The real point of incremental parsing: an append-only edit (the most
/// common shape of a single save while actively coding) must produce
/// results identical to parsing the final content from scratch, for a
/// large-enough file that the incremental path actually engages
/// (tree-sitter's fast path can still choose a full re-parse for very small
/// inputs; this covers correctness either way).
#[test]
fn incremental_append_matches_full_reparse() {
    let mut src_v1 = String::new();
    for i in 0..200 {
        src_v1.push_str(&format!("fn func_{i}() {{ helper_{i}(); }}\n"));
    }
    let mut src_v2 = src_v1.clone();
    src_v2.push_str("fn appended_at_end() { func_0(); }\n");
    let path = Path::new("append.rs");

    let ex = extractor();
    ex.extract_incremental(path, src_v1.as_bytes()).unwrap();
    let incremental = ex.extract_incremental(path, src_v2.as_bytes()).unwrap();
    let full = ex.extract(path, src_v2.as_bytes()).unwrap();

    let mut inc_names: Vec<_> = incremental.symbols.iter().map(|s| s.name.clone()).collect();
    let mut full_names: Vec<_> = full.symbols.iter().map(|s| s.name.clone()).collect();
    inc_names.sort();
    full_names.sort();
    assert_eq!(inc_names, full_names);
    assert!(inc_names.iter().any(|n| n == "appended_at_end"));
}

/// A mid-file edit (insert a new function between two existing ones) — the
/// shape where getting the byte/position math wrong would most plausibly
/// show up as a wrong or missing symbol.
#[test]
fn incremental_mid_file_insert_matches_full_reparse() {
    let src_v1 = "fn one() {}\nfn two() {}\nfn three() {}\n";
    let src_v2 = "fn one() {}\nfn inserted() { two(); }\nfn two() {}\nfn three() {}\n";
    let path = Path::new("mid_insert.rs");

    let ex = extractor();
    ex.extract_incremental(path, src_v1.as_bytes()).unwrap();
    let incremental = ex.extract_incremental(path, src_v2.as_bytes()).unwrap();
    let full = ex.extract(path, src_v2.as_bytes()).unwrap();

    let mut inc_names: Vec<_> = incremental.symbols.iter().map(|s| s.name.clone()).collect();
    let mut full_names: Vec<_> = full.symbols.iter().map(|s| s.name.clone()).collect();
    inc_names.sort();
    full_names.sort();
    assert_eq!(inc_names, full_names);
    assert!(
        incremental.edges.iter().any(|e| e.kind == EdgeKind::Calls),
        "sanity: call edges still extractable post-edit"
    );
}

/// A deletion (removing a whole function from the middle) — the same
/// byte-math risk as an insert, opposite direction.
#[test]
fn incremental_deletion_matches_full_reparse() {
    let src_v1 = "fn one() {}\nfn two() {}\nfn three() {}\n";
    let src_v2 = "fn one() {}\nfn three() {}\n";
    let path = Path::new("deletion.rs");

    let ex = extractor();
    ex.extract_incremental(path, src_v1.as_bytes()).unwrap();
    let incremental = ex.extract_incremental(path, src_v2.as_bytes()).unwrap();
    let full = ex.extract(path, src_v2.as_bytes()).unwrap();

    let mut inc_names: Vec<_> = incremental.symbols.iter().map(|s| s.name.clone()).collect();
    let mut full_names: Vec<_> = full.symbols.iter().map(|s| s.name.clone()).collect();
    inc_names.sort();
    full_names.sort();
    assert_eq!(inc_names, full_names);
    assert!(!inc_names.iter().any(|n| n == "two"));
}

/// Repeated saves of byte-identical content (e.g. a save with no real
/// change) must not panic or corrupt the cached tree.
#[test]
fn incremental_identical_content_is_a_no_op() {
    let src = b"fn stable() {}\n";
    let path = Path::new("stable.rs");

    let ex = extractor();
    let first = ex.extract_incremental(path, src).unwrap();
    let second = ex.extract_incremental(path, src).unwrap();

    assert_eq!(
        first.symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        second.symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
    );
}

/// After `invalidate_incremental_cache`, the next call must behave like a
/// first-ever parse (no stale old-tree diffing against unrelated content) —
/// this is what the watcher calls on file deletion.
#[test]
fn invalidate_incremental_cache_forces_fresh_parse() {
    let src_v1 = b"fn foo() {}";
    let src_v2 = b"fn bar() {}";
    let path = Path::new("invalidate.rs");

    let ex = extractor();
    ex.extract_incremental(path, src_v1).unwrap();
    ex.invalidate_incremental_cache(path);
    let result = ex.extract_incremental(path, src_v2).unwrap();

    let names: Vec<_> = result.symbols.iter().map(|s| s.name.clone()).collect();
    assert!(names.contains(&"bar".to_string()));
    assert!(!names.contains(&"foo".to_string()));
}

// ── Error handling ───────────────────────────────────────────────────────────

#[test]
fn unsupported_extension_returns_error() {
    let result = extractor().extract(Path::new("sample.unknown_ext_xyz"), b"something");
    assert!(result.is_err(), "expected error for unsupported extension");
}

#[test]
fn empty_source_returns_no_symbols() {
    let result = extractor().extract(Path::new("empty.rs"), b"").unwrap();
    assert!(result.symbols.is_empty());
    assert!(result.edges.is_empty());
}


// ── Plan 24 Phase 4: root-relative qualified names ───────────────────────────

#[test]
fn with_root_makes_qualified_names_and_ids_independent_of_checkout_location() {
    let src = b"class Widget:\n    def render(self):\n        return 1\n\ndef helper():\n    return 2\n";
    let a = Extractor::new(LanguageRegistry::with_defaults()).with_root("/home/alice/repo");
    let b = Extractor::new(LanguageRegistry::with_defaults()).with_root("/srv/build/checkout");
    let ra = a.extract(Path::new("/home/alice/repo/pkg/ui.py"), src).unwrap();
    let rb = b.extract(Path::new("/srv/build/checkout/pkg/ui.py"), src).unwrap();

    let names_a: Vec<_> = ra.symbols.iter().map(|s| (s.qualified_name.clone(), s.id)).collect();
    let names_b: Vec<_> = rb.symbols.iter().map(|s| (s.qualified_name.clone(), s.id)).collect();
    assert!(!names_a.is_empty());
    assert_eq!(names_a, names_b, "same file at a different checkout path must yield identical ids");
    assert!(names_a.iter().all(|(q, _)| q.starts_with("pkg/ui.py::")), "got {names_a:?}");
    assert!(names_a.iter().any(|(q, _)| q == "pkg/ui.py::Widget::render"), "got {names_a:?}");
}

#[test]
fn without_a_root_qualified_names_keep_the_path_as_given() {
    let src = b"def helper():\n    return 2\n";
    let r = extractor().extract(Path::new("/tmp/x/mod.py"), src).unwrap();
    assert!(r.symbols.iter().any(|s| s.qualified_name == "/tmp/x/mod.py::helper"));
}

#[test]
fn a_path_outside_the_root_falls_back_to_the_given_path() {
    let src = b"def helper():\n    return 2\n";
    let e = Extractor::new(LanguageRegistry::with_defaults()).with_root("/home/alice/repo");
    let r = e.extract(Path::new("/elsewhere/mod.py"), src).unwrap();
    assert!(r.symbols.iter().any(|s| s.qualified_name == "/elsewhere/mod.py::helper"));
}

#[test]
fn decorated_python_function_is_one_symbol_that_owns_its_calls() {
    let src = b"def inner():\n    pass\n\n@whitelist()\ndef outer():\n    inner()\n";
    let r = extractor().extract(Path::new("m.py"), src).unwrap();
    let outers: Vec<_> = r.symbols.iter().filter(|s| s.name == "outer").collect();
    assert_eq!(outers.len(), 1, "got {:?}", r.symbols.iter().map(|s| &s.qualified_name).collect::<Vec<_>>());
    assert!(!outers[0].qualified_name.contains('#'));
    let outer_id = outers[0].id;
    let inner_id = r.symbols.iter().find(|s| s.name == "inner").unwrap().id;
    assert!(
        r.edges.iter().any(|e| e.from_id == outer_id && e.to_id == Some(inner_id)),
        "the body's call must hang off the one decorated symbol"
    );
}
