use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct KotlinLang;

// Kotlin's grammar represents `class`, `interface`, and `enum class` all as
// the same `class_declaration` node — `interface`/`enum` are literal
// keyword children, not distinct node types (see grammar.js: `choice('class',
// seq(optional('fun'), 'interface'))`). The generic `class_declaration`
// query below is listed first (kind: Class) and the `"interface"`-anchored
// one after it, so the extractor's "last match wins" per-id dedup (see
// `Extractor::extract_symbols`) lets the more specific pattern override the
// generic one for interfaces only.
const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(class_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: r#"(class_declaration "interface" name: (identifier) @name) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Interface,
    },
    SymbolQuery {
        pattern: "(object_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
    SymbolQuery {
        pattern: "(enum_entry (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Const,
    },
];

impl LanguageSupport for KotlinLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_kotlin_ng::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call_expression (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import (identifier) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // `class Foo : Bar(), Baz` — delegation_specifier wraps a
        // constructor_invocation for the superclass call, or a bare
        // user_type for implemented interfaces; both forms are needed.
        // A qualified base (`: ns.Bar()`) puts 2 unfielded `identifier`
        // children directly under `user_type`; the trailing `.` anchors the
        // capture to the *last* child so it resolves to the real class name
        // in both the 1-child (bare) and 2-child (qualified) cases without
        // a separate pattern also matching the leading `ns` segment.
        r#"(class_declaration name: (identifier) @inherits.from
            (delegation_specifiers (delegation_specifier (constructor_invocation (user_type (identifier) @inherits.to .)))))
           (class_declaration name: (identifier) @inherits.from
            (delegation_specifiers (delegation_specifier (user_type (identifier) @inherits.to .))))"#
    }
    fn references_query(&self) -> &'static str {
        // Both the return type and each parameter's `user_type` are
        // unfielded children in this grammar — the differing nesting depth
        // (direct child of function_declaration for the return type, vs.
        // nested inside function_value_parameters for a parameter) keeps
        // the two patterns from matching each other's type.
        r#"(function_declaration name: (identifier) @references.from
            (user_type (identifier) @references.to))
           (function_declaration name: (identifier) @references.from
            (function_value_parameters (parameter (identifier) (user_type (identifier) @references.to))))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["kt", "kts"] }
    fn language_kind(&self) -> Language { Language::Kotlin }
}
