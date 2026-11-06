use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct ZigLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // `const Foo = struct { ... };` — the struct itself carries no name;
    // it's the enclosing `const` binding's identifier (positional, no
    // field — anchored to the first child) that names the type.
    SymbolQuery {
        pattern: "(variable_declaration . (identifier) @name (struct_declaration)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
];

impl LanguageSupport for ZigLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_zig::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call_expression function: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(builtin_function
            (builtin_identifier) @import.fn
            (#eq? @import.fn "@import")
            (arguments (string_literal) @import.path)) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["zig"] }
    fn language_kind(&self) -> Language { Language::Zig }
}
