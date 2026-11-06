use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct CSharpLang;

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
        pattern: "(struct_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: "(enum_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    SymbolQuery {
        // (_) matches both identifier (Foo) and qualified_name (Foo.Bar)
        pattern: "(namespace_declaration name: (_) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
    SymbolQuery {
        pattern: "(constructor_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
];

impl LanguageSupport for CSharpLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_c_sharp::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(invocation_expression function: [
    (identifier) @call.name
    (member_access_expression name: (identifier) @call.name)
  ]) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(using_directive (identifier) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // C#'s `base_list` holds both the base class and any implemented
        // interfaces as plain identifiers with no syntactic distinction
        // (the `I`-prefix convention isn't grammar-enforced) — captured
        // uniformly as Inherits, same as the language itself doesn't
        // distinguish them structurally.
        r#"(class_declaration name: (identifier) @inherits.from
            (base_list (identifier) @inherits.to))
           (class_declaration name: (identifier) @inherits.from
            (base_list (qualified_name name: (identifier) @inherits.to)))"#
    }
    fn references_query(&self) -> &'static str {
        // Field order must match the grammar's actual child order
        // (`returns:` precedes `name:` here).
        r#"(method_declaration returns: (identifier) @references.to
            name: (identifier) @references.from)
           (method_declaration name: (identifier) @references.from
            parameters: (parameter_list (parameter type: (identifier) @references.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["cs"] }
    fn language_kind(&self) -> Language { Language::CSharp }
}
