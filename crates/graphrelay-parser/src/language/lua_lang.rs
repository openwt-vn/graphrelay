use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct LuaLang;

// Lua's grammar aliases `local function foo() end` onto the same visible
// `function_declaration` node type as `function foo() end` (see grammar.js:
// `alias($._local_function_declaration, $.function_declaration)`), so one
// query covers both forms whenever the name is a plain identifier (as
// opposed to `function t.foo()` / `function t:foo()`, whose `name` field is
// a `dot_index_expression`/`method_index_expression` rather than a bare
// `identifier` — not covered by this query, matching this codebase's
// existing per-language scoping of "simple" declaration forms).
const SYMBOL_QUERIES: &[SymbolQuery] = &[SymbolQuery {
    pattern: "(function_declaration name: (identifier) @name) @def",
    name_capture: "name",
    def_capture: "def",
    kind: SymbolKind::Function,
}];

impl LanguageSupport for LuaLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_lua::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(function_call name: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(function_call
            name: (identifier) @import.fn
            (#eq? @import.fn "require")
            arguments: (arguments (string) @import.path)) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["lua"] }
    fn language_kind(&self) -> Language { Language::Lua }
}
