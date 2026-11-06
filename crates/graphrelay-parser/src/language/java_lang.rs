use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct JavaLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(method_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    SymbolQuery {
        pattern: "(class_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(interface_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Interface,
    },
    SymbolQuery {
        pattern: "(enum_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    SymbolQuery {
        pattern: "(constructor_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // Java 16+ record classes
    SymbolQuery {
        pattern: "(record_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    // @interface annotation type declarations
    SymbolQuery {
        pattern: "(annotation_type_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Interface,
    },
];

impl LanguageSupport for JavaLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_java::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(method_invocation name: (identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import_declaration (scoped_identifier) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // Package-qualified base types (`extends pkg.Bar`) parse as
        // `scoped_type_identifier`, whose 2 children are unfielded in this
        // grammar — capture the tail (the actual class name) positionally.
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
        // Field order in the pattern must match the grammar's actual child
        // order (`type:` precedes `name:` here) — tree-sitter's query
        // compiler rejects an out-of-order field pattern as a structure
        // error, even though field-based matching is otherwise unordered.
        r#"(method_declaration type: (type_identifier) @references.to
            name: (identifier) @references.from)
           (method_declaration name: (identifier) @references.from
            parameters: (formal_parameters (formal_parameter type: (type_identifier) @references.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["java"] }
    fn language_kind(&self) -> Language { Language::Java }
}
