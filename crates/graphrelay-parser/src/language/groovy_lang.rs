use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct GroovyLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(class_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(method_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    // Top-level `def foo(...) { ... }`, outside any class body.
    SymbolQuery {
        pattern: "(function_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
];

impl LanguageSupport for GroovyLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_groovy::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(method_invocation name: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import_declaration (scoped_identifier) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // A qualified base (`extends ns.Bar`) parses as `scoped_type_identifier`,
        // whose 2 children are unfielded — capture the tail.
        r#"(class_declaration name: (identifier) @inherits.from
            superclass: (superclass (type_identifier) @inherits.to))
           (class_declaration name: (identifier) @inherits.from
            superclass: (superclass (scoped_type_identifier (_) (type_identifier) @inherits.to)))
           (class_declaration name: (identifier) @inherits.from
            interfaces: (super_interfaces (type_list (type_identifier) @inherits.to)))
           (class_declaration name: (identifier) @inherits.from
            interfaces: (super_interfaces (type_list (scoped_type_identifier (_) (type_identifier) @inherits.to))))
           (interface_declaration name: (identifier) @inherits.from
            (extends_interfaces (type_list (type_identifier) @inherits.to)))
           (interface_declaration name: (identifier) @inherits.from
            (extends_interfaces (type_list (scoped_type_identifier (_) (type_identifier) @inherits.to))))"#
    }
    fn references_query(&self) -> &'static str {
        // Field order must match the grammar's actual child order
        // (`type:` precedes `name:` here).
        r#"(method_declaration type: (type_identifier) @references.to
            name: (identifier) @references.from)
           (method_declaration name: (identifier) @references.from
            parameters: (formal_parameters (formal_parameter type: (type_identifier) @references.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["groovy", "gradle"] }
    fn language_kind(&self) -> Language { Language::Groovy }
}
