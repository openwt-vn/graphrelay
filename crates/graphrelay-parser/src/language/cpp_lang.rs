use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct CppLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: r#"(function_definition
  declarator: (function_declarator
    declarator: [(identifier)(field_identifier)] @name)) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(class_specifier name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(struct_specifier name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: "(enum_specifier name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    SymbolQuery {
        pattern: "(namespace_definition name: (namespace_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
];

impl LanguageSupport for CppLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_cpp::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(call_expression function: [
    (identifier) @call.name
    (field_expression field: (field_identifier) @call.name)
  ]) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(preproc_include path: (_) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // `: public ns::Bar` parses the base as `qualified_identifier`
        // (a `name:` field) rather than a bare `type_identifier`.
        r#"(class_specifier name: (type_identifier) @inherits.from
            (base_class_clause (type_identifier) @inherits.to))
           (class_specifier name: (type_identifier) @inherits.from
            (base_class_clause (qualified_identifier name: (type_identifier) @inherits.to)))
           (struct_specifier name: (type_identifier) @inherits.from
            (base_class_clause (type_identifier) @inherits.to))
           (struct_specifier name: (type_identifier) @inherits.from
            (base_class_clause (qualified_identifier name: (type_identifier) @inherits.to)))"#
    }
    fn references_query(&self) -> &'static str {
        // Only inline definitions (function_definition — the shape our own
        // symbol_queries capture as a symbol); a prototype-only
        // field_declaration has no matching "from" symbol to attach to.
        r#"(function_definition
            type: (type_identifier) @references.to
            declarator: (function_declarator declarator: [(identifier)(field_identifier)] @references.from))
           (function_definition
            declarator: (function_declarator
              declarator: [(identifier)(field_identifier)] @references.from
              parameters: (parameter_list (parameter_declaration type: (type_identifier) @references.to))))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["cpp", "cc", "cxx", "hpp", "hxx"] }
    fn language_kind(&self) -> Language { Language::Cpp }
}
