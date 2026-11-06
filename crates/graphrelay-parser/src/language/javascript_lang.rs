use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct JavaScriptLang;

const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: "(function_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // const/let foo = () => {}  or  const foo = function() {}
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
        pattern: "(lexical_declaration (variable_declarator name: (identifier) @name value: (generator_function))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    // var foo = function() {} (older style)
    SymbolQuery {
        pattern: "(variable_declaration (variable_declarator name: (identifier) @name value: (arrow_function))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(variable_declaration (variable_declarator name: (identifier) @name value: (function_expression))) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: "(class_declaration name: (identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Class,
    },
    SymbolQuery {
        pattern: "(method_definition name: (property_identifier) @name) @def",
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Method,
    },
    // Object-literal property whose value is a function, e.g.
    // `{ refresh: function (frm) {...} }` — the ES6 shorthand form
    // (`{ refresh(frm) {...} }`) is already a `method_definition` node
    // (matched above) regardless of whether it sits in a class body or a
    // plain object literal; this pattern covers the colon + function-
    // expression form, which is a distinct node shape (`pair`) with no
    // prior query. Common in config/options objects generally and, in this
    // project's own benchmark corpus, in every frappe.io
    // `frappe.ui.form.on("Doctype", { event: function(frm) {...} })` block.
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

impl LanguageSupport for JavaScriptLang {
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_javascript::LANGUAGE.into()
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
        // Unlike TS, JS's `class_heritage` wraps the extends expression
        // directly with no `extends_clause` node — a bare identifier or,
        // for a module-qualified base (`class Foo extends ns.Bar`), a
        // `member_expression`.
        r#"(class_declaration name: (identifier) @inherits.from
            (class_heritage (identifier) @inherits.to))
           (class_declaration name: (identifier) @inherits.from
            (class_heritage (member_expression property: (property_identifier) @inherits.to)))"#
    }

    fn file_extensions(&self) -> &[&'static str] {
        &["js", "jsx", "mjs", "cjs"]
    }

    fn language_kind(&self) -> Language {
        Language::JavaScript
    }
}
