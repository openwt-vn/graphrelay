use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct PythonLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    // Top-level and nested function definitions (includes async def — same node in tree-sitter-python)
    SymbolQuery {
        pattern: "(function_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // Decorated functions: @staticmethod, @classmethod, @property, @dataclass, etc.
    SymbolQuery {
        pattern: "(decorated_definition
            (function_definition name: (identifier) @name)) @def",
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
];

impl LanguageSupport for PythonLang {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn symbol_queries(&self) -> &[SymbolQuery] {
        SYMBOL_QUERIES
    }

    fn call_query(&self) -> &'static str {
        r#"(call function: [
    (identifier) @call.name
    (attribute attribute: (identifier) @call.name)
  ]) @call.site"#
    }

    fn import_query(&self) -> &'static str {
        r#"(import_statement name: (_) @import.path) @import.decl
           (import_from_statement module_name: (_) @import.path) @import.decl"#
    }

    fn inherits_query(&self) -> &'static str {
        // Base classes are frequently module-qualified (`class Foo(frappe.ValidationError)`)
        // — same `attribute`-node handling as `call_query` above, tail identifier only.
        r#"(class_definition name: (identifier) @inherits.from
            superclasses: (argument_list [
              (identifier) @inherits.to
              (attribute attribute: (identifier) @inherits.to)
            ]))"#
    }

    fn references_query(&self) -> &'static str {
        // Type-hint references only (parameter/return type annotations) —
        // not every identifier mentioned in a function's body.
        r#"(function_definition name: (identifier) @references.from
            parameters: (parameters (typed_parameter type: (type (identifier) @references.to))))
           (function_definition name: (identifier) @references.from
            parameters: (parameters (typed_parameter type: (type (attribute attribute: (identifier) @references.to)))))
           (function_definition name: (identifier) @references.from
            return_type: (type (identifier) @references.to))
           (function_definition name: (identifier) @references.from
            return_type: (type (attribute attribute: (identifier) @references.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["py", "pyi"]
    }

    fn language_kind(&self) -> Language {
        Language::Python
    }
}
