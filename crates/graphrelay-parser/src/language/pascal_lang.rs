use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct PascalLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(defProc header: (declProc (kProcedure) name: (identifier) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(defProc header: (declProc (kFunction) name: (identifier) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
];

impl LanguageSupport for PascalLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_pascal::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(exprCall entity: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(declUses (moduleName) @import.path) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] {
        &["pas", "pp", "dpr", "dpk", "lpr", "inc"]
    }
    fn language_kind(&self) -> Language { Language::Pascal }
}
