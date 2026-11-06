use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct FortranLang;

// `@def` is anchored to the outer `function`/`subroutine`/`module` wrapper
// node, not the `*_statement` header line alone — `extract_edges` resolves
// a call site to its enclosing symbol by byte-range containment, so a
// header-only span (which ends before the body even starts) would make
// every call inside the body resolve to no enclosing symbol at all.
const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function (function_statement name: (name) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(subroutine (subroutine_statement name: (name) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(module (module_statement . (name) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for FortranLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_fortran::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(subroutine_call subroutine: (identifier) @call.name) @call.site
(call_expression function: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(use_statement (module_name) @import.path) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] {
        &["f90", "f95", "f03", "f08", "f", "F90", "F95", "F03", "F08", "F"]
    }
    fn language_kind(&self) -> Language { Language::Fortran }
}
