use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct DartLang;

// Pinned to the last ABI-14-compatible `tree-sitter-dart` (0.0.4, see the
// root Cargo.toml comment) — a much older grammar than the rest of this
// project's languages. It mis-parses some modern Dart forms (a top-level
// `int f(int x) => x;` arrow function lands as a bare `lambda_expression`,
// not a named declaration) — scoped down to what parses reliably: classes
// and their methods, not free-standing functions.
const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(class_definition name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(class_member_definition (method_signature (function_signature name: (identifier) @name))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
];

impl LanguageSupport for DartLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_dart::language() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        r#"(member_access . (identifier) @call.name (selector (argument_part))) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(import_specification (uri) @import.path) @import.decl"#
    }
    fn inherits_query(&self) -> &'static str {
        // A qualified base (`extends ns.Bar`) puts 2 unfielded
        // `type_identifier` children directly under `superclass`; the
        // trailing `.` anchors the capture to the last child so it resolves
        // to the real class name in both the bare and qualified cases
        // without a separate pattern also matching the leading `ns`.
        r#"(class_definition name: (identifier) @inherits.from
            superclass: (superclass (type_identifier) @inherits.to .))
           (class_definition name: (identifier) @inherits.from
            interfaces: (interfaces (type_identifier) @inherits.to))"#
    }
    fn references_query(&self) -> &'static str {
        // `function_signature`'s own return type is an unfielded direct
        // child (before the `name:` field); a parameter's type is
        // similarly unfielded inside `formal_parameter`.
        r#"(function_signature (type_identifier) @references.to name: (identifier) @references.from)
           (function_signature name: (identifier) @references.from
            (formal_parameter_list (formal_parameter (type_identifier) @references.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] { &["dart"] }
    fn language_kind(&self) -> Language { Language::Dart }
}
