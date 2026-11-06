use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct RustLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_item name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(struct_item name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: "(enum_item name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    SymbolQuery {
        pattern: "(trait_item name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Trait,
    },
    SymbolQuery {
        pattern: "(type_item name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
    SymbolQuery {
        pattern: "(const_item name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Const,
    },
    SymbolQuery {
        pattern: "(mod_item name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for RustLang {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn symbol_queries(&self) -> &[SymbolQuery] {
        SYMBOL_QUERIES
    }

    fn call_query(&self) -> &'static str {
        r#"(call_expression
  function: [
    (identifier) @call.name
    (scoped_identifier name: (identifier) @call.name)
    (field_expression field: (field_identifier) @call.name)
  ]) @call.site"#
    }

    fn import_query(&self) -> &'static str {
        r#"(use_declaration argument: (_) @import.path) @import.decl"#
    }

    fn references_query(&self) -> &'static str {
        r#"(function_item name: (identifier) @references.from
            return_type: (type_identifier) @references.to)
           (function_item name: (identifier) @references.from
            parameters: (parameters (parameter type: (type_identifier) @references.to)))"#
    }

    fn implements_query(&self) -> &'static str {
        // Rust has no class inheritance — `impl Trait for Type` is a
        // genuine "implements", the correct semantic home for
        // EdgeKind::Implements (previously defined in the schema but
        // populated by no language at all). An inherent `impl Type { }`
        // (no `trait:` field) correctly doesn't match any pattern here.
        // Field order in the pattern must match the grammar's actual child
        // order (`trait:` precedes `type:` here) — see the Java/C#/Groovy
        // Inherits fix for why an out-of-order pattern fails to compile.
        //
        // A bare `Trait` is the minority case in real code — `impl
        // fmt::Display for Foo` (scoped_type_identifier, tail via its
        // `name:` field, present regardless of nesting depth) and `impl
        // From<Bar> for Foo` (generic_type, base name via its `type:`
        // field) are at least as common, so both trait shapes are crossed
        // with both `type:`-side shapes (bare and generic) below.
        r#"(impl_item trait: (type_identifier) @implements.to
            type: (type_identifier) @implements.from)
           (impl_item trait: (type_identifier) @implements.to
            type: (generic_type type: (type_identifier) @implements.from))
           (impl_item trait: (scoped_type_identifier name: (type_identifier) @implements.to)
            type: (type_identifier) @implements.from)
           (impl_item trait: (scoped_type_identifier name: (type_identifier) @implements.to)
            type: (generic_type type: (type_identifier) @implements.from))
           (impl_item trait: (generic_type type: (type_identifier) @implements.to)
            type: (type_identifier) @implements.from)
           (impl_item trait: (generic_type type: (type_identifier) @implements.to)
            type: (generic_type type: (type_identifier) @implements.from))"#
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["rs"]
    }

    fn language_kind(&self) -> Language {
        Language::Rust
    }
}
