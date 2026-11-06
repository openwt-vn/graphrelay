use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct JuliaLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: r#"(function_definition (signature (call_expression . (identifier) @name))) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(struct_definition (type_head . (identifier) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: "(module_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for JuliaLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_julia::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call_expression . (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import_statement (selected_import (identifier) @import.path)) @import.decl
(using_statement (identifier) @import.path) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["jl"] }
    fn language_kind(&self) -> Language { Language::Julia }
}
