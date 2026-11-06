use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct PhpLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_definition name: (name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(method_declaration name: (name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    SymbolQuery {
        pattern: "(class_declaration name: (name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(interface_declaration name: (name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Interface,
    },
    SymbolQuery {
        pattern: "(trait_declaration name: (name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Trait,
    },
    // PHP 8.1+ backed enums: enum Status: string { case Active = 'active'; }
    SymbolQuery {
        pattern: "(enum_declaration name: (name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    // namespace Foo\Bar — used as a module boundary
    SymbolQuery {
        pattern: "(namespace_definition name: (namespace_name) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for PhpLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_php::LANGUAGE_PHP.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(function_call_expression function: (name) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(include_expression (string (string_value) @import.path)) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // Namespace-qualified bases (`extends \Ns\Bar`) parse as
        // `qualified_name`, whose trailing `(name)` (the class itself,
        // after the `namespace_name` prefix) is the last positional child.
        r#"(class_declaration name: (name) @inherits.from
            (base_clause (name) @inherits.to))
           (class_declaration name: (name) @inherits.from
            (base_clause (qualified_name (name) @inherits.to)))
           (class_declaration name: (name) @inherits.from
            (class_interface_clause (name) @inherits.to))
           (class_declaration name: (name) @inherits.from
            (class_interface_clause (qualified_name (name) @inherits.to)))
           (interface_declaration name: (name) @inherits.from
            (base_clause (name) @inherits.to))
           (interface_declaration name: (name) @inherits.from
            (base_clause (qualified_name (name) @inherits.to)))"#
    }
    fn references_query(&self) -> &'static str {
        r#"(function_definition name: (name) @references.from
            return_type: (named_type (name) @references.to))
           (function_definition name: (name) @references.from
            parameters: (formal_parameters (simple_parameter type: (named_type (name) @references.to))))
           (method_declaration name: (name) @references.from
            return_type: (named_type (name) @references.to))
           (method_declaration name: (name) @references.from
            parameters: (formal_parameters (simple_parameter type: (named_type (name) @references.to))))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["php"] }
    fn language_kind(&self) -> Language { Language::Php }
}
