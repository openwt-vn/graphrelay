use super::{LanguageSupport, SymbolQuery};
use graphrelay_core::types::{Language, SymbolKind};

pub struct ElixirLang;

// Elixir's `defmodule`/`def`/`defp` are all just ordinary macro calls at the
// grammar level (`defmodule Foo do ... end` parses as a `call` whose target
// is the identifier "defmodule" and whose sole argument is itself a `call`
// for the module/function name), not dedicated declaration node types —
// there is no simpler query shape available.
const SYMBOL_QUERIES: &[SymbolQuery] = &[
    SymbolQuery {
        pattern: r#"(call target: (identifier) @kw
            (#eq? @kw "defmodule")
            (arguments (alias) @name)) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Module,
    },
    SymbolQuery {
        pattern: r#"(call target: (identifier) @kw
            (#eq? @kw "def")
            (arguments (call target: (identifier) @name))) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
    SymbolQuery {
        pattern: r#"(call target: (identifier) @kw
            (#eq? @kw "defp")
            (arguments (call target: (identifier) @name))) @def"#,
        name_capture: "name",
        def_capture: "def",
        kind: SymbolKind::Function,
    },
];

impl LanguageSupport for ElixirLang {
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_elixir::LANGUAGE.into() }
    fn symbol_queries(&self) -> &[SymbolQuery] { SYMBOL_QUERIES }
    fn call_query(&self) -> &'static str {
        // Excludes the declaration keywords themselves — they're `call`
        // nodes too (see the module doc above), and without this exclusion
        // every `def`/`defmodule`/etc. would also show up as a bogus call.
        r#"(call target: (identifier) @call.name
            (#not-eq? @call.name "def")
            (#not-eq? @call.name "defp")
            (#not-eq? @call.name "defmodule")
            (#not-eq? @call.name "import")
            (#not-eq? @call.name "alias")
            (#not-eq? @call.name "use")
            (#not-eq? @call.name "require")) @call.site"#
    }
    fn import_query(&self) -> &'static str {
        r#"(call target: (identifier) @import.fn (#eq? @import.fn "import") (arguments (alias) @import.path)) @import.decl
(call target: (identifier) @import.fn (#eq? @import.fn "alias") (arguments (alias) @import.path)) @import.decl
(call target: (identifier) @import.fn (#eq? @import.fn "use") (arguments (alias) @import.path)) @import.decl
(call target: (identifier) @import.fn (#eq? @import.fn "require") (arguments (alias) @import.path)) @import.decl"#
    }
    fn file_extensions(&self) -> &[&'static str] { &["ex", "exs"] }
    fn language_kind(&self) -> Language { Language::Elixir }
}
