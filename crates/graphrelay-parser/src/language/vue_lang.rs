use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::Language;

/// `.vue` Single-File Components are not parsed with their own tree-sitter
/// grammar — `Extractor` special-cases `Language::Vue`/`Language::Svelte`
/// (see `extractor.rs`'s `try_sfc_delegate`) to slice out the `<script>`
/// block and delegate parsing to the existing TypeScript/JavaScript
/// `LanguageSupport` impls, then offset the resulting spans back into the
/// original file's coordinates. This impl exists only so `.vue` files are
/// recognized and routed to that delegation path by `LanguageRegistry`;
/// `ts_language()`/`symbol_queries()`/`call_query()`/`import_query()` are
/// never actually invoked for this language and are stubbed accordingly.
pub struct VueLang;

impl LanguageSupport for VueLang {
    fn ts_language(&self) -> tree_sitter::Language {
        // Never called: `try_sfc_delegate` intercepts before `Extractor::parse`
        // would reach this. TypeScript is a harmless placeholder.
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
    fn symbol_queries(&self) -> &[SymbolQuery] { &[] }
    fn call_query(&self) -> &'static str { "" }
    fn import_query(&self) -> &'static str { "" }
    fn file_extensions(&self) -> &[&'static str] { &["vue"] }
    fn language_kind(&self) -> Language { Language::Vue }
}
