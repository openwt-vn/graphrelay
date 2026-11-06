use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct GoLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(method_declaration name: (field_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    // True alias: type Foo = Bar  (type_alias node, not type_spec)
    SymbolQuery {
        pattern: "(type_declaration (type_alias name: (type_identifier) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
    // Generic named type fallback — covers `type Foo int`, `type Status string`, etc.
    // Placed BEFORE struct/interface so they can overwrite with a more specific kind.
    SymbolQuery {
        pattern: "(type_declaration (type_spec name: (type_identifier) @name)) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::TypeAlias,
    },
    // Struct type: positional child is struct_type — overwrites TypeAlias for structs
    SymbolQuery {
        pattern: "(type_declaration (type_spec name: (type_identifier) @name (struct_type))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    // Interface type: positional child is interface_type — overwrites TypeAlias for interfaces
    SymbolQuery {
        pattern: "(type_declaration (type_spec name: (type_identifier) @name (interface_type))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Interface,
    },
];

/// The base type of a receiver type expression: `T`, `*T`, `T[K, V]`, `*T[K]`,
/// `(T)`. Pre-order, so for a generic the type itself comes before its
/// type arguments.
fn first_type_identifier(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "type_identifier" {
        return node.utf8_text(source).ok().map(str::to_string);
    }
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find_map(|c| first_type_identifier(c, source));
    found
}

impl LanguageSupport for GoLang {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }

    fn symbol_queries(&self) -> &[SymbolQuery] {
        SYMBOL_QUERIES
    }

    fn call_query(&self) -> &'static str {
        r#"(call_expression function: [
    (identifier) @call.name
    (selector_expression field: (field_identifier) @call.name)
  ]) @call.site"#
    }

    fn import_query(&self) -> &'static str {
        r#"(import_spec path: (interpreted_string_literal) @import.path) @import.decl"#
    }

    fn references_query(&self) -> &'static str {
        r#"(function_declaration name: (identifier) @references.from
            result: (type_identifier) @references.to)
           (function_declaration name: (identifier) @references.from
            parameters: (parameter_list (parameter_declaration type: (type_identifier) @references.to)))
           (method_declaration name: (field_identifier) @references.from
            result: (type_identifier) @references.to)
           (method_declaration name: (field_identifier) @references.from
            parameters: (parameter_list (parameter_declaration type: (type_identifier) @references.to)))"#
    }

    fn receiver_type(&self, def: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
        if def.kind() != "method_declaration" {
            return None;
        }
        let receiver = def.child_by_field_name("receiver")?;
        let mut cursor = receiver.walk();
        let param = receiver.named_children(&mut cursor).find(|n| n.kind() == "parameter_declaration")?;
        let ty = param.child_by_field_name("type")?;
        first_type_identifier(ty, source)
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["go"]
    }

    fn language_kind(&self) -> Language {
        Language::Go
    }
}
