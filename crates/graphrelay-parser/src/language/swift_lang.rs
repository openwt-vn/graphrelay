use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct SwiftLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_declaration name: (simple_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // tree-sitter-swift uses class_declaration for all value types; discriminate via keyword child.
    SymbolQuery {
        pattern: r#"(class_declaration "class" name: (type_identifier) @name) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: r#"(class_declaration "actor" name: (type_identifier) @name) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: r#"(class_declaration "struct" name: (type_identifier) @name) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Struct,
    },
    SymbolQuery {
        pattern: r#"(class_declaration "enum" name: (type_identifier) @name) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Enum,
    },
    SymbolQuery {
        pattern: "(protocol_declaration name: (type_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Trait,
    },
];

impl LanguageSupport for SwiftLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_swift::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        // tree-sitter-swift has no "function:" field on call_expression; use positional match.
        r#"(call_expression (simple_identifier) @call.name) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import_declaration (identifier) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // class_declaration also covers struct/enum/actor (discriminated
        // only by a keyword child, see symbol_queries above), so this one
        // pattern captures superclass/protocol conformance for all of them.
        // A qualified conformance (`: ns.Bar`) puts 2 unfielded
        // `type_identifier` children directly under `user_type`; the
        // trailing `.` anchors the capture to the last child so it resolves
        // to the real type name in both the bare and qualified cases
        // without a separate pattern also matching the leading `ns`.
        r#"(class_declaration name: (type_identifier) @inherits.from
            (inheritance_specifier inherits_from: (user_type (type_identifier) @inherits.to .)))
           (protocol_declaration name: (type_identifier) @inherits.from
            (inheritance_specifier inherits_from: (user_type (type_identifier) @inherits.to .)))"#
    }
    fn references_query(&self) -> &'static str {
        // The return type's `user_type` is a direct child of
        // function_declaration; a parameter's is nested inside `parameter`
        // — the differing depth (rather than field name, which this
        // grammar reuses oddly between the two) keeps the patterns apart.
        r#"(function_declaration name: (simple_identifier) @references.from
            (user_type (type_identifier) @references.to))
           (function_declaration name: (simple_identifier) @references.from
            (parameter (user_type (type_identifier) @references.to)))"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["swift"] }
    fn language_kind(&self) -> Language { Language::Swift }
}
