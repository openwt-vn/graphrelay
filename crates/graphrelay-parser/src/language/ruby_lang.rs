use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct RubyLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(method name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    SymbolQuery {
        pattern: "(singleton_method name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    SymbolQuery {
        pattern: "(class name: (constant) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(module name: (constant) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for RubyLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_ruby::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call method: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(call method: (identifier) @import.path
    (#match? @import.path "^require")) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        r#"(class name: (constant) @inherits.from
            superclass: (superclass (constant) @inherits.to))
           (class name: (constant) @inherits.from
            superclass: (superclass (scope_resolution name: (constant) @inherits.to)))"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["rb"] }
    fn language_kind(&self) -> Language { Language::Ruby }
}
