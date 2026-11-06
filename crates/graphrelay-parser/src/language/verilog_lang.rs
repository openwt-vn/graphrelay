use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct VerilogLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[SymbolQuery {
    pattern: "(module_declaration (module_header (module_keyword) . (simple_identifier) @name)) @def",
    name_capture: "name",
    def_capture: "def",
    kind: SymbolKind::Module,
}];

impl LanguageSupport for VerilogLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_verilog::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        // Module instantiation (`bar baz_inst(...)` inside `module foo`) is
        // the closest Verilog concept to a "call" — the structural
        // dependency this codebase's graph expansion is built around.
        //
        // This grammar's `X Y(...)` instantiation syntax is genuinely
        // ambiguous without a symbol table (real Verilog parsers resolve it
        // semantically) — it disambiguates as `module_instantiation` only
        // when the instantiating module's declaration comes *before* the
        // instantiated one in the same file; the reverse order parses as
        // `checker_instantiation`/`udp_instantiation` instead and this
        // query won't match it. A real, source-order-dependent miss, not a
        // query bug — verified directly against the built grammar.
        "(module_instantiation . (simple_identifier) @call.name) @call.site"
    }
    fn import_query(&self) -> &'static str {
        // Verilog has no module-graph-relevant import — cross-file
        // dependencies go through `` `include `` preprocessor directives,
        // which are textual, not semantic.
        ""
    }
    fn file_extensions(&self) -> &[&'static str] { &["v", "sv", "svh"] }
    fn language_kind(&self) -> Language { Language::Verilog }
}
