use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct ObjcLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(class_interface . (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    // The method's selector identifier immediately follows `method_type`
    // with nothing else between them — anchoring to that adjacency (rather
    // than "first child", which is `method_type` itself) is what picks the
    // selector out instead of a later parameter's own identifier.
    SymbolQuery {
        pattern: "(method_definition (method_type) . (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
];

impl LanguageSupport for ObjcLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_objc::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(message_expression method: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(preproc_include path: (_) @import.path) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["m", "mm"] }
    fn language_kind(&self) -> Language { Language::ObjectiveC }
}
