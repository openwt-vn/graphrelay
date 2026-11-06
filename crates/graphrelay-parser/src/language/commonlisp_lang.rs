use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct CommonLispLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[SymbolQuery {
    pattern: "(defun . (defun_header function_name: (sym_lit) @name)) @def",
    name_capture: "name",
    def_capture: "def",
    kind: SymbolKind::Function,
}];

impl LanguageSupport for CommonLispLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        // `(foo a b)` — the first element of any list is the call target.
        // Necessarily approximate: this also matches every list literal
        // that happens to start with a symbol (a common Lisp idiom for
        // data, not just calls) — Lisp's uniform syntax makes "is this a
        // call" undecidable without evaluating macros, so this is the same
        // best-effort tradeoff this codebase already accepts elsewhere for
        // dynamic/macro-heavy call shapes.
        "(list_lit . value: (sym_lit) @call.name) @call.site"
    }
    fn import_query(&self) -> &'static str {
        // No single query shape covers `require`/`in-package`/`defpackage`
        // cleanly (Lisp's package system is itself macro-driven, and the
        // grammar's `value:` field repeats per list element rather than
        // naming a second argument) — left unsupported.
        ""
    }
    fn file_extensions(&self) -> &[&'static str] { &["lisp", "cl", "lsp", "asd"] }
    fn language_kind(&self) -> Language { Language::CommonLisp }
}
