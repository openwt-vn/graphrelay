use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct PowerShellLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[SymbolQuery {
    pattern: "(function_statement (function_name) @name) @def",
    name_capture: "name",
    def_capture: "def",
    kind: SymbolKind::Function,
}];

impl LanguageSupport for PowerShellLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_powershell::language() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(command command_name: (command_name) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        // No single query shape reliably captures `Import-Module`'s
        // argument across its positional/named-parameter forms — left
        // unsupported rather than shipping a query that only sometimes
        // matches.
        ""
    }
    fn file_extensions(&self) -> &[&'static str] { &["ps1", "psm1"] }
    fn language_kind(&self) -> Language { Language::PowerShell }
}
