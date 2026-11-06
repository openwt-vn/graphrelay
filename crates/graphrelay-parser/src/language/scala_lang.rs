use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct ScalaLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(class_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(object_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
    SymbolQuery {
        pattern: "(trait_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Trait,
    },
    // `case class` is a class_definition with a `case` modifier in this grammar; the query above covers it.
    // type Foo = Bar  (type aliases)
    SymbolQuery {
        pattern: "(type_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
    // Scala 3 enum
    SymbolQuery {
        pattern: "(enum_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    // Method calls via dot chain: foo.bar(...)
    // call_query already handles chained selectors; this improves the Scala-specific
    // field access patterns
];

impl LanguageSupport for ScalaLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_scala::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call_expression function: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import_declaration (stable_identifier) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // extends_clause's `type:` field repeats for `extends Bar with Baz`
        // — the query matches each occurrence, so both land as Inherits.
        // A qualified base (`extends ns.Bar`) parses as `stable_type_identifier`,
        // whose 2 children are unfielded — capture the tail.
        r#"(class_definition name: (identifier) @inherits.from
            (extends_clause type: (type_identifier) @inherits.to))
           (class_definition name: (identifier) @inherits.from
            (extends_clause type: (stable_type_identifier (_) (type_identifier) @inherits.to)))
           (trait_definition name: (identifier) @inherits.from
            (extends_clause type: (type_identifier) @inherits.to))
           (trait_definition name: (identifier) @inherits.from
            (extends_clause type: (stable_type_identifier (_) (type_identifier) @inherits.to)))
           (object_definition name: (identifier) @inherits.from
            (extends_clause type: (type_identifier) @inherits.to))
           (object_definition name: (identifier) @inherits.from
            (extends_clause type: (stable_type_identifier (_) (type_identifier) @inherits.to)))"#
    }
    fn references_query(&self) -> &'static str {
        r#"(function_definition name: (identifier) @references.from
            return_type: (type_identifier) @references.to)
           (function_definition name: (identifier) @references.from
            parameters: (parameters (parameter type: (type_identifier) @references.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["scala", "sc"] }
    fn language_kind(&self) -> Language { Language::Scala }
}
