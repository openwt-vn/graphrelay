use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct BashLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[SymbolQuery {
    pattern: "(function_definition name: (word) @name) @def",
    name_capture: "name",
    def_capture: "def",
    kind: SymbolKind::Function,
}];

impl LanguageSupport for BashLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_bash::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(command name: (command_name (word) @call.name)) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(command
            name: (command_name (word) @import.fn)
            (#eq? @import.fn "source")
            argument: (word) @import.path) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["sh", "bash"] }
    fn language_kind(&self) -> Language { Language::Bash }
}
