use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct OcamlLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    // Only `let`-bindings that take at least one parameter count as
    // functions — a bare `let x = 1` is a value, not a symbol worth
    // indexing as one, matching this codebase's existing per-language
    // scoping of "simple declaration forms".
    SymbolQuery {
        pattern: "(value_definition (let_binding pattern: (value_name) @name (parameter))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // Generic fallback first so a record type_binding below can overwrite
    // it with the more specific Struct kind — same pattern as Go's
    // type_alias-then-struct ordering.
    SymbolQuery {
        pattern: "(type_binding name: (type_constructor) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
    SymbolQuery {
        pattern: "(type_binding name: (type_constructor) @name body: (record_declaration)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: "(module_definition (module_binding name: (module_name) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for OcamlLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_ocaml::LANGUAGE_OCAML.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(application_expression function: (value_path) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        // `open Foo` resolves to a hidden `_module_expression` supertype in
        // this grammar (not a queryable concrete node), and there is no
        // simple concrete-type query that covers it — left unsupported
        // rather than shipping a query that silently never matches.
        ""
    }
    // `.mli` interface files use a distinct sibling grammar
    // (`LANGUAGE_OCAML_INTERFACE`) this crate also exports; not wired up
    // here; `.mli` is still routed through the `.ml` implementation grammar,
    // which will parse many but not all interface-only constructs correctly.
    fn file_extensions(&self) -> &[&'static str] { &["ml", "mli"] }
    fn language_kind(&self) -> Language { Language::Ocaml }
}
