use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct CLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: r#"(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(struct_specifier name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: "(enum_specifier name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    SymbolQuery {
        pattern: "(type_definition declarator: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
];

impl LanguageSupport for CLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_c::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call_expression function: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(preproc_include path: (_) @import.path) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["c", "h"] }
    fn language_kind(&self) -> Language { Language::C }
}
