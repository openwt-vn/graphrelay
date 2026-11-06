use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

/// Config/markup format, not a "real" programming language — see Plan 19
/// Phase C. Deliberately generic rather than tuned to any one JSON
/// convention (e.g. Frappe/ERPNext doctype schemas, `package.json`,
/// `tsconfig.json`): only a file's *top-level* object keys become symbols.
/// This is bounded (a handful of keys per file, not one per nested field)
/// and stays useful across arbitrary JSON rather than overfitting to a
/// single framework's file shape.
pub struct JsonLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[SymbolQuery {
    // Anchored to `document > object > pair` specifically — matches only
    // pairs that are direct children of the root object, not pairs nested
    // inside arrays/objects at any depth (verified against the real parse
    // tree, not assumed: a nested `{"fieldname": ..., "nested": {"deep":
    // ...}}` under a top-level `fields` array contributes zero symbols).
    pattern: "(document (object (pair key: (string (string_content) @name)) @def))",
    name_capture: "name",
    def_capture: "def",
    kind: SymbolKind::Const,
}];

impl LanguageSupport for JsonLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_json::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        // JSON has no call semantics.
        ""
    }
    fn import_query(&self) -> &'static str {
        // No general-purpose reference/include convention across arbitrary
        // JSON (unlike, say, a `$ref` in JSON Schema specifically) — left
        // unsupported rather than special-casing one dialect.
        ""
    }
    fn file_extensions(&self) -> &[&'static str] { &["json"] }
    fn language_kind(&self) -> Language { Language::Json }
}
