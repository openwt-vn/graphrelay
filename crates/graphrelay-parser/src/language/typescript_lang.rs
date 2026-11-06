use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct TypeScriptLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // const/let foo = (): void => {}  — dominant modern TS pattern
    SymbolQuery {
        pattern: "(lexical_declaration (variable_declarator name: (identifier) @name value: (arrow_function))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(lexical_declaration (variable_declarator name: (identifier) @name value: (function_expression))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(class_declaration name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(abstract_class_declaration name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(interface_declaration name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Interface,
    },
    SymbolQuery {
        pattern: "(type_alias_declaration name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
    SymbolQuery {
        pattern: "(method_definition name: (property_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    SymbolQuery {
        pattern: "(enum_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    // Object-literal property whose value is a function — see the identical
    // pattern's comment in javascript_lang.rs for the motivating case (the
    // ES6 shorthand form is already covered by method_definition above;
    // this is specifically the colon + function-expression form, a `pair`
    // node with no prior query).
    SymbolQuery {
        pattern: "(pair key: (property_identifier) @name value: (function_expression)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    SymbolQuery {
        pattern: "(pair key: (property_identifier) @name value: (arrow_function)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
];

impl LanguageSupport for TypeScriptLang {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }

    fn symbol_queries(&self) -> &[SymbolQuery] {
        SYMBOL_QUERIES
    }

    fn call_query(&self) -> &'static str {
        r#"(call_expression
  function: [
    (identifier) @call.name
    (member_expression property: (property_identifier) @call.name)
  ]) @call.site"#
    }

    fn import_query(&self) -> &'static str {
        r#"(import_statement source: (string) @import.path) @import.decl"#
    }

    fn inherits_query(&self) -> &'static str {
        // Base classes/interfaces are frequently module-qualified
        // (`class Foo extends ns.Bar implements ns.Baz`) — `extends_clause`'s
        // qualified form is a `member_expression`, and `implements_clause`'s
        // is a `nested_type_identifier`; both need their own pattern
        // alongside the plain-identifier form.
        r#"(class_declaration name: (type_identifier) @inherits.from
            (class_heritage (extends_clause value: (identifier) @inherits.to)))
           (class_declaration name: (type_identifier) @inherits.from
            (class_heritage (extends_clause value: (member_expression property: (property_identifier) @inherits.to))))
           (class_declaration name: (type_identifier) @inherits.from
            (class_heritage (implements_clause (type_identifier) @inherits.to)))
           (class_declaration name: (type_identifier) @inherits.from
            (class_heritage (implements_clause (nested_type_identifier name: (type_identifier) @inherits.to))))
           (abstract_class_declaration name: (type_identifier) @inherits.from
            (class_heritage (extends_clause value: (identifier) @inherits.to)))
           (abstract_class_declaration name: (type_identifier) @inherits.from
            (class_heritage (extends_clause value: (member_expression property: (property_identifier) @inherits.to))))
           (abstract_class_declaration name: (type_identifier) @inherits.from
            (class_heritage (implements_clause (type_identifier) @inherits.to)))
           (abstract_class_declaration name: (type_identifier) @inherits.from
            (class_heritage (implements_clause (nested_type_identifier name: (type_identifier) @inherits.to))))
           (interface_declaration name: (type_identifier) @inherits.from
            (extends_type_clause type: (type_identifier) @inherits.to))
           (interface_declaration name: (type_identifier) @inherits.from
            (extends_type_clause type: (nested_type_identifier name: (type_identifier) @inherits.to)))"#
    }

    fn references_query(&self) -> &'static str {
        // Type-annotation references only (parameter/return types) — not
        // every identifier mentioned in a function/method's body.
        r#"(function_declaration name: (identifier) @references.from
            parameters: (formal_parameters (required_parameter type: (type_annotation (type_identifier) @references.to))))
           (function_declaration name: (identifier) @references.from
            parameters: (formal_parameters (optional_parameter type: (type_annotation (type_identifier) @references.to))))
           (function_declaration name: (identifier) @references.from
            parameters: (formal_parameters (required_parameter type: (type_annotation (nested_type_identifier name: (type_identifier) @references.to)))))
           (function_declaration name: (identifier) @references.from
            return_type: (type_annotation (type_identifier) @references.to))
           (function_declaration name: (identifier) @references.from
            return_type: (type_annotation (nested_type_identifier name: (type_identifier) @references.to)))
           (method_definition name: (property_identifier) @references.from
            parameters: (formal_parameters (required_parameter type: (type_annotation (type_identifier) @references.to))))
           (method_definition name: (property_identifier) @references.from
            parameters: (formal_parameters (optional_parameter type: (type_annotation (type_identifier) @references.to))))
           (method_definition name: (property_identifier) @references.from
            return_type: (type_annotation (type_identifier) @references.to))"#
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["ts", "tsx"]
    }

    fn language_kind(&self) -> Language {
        Language::TypeScript
    }
}
