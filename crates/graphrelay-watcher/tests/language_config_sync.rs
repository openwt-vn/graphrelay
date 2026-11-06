//! Guards against the exact drift bug found while adding Kotlin/Lua support:
//! `graphrelay-core`'s default `[index] languages`
//! list and `graphrelay-parser`'s `LanguageRegistry` are two independent
//! places that both need to agree on "which languages are supported," and
//! nothing in the type system enforces that. Before this test existed,
//! `default_languages()` only listed 5 of the (then) 13 registered
//! languages, so most languages the parser could extract were never
//! actually watched by default.

use graphrelay_core::{types::Language, Config};
use graphrelay_parser::LanguageRegistry;
use std::collections::HashSet;

#[test]
fn default_languages_cover_every_registered_language() {
    let registry = LanguageRegistry::with_defaults();
    let registered: HashSet<Language> = registry.registered_languages().into_iter().collect();

    let default_config = Config::default();
    let configured: HashSet<Language> = default_config
        .index
        .languages
        .iter()
        .filter_map(|s| Language::from_str(s))
        .collect();

    let missing: Vec<&Language> = registered.difference(&configured).collect();
    assert!(
        missing.is_empty(),
        "default_languages() is missing these registered languages: {missing:?} — \
         a file in one of these languages will never be watched by default. \
         Add its config name to `default_languages()` in graphrelay-core/src/config.rs.",
    );

    // Not a hard failure (an intentionally-listed-but-not-yet-registered
    // language is a fine transitional state), but worth surfacing loudly if
    // it happens silently.
    let extra: Vec<&Language> = configured.difference(&registered).collect();
    assert!(
        extra.is_empty(),
        "default_languages() lists these languages but LanguageRegistry has no \
         extractor for them: {extra:?} — either register one or remove the config entry.",
    );
}

/// Every language in the default config must map to at least one real file
/// extension via the same `LanguageRegistry` the extractor uses — this is
/// the specific mechanism that silently broke for csharp/ruby/kotlin before
/// `languages_to_extensions` was rewritten to derive from the registry
/// instead of a hand-maintained match statement.
#[test]
fn every_default_language_resolves_to_a_real_extension() {
    let registry = LanguageRegistry::with_defaults();
    for lang_name in &Config::default().index.languages {
        let lang = Language::from_str(lang_name)
            .unwrap_or_else(|| panic!("'{lang_name}' is not a recognized Language"));
        let support = registry
            .for_language(lang)
            .unwrap_or_else(|| panic!("'{lang_name}' has no registered LanguageSupport"));
        assert!(
            !support.file_extensions().is_empty(),
            "'{lang_name}' resolved to a LanguageSupport with zero file extensions",
        );
    }
}
