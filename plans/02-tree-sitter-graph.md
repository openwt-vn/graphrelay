# Plan 02 — Tree-sitter Code Graph Extraction

## Goal
Parse source files for 14 languages using tree-sitter, extract symbols (functions, classes, structs, etc.) and edges (calls, imports, references), and build an in-memory code graph that can be persisted to SQLite.

## Success Criteria
- 14 languages parse without panic
- Symbol extraction covers: functions, classes, structs, enums, traits/interfaces, type aliases, constants
- Edge extraction covers: direct calls, import statements, trait/class inheritance
- Incremental re-parse (only changed byte ranges) works on a 100k LOC repo in < 200 ms
- Zero unsafe code in extraction path

---

## 1. Supported Languages at Launch

| # | Language | Tree-sitter grammar crate |
|---|----------|--------------------------|
| 1 | Rust | `tree-sitter-rust` |
| 2 | TypeScript | `tree-sitter-typescript` |
| 3 | JavaScript | `tree-sitter-javascript` |
| 4 | Python | `tree-sitter-python` |
| 5 | Go | `tree-sitter-go` |
| 6 | Java | `tree-sitter-java` |
| 7 | C | `tree-sitter-c` |
| 8 | C++ | `tree-sitter-cpp` |
| 9 | C# | `tree-sitter-c-sharp` |
| 10 | Ruby | `tree-sitter-ruby` |
| 11 | Kotlin | `tree-sitter-kotlin` |
| 12 | Swift | `tree-sitter-swift` |
| 13 | PHP | `tree-sitter-php` |
| 14 | Scala | `tree-sitter-scala` |

---

## 2. Grammar Build Strategy

Tree-sitter grammars are distributed as C source files. Compile them via `build.rs`:

```rust
// crates/codegraph-parser/build.rs
fn main() {
    let langs: &[(&str, &[&str])] = &[
        ("tree-sitter-rust",       &["src/parser.c", "src/scanner.c"]),
        ("tree-sitter-typescript", &["typescript/src/parser.c", "typescript/src/scanner.c"]),
        ("tree-sitter-javascript", &["src/parser.c", "src/scanner.c"]),
        // ... etc.
    ];

    for (name, sources) in langs {
        let mut build = cc::Build::new();
        build.include(format!("vendor/{name}"))
             .flag_if_supported("-Wno-unused-parameter")
             .flag_if_supported("-Wno-unused-but-set-variable");
        for s in *sources {
            build.file(format!("vendor/{name}/{s}"));
        }
        build.compile(name);
        println!("cargo:rustc-link-lib=static={name}");
    }
}
```

Vendor grammar sources under `crates/codegraph-parser/vendor/` (git submodules or vendored copies). This eliminates any runtime dependency on tree-sitter CLI.

---

## 3. Language Registry

```rust
// crates/codegraph-parser/src/language/mod.rs

pub trait LanguageSupport: Send + Sync {
    fn language(&self) -> tree_sitter::Language;
    fn symbol_queries(&self) -> &[SymbolQuery];
    fn call_query(&self) -> &str;
    fn import_query(&self) -> &str;
    fn file_extensions(&self) -> &[&str];
}

pub struct LanguageRegistry {
    langs: HashMap<Language, Box<dyn LanguageSupport>>,
}

impl LanguageRegistry {
    pub fn default() -> Self {
        let mut r = Self { langs: HashMap::new() };
        r.register(codegraph_core::Language::Rust,       Box::new(RustLang));
        r.register(codegraph_core::Language::TypeScript, Box::new(TypeScriptLang));
        // ...
        r
    }

    pub fn for_path(&self, path: &Path) -> Option<&dyn LanguageSupport> {
        let ext = path.extension()?.to_str()?;
        self.langs.values().find(|l| l.file_extensions().contains(&ext))
    }
}
```

---

## 4. Tree-sitter Queries (example: Rust)

Queries use [tree-sitter query syntax](https://tree-sitter.github.io/tree-sitter/using-parsers#query-syntax).

### Symbol query — functions
```scheme
; crates/codegraph-parser/src/language/rust/queries/symbols.scm
(function_item
  name: (identifier) @symbol.name
  parameters: (parameters) @symbol.params
  body: (block) @symbol.body) @symbol.def

(impl_item
  type: (_) @impl.type
  body: (declaration_list
    (function_item
      name: (identifier) @symbol.name) @symbol.def))

(struct_item   name: (type_identifier) @symbol.name) @symbol.def
(enum_item     name: (type_identifier) @symbol.name) @symbol.def
(trait_item    name: (type_identifier) @symbol.name) @symbol.def
(type_item     name: (type_identifier) @symbol.name) @symbol.def
(const_item    name: (identifier)      @symbol.name) @symbol.def
```

### Call edge query
```scheme
; calls.scm
(call_expression
  function: [
    (identifier) @call.name
    (scoped_identifier path: (_) name: (identifier) @call.name)
    (field_expression field: (field_identifier) @call.name)
  ]) @call.site
```

### Import query
```scheme
; imports.scm
(use_declaration argument: (_) @import.path) @import.decl
(extern_crate_declaration name: (identifier) @import.name) @import.decl
```

Each language implements equivalent queries. Queries are loaded from embedded `.scm` files via `include_str!`.

---

## 5. Extractor Implementation

```rust
// crates/codegraph-parser/src/extractor.rs

pub struct FileExtraction {
    pub file_id:   FileId,
    pub file_path: PathBuf,
    pub symbols:   Vec<Symbol>,
    pub edges:     Vec<Edge>,
    pub checksum:  u64,   // xxhash of file content, for change detection
}

pub struct Extractor {
    registry: Arc<LanguageRegistry>,
    parsers:  DashMap<Language, Mutex<tree_sitter::Parser>>,
}

impl Extractor {
    pub fn extract(&self, path: &Path, source: &[u8]) -> anyhow::Result<FileExtraction> {
        let lang_support = self.registry.for_path(path)
            .ok_or_else(|| anyhow!("unsupported language: {:?}", path))?;

        let mut parser = self.get_or_create_parser(lang_support.language());
        let tree = parser.parse(source, None)
            .ok_or_else(|| anyhow!("parse failed: {:?}", path))?;

        let symbols = self.extract_symbols(&tree, source, lang_support)?;
        let edges   = self.extract_edges(&tree, source, lang_support, &symbols)?;

        Ok(FileExtraction { symbols, edges, .. })
    }

    fn extract_symbols(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        lang: &dyn LanguageSupport,
    ) -> anyhow::Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        for query_def in lang.symbol_queries() {
            let query = tree_sitter::Query::new(lang.language(), query_def.pattern)?;
            let mut cursor = tree_sitter::QueryCursor::new();
            for m in cursor.matches(&query, tree.root_node(), source) {
                let name_node = m.nodes_for_capture_index(query_def.name_capture).next()
                    .ok_or_else(|| anyhow!("missing name capture"))?;
                let name = name_node.utf8_text(source)?.to_string();
                let span = Span {
                    start_byte: name_node.start_byte(),
                    end_byte:   name_node.end_byte(),
                    start_line: name_node.start_position().row as u32,
                    end_line:   name_node.end_position().row as u32,
                };
                symbols.push(Symbol {
                    id:        generate_symbol_id(),
                    name,
                    kind:      query_def.kind,
                    span,
                    docstring: extract_docstring(tree, &name_node, source),
                    ..
                });
            }
        }
        Ok(symbols)
    }
}
```

---

## 6. Call Resolution Strategy

Tree-sitter gives us syntactic call sites (the name being called) but not the resolved definition. Resolution strategy:

1. **Local scope first**: if the called name matches a symbol in the same file, link it directly.

2. **Import-guided** (detailed):
   - Parse the file's import statements using the import query (§4).
   - Build an `ImportMap: HashMap<alias, module_path>` for the file:
     - Rust `use foo::bar::Baz` → `{ "Baz": "foo::bar" }`
     - Python `from foo import bar` → `{ "bar": "foo" }`
     - TS/JS `import { fn } from "./module"` → `{ "fn": "./module" }`
   - For each call site `alias::method` or bare `alias`, look up `alias` in `ImportMap`.
   - Resolve the module path relative to the file's directory, then search the symbol store for `<resolved_module>::<method>`.
   - Accept if exactly one match; skip if ambiguous.

3. **Global name match (fuzzy fallback)**: search the symbol store for the called name across all files.
   - Tiebreaker order: (a) same language > other language, (b) highest in-degree > lowest, (c) alphabetically first.
   - Apply only when name matches exactly (no edit distance in v1 — too slow without an index).

4. **Unresolved calls**: if all three stages fail, store as `Edge { to_id: None, to_name: call_name, kind: Calls }` — still useful for impact analysis and chunk context even without a `SymbolId`.

**Accepted accuracy**: ~5–10% of calls will be unresolved or misresolved, matching the spec's stated tree-sitter vs. LSP tradeoff. This is validated in the call resolution accuracy tests (Plan 10 §12).

---

## 7. Incremental Parsing

tree-sitter supports incremental re-parsing via `InputEdit`. The watcher provides byte-level diff information:

```rust
pub struct FileChange {
    pub path:        PathBuf,
    pub old_source:  Bytes,
    pub new_source:  Bytes,
    pub edits:       Vec<tree_sitter::InputEdit>,  // from diff
}

impl Extractor {
    pub fn extract_incremental(
        &self,
        prev_tree: &tree_sitter::Tree,
        change: &FileChange,
    ) -> anyhow::Result<FileExtraction> {
        let mut parser = self.get_or_create_parser(..);
        let mut tree = prev_tree.clone();
        for edit in &change.edits {
            tree.edit(edit);
        }
        let new_tree = parser.parse(&change.new_source, Some(&tree))
            .ok_or_else(|| anyhow!("incremental parse failed"))?;
        // Only re-run queries on changed subtrees
        self.extract_changed_nodes(&new_tree, &tree, &change.new_source, ..)
    }
}
```

Cache `prev_tree` per file in a `DashMap<FileId, tree_sitter::Tree>`.

---

## 7b. Symbol ID Strategy

`SymbolId` is a **content-addressed `u64`**: `xxhash64(qualified_name_bytes)`.

```rust
pub fn generate_symbol_id(qualified_name: &str) -> SymbolId {
    xxhash_rust::xxh64::xxh64(qualified_name.as_bytes(), 0)
}
```

**Why content-addressed, not auto-increment?**
- **Stability across index rebuilds**: the ID for `src/query/engine.rs::QueryEngine::execute` is always the same hash, so incremental updates can upsert by ID without a lookup round-trip.
- **LanceDB coordination**: LanceDB uses `symbol_id` as a delete key (Plan 03 §4). Auto-increment IDs would change on re-index, making deletes unreliable.
- **Collision risk**: xxhash64 has a collision probability of ~1/2^64 across a codebase — negligible. If detected (two different qualified names hash to the same ID), the second symbol appends `_1` suffix to its qualified name before hashing.

**ID stability invariant**: if a symbol is renamed or moved, it gets a new ID and the old ID is deleted. The `upsert_symbols_batch()` call in Plan 03 handles this via `ON CONFLICT(qualified_name) DO UPDATE`.

---

## 8. Symbol Path Construction

For unambiguous symbol identification across files, construct a dotted path:

```
<relative_file_path>::<enclosing_type>::<symbol_name>
```

Examples:
- `src/query/engine.rs::QueryEngine::execute`
- `src/types.rs::Symbol`
- `packages/api/routes/search.ts::SearchHandler::handleRequest`

This path is stored in `Symbol.qualified_name` and used as the embedding chunk prefix.

---

## 9. Docstring Extraction

Per-language heuristics:
- **Rust**: `///` or `/** */` comment immediately preceding the item node
- **Python**: string literal as first statement in function/class body
- **TypeScript/JS**: JSDoc `/** */` comment immediately preceding
- **Go**: comment block ending on the line before the declaration
- **Java**: Javadoc `/** */`

Extract up to 500 characters. Strip leading `///` / `*` decorators.

---

## 10. Testing Strategy

```rust
#[cfg(test)]
mod tests {
    // Fixture: small Rust file with known symbols and calls
    const RUST_FIXTURE: &str = r#"
        fn foo() { bar(); baz(); }
        fn bar() {}
        fn baz() {}
        struct Config { name: String }
    "#;

    #[test]
    fn test_rust_symbol_extraction() {
        let extractor = Extractor::new(LanguageRegistry::default());
        let result = extractor.extract(Path::new("test.rs"), RUST_FIXTURE.as_bytes()).unwrap();
        assert_eq!(result.symbols.len(), 4); // foo, bar, baz, Config
        let fn_names: Vec<_> = result.symbols.iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .map(|s| s.name.as_str())
            .collect();
        assert!(fn_names.contains(&"foo"));
    }

    #[test]
    fn test_rust_call_extraction() {
        // ...
        let calls: Vec<_> = result.edges.iter()
            .filter(|e| e.kind == EdgeKind::Calls)
            .collect();
        assert_eq!(calls.len(), 2); // foo→bar, foo→baz
    }
}
```

Add fixture files for each of the 14 languages under `crates/codegraph-parser/tests/fixtures/`.

---

## 11. Implementation Steps

1. Add grammar vendor submodules (or vendored C sources) under `crates/codegraph-parser/vendor/`
2. Write `build.rs` to compile all 14 grammars
3. Implement `LanguageSupport` trait
4. Write tree-sitter query `.scm` files for all 14 languages (symbols + calls + imports)
5. Implement `Extractor::extract()` (full parse)
6. Implement call resolution (local → import-guided → fuzzy)
7. Implement docstring extraction
8. Implement `Extractor::extract_incremental()` (tree-sitter edits)
9. Write unit tests for each language using fixture files
10. Benchmark: index 100k LOC, verify < 200 ms incremental re-parse

---

## 12. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| Unresolved calls | Drop | Skip or store | Store as `UnresolvedEdge` — useful for impact analysis |
| Grammar vendoring | git submodule vs. copy | Submodule — easier to update |
| Call resolution depth | Shallow (same-file) vs. cross-file | Cross-file via import tracking |
| Docstring max length | 200 / 500 / 1000 chars | 500 — enough for semantic context, not too large for embedding |
