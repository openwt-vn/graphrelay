use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::Language;

/// `.svelte` files use the same script-block delegation as `.vue` — see
/// `vue_lang.rs`'s doc comment and `extractor.rs`'s `try_sfc_delegate`.
pub struct SvelteLang;

impl LanguageSupport for SvelteLang {
    fn ts_language(&self) -> tree_sitter::Language {
        // Never called — see VueLang::ts_language.
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
    fn symbol_queries(&self) -> &[SymbolQuery] { &[] }
    fn call_query(&self) -> &'static str { "" }
    fn import_query(&self) -> &'static str { "" }
    fn file_extensions(&self) -> &[&'static str] { &["svelte"] }
    fn language_kind(&self) -> Language { Language::Svelte }
}
