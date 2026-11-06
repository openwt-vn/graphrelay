use graphrelay_core::types::Language;
use std::collections::HashMap;
use std::path::Path;

pub mod rust_lang;
pub mod typescript_lang;
pub mod javascript_lang;
pub mod python_lang;
pub mod go_lang;
pub mod java_lang;
pub mod c_lang;
pub mod cpp_lang;
pub mod csharp_lang;
pub mod ruby_lang;
pub mod kotlin_lang;
pub mod swift_lang;
pub mod php_lang;
pub mod scala_lang;
pub mod lua_lang;
pub mod vue_lang;
pub mod svelte_lang;
pub mod bash_lang;
pub mod zig_lang;
pub mod elixir_lang;
pub mod julia_lang;
pub mod fortran_lang;
pub mod ocaml_lang;
pub mod commonlisp_lang;
pub mod verilog_lang;
pub mod pascal_lang;
pub mod dart_lang;
pub mod objc_lang;
pub mod groovy_lang;
pub mod powershell_lang;
pub mod json_lang;

pub use rust_lang::RustLang;
pub use typescript_lang::TypeScriptLang;
pub use javascript_lang::JavaScriptLang;
pub use python_lang::PythonLang;
pub use go_lang::GoLang;
pub use java_lang::JavaLang;
pub use c_lang::CLang;
pub use cpp_lang::CppLang;
pub use csharp_lang::CSharpLang;
pub use ruby_lang::RubyLang;
pub use kotlin_lang::KotlinLang;
pub use swift_lang::SwiftLang;
pub use php_lang::PhpLang;
pub use scala_lang::ScalaLang;
pub use lua_lang::LuaLang;
pub use vue_lang::VueLang;
pub use svelte_lang::SvelteLang;
pub use bash_lang::BashLang;
pub use zig_lang::ZigLang;
pub use elixir_lang::ElixirLang;
pub use julia_lang::JuliaLang;
pub use fortran_lang::FortranLang;
pub use ocaml_lang::OcamlLang;
pub use commonlisp_lang::CommonLispLang;
pub use verilog_lang::VerilogLang;
pub use pascal_lang::PascalLang;
pub use dart_lang::DartLang;
pub use objc_lang::ObjcLang;
pub use groovy_lang::GroovyLang;
pub use powershell_lang::PowerShellLang;
pub use json_lang::JsonLang;

/// Query pattern for extracting symbols.
#[derive(Debug, Clone)]
pub struct SymbolQuery {
    /// Tree-sitter query pattern string.
    pub pattern: &'static str,
    /// Name of the capture that holds the symbol identifier.
    pub name_capture: &'static str,
    /// Kind of symbol this query targets.
    pub kind: graphrelay_core::types::SymbolKind,
    /// Name of the capture that encompasses the full definition (for body extraction).
    pub def_capture: &'static str,
}

/// Trait for language-specific tree-sitter support.
pub trait LanguageSupport: Send + Sync {
    fn ts_language(&self) -> tree_sitter::Language;
    fn symbol_queries(&self) -> &[SymbolQuery];
    fn call_query(&self) -> &'static str;
    fn import_query(&self) -> &'static str;
    /// Query producing `EdgeKind::Inherits` edges: class/base-class (or
    /// interface/protocol) pairs, one `@inherits.from`/`@inherits.to`
    /// capture pair per match — see `extract_edges`. Defaulted to `""`
    /// (no matches) since not every language has classical inheritance
    /// (Rust, Go, C, and most Phase A/C additions don't); only override
    /// where the concept genuinely applies.
    fn inherits_query(&self) -> &'static str { "" }
    fn references_query(&self) -> &'static str { "" }
    fn implements_query(&self) -> &'static str { "" }
    /// Name of the type a method belongs to when the grammar declares it
    /// outside the type's own span (Go: `func (r *T) M()` is a top-level
    /// declaration, so the span-containment lookup in `extract_symbols` finds
    /// no enclosing type). `def` is the symbol's definition node. Defaults to
    /// `None`: in other languages methods are nested inside their type.
    fn receiver_type(&self, _def: tree_sitter::Node<'_>, _source: &[u8]) -> Option<String> { None }
    fn file_extensions(&self) -> &[&'static str];
    fn language_kind(&self) -> Language;
}

pub struct LanguageRegistry {
    langs: HashMap<Language, Box<dyn LanguageSupport>>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self {
            langs: HashMap::new(),
        }
    }

    pub fn with_defaults() -> Self {
        let mut r = Self::new();
        r.register(Language::Rust,       Box::new(RustLang));
        r.register(Language::TypeScript, Box::new(TypeScriptLang));
        r.register(Language::JavaScript, Box::new(JavaScriptLang));
        r.register(Language::Python,     Box::new(PythonLang));
        r.register(Language::Go,         Box::new(GoLang));
        r.register(Language::Java,       Box::new(JavaLang));
        r.register(Language::C,          Box::new(CLang));
        r.register(Language::Cpp,        Box::new(CppLang));
        r.register(Language::CSharp,     Box::new(CSharpLang));
        r.register(Language::Ruby,       Box::new(RubyLang));
        r.register(Language::Kotlin,     Box::new(KotlinLang));
        r.register(Language::Swift,      Box::new(SwiftLang));
        r.register(Language::Php,        Box::new(PhpLang));
        r.register(Language::Scala,      Box::new(ScalaLang));
        r.register(Language::Lua,        Box::new(LuaLang));
        r.register(Language::Vue,        Box::new(VueLang));
        r.register(Language::Svelte,     Box::new(SvelteLang));
        r.register(Language::Bash,       Box::new(BashLang));
        r.register(Language::Zig,        Box::new(ZigLang));
        r.register(Language::Elixir,     Box::new(ElixirLang));
        r.register(Language::Julia,      Box::new(JuliaLang));
        r.register(Language::Fortran,    Box::new(FortranLang));
        r.register(Language::Ocaml,      Box::new(OcamlLang));
        r.register(Language::CommonLisp, Box::new(CommonLispLang));
        r.register(Language::Verilog,    Box::new(VerilogLang));
        r.register(Language::Pascal,     Box::new(PascalLang));
        r.register(Language::Dart,       Box::new(DartLang));
        r.register(Language::ObjectiveC, Box::new(ObjcLang));
        r.register(Language::Groovy,     Box::new(GroovyLang));
        r.register(Language::PowerShell, Box::new(PowerShellLang));
        r.register(Language::Json,       Box::new(JsonLang));
        r
    }

    pub fn register(&mut self, lang: Language, support: Box<dyn LanguageSupport>) {
        self.langs.insert(lang, support);
    }

    pub fn for_path(&self, path: &Path) -> Option<&dyn LanguageSupport> {
        let ext = path.extension()?.to_str()?;
        self.langs
            .values()
            .find(|l| l.file_extensions().contains(&ext))
            .map(|b| b.as_ref())
    }

    /// All languages currently registered — used to keep other subsystems
    /// (e.g. `graphrelay-watcher`'s default `[index] languages` config) in
    /// sync with what the extractor actually supports, rather than
    /// hand-maintaining a second list that can silently drift.
    pub fn registered_languages(&self) -> Vec<Language> {
        self.langs.keys().copied().collect()
    }

    pub fn for_language(&self, lang: Language) -> Option<&dyn LanguageSupport> {
        self.langs.get(&lang).map(|b| b.as_ref())
    }
}

impl Default for LanguageRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;

    #[test]
    fn every_symbol_query_compiles_against_its_grammar() {
        let registry = LanguageRegistry::default();
        let mut broken = Vec::new();
        for lang in registry.registered_languages() {
            let support = registry.for_language(lang).unwrap();
            for q in support.symbol_queries() {
                if let Err(e) = tree_sitter::Query::new(&support.ts_language(), q.pattern) {
                    broken.push(format!("{lang:?}: {} -> {e}", q.pattern));
                }
            }
        }
        assert!(broken.is_empty(), "symbol queries that do not compile:\n{}", broken.join("\n"));
    }
}
