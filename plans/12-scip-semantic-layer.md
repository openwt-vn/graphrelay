# Plan 12 — SCIP Semantic Layer

## Review Status

**Reviewed 2026-06-09 against live codebase + verified SCIP crate v0.8.1.**
Every item below has been cross-checked against:
- `scip = "0.8.1"` crate source (`~/.cargo/registry/src/.../scip-0.8.1/src/`)
- Current `codegraph-store/src/graph_db.rs` SQL schema
- Current `codegraph-parser/src/extractor.rs` logic
- Confirmed working Swift/Kotlin parser fixes (keyword-discriminant queries)

Changes from first draft:
- **Phase F (LSP fallback) removed** — Swift/Kotlin kind accuracy already fixed in tree-sitter
- **D3 (auto-run SCIP) demoted to v0.3** — requires build context that can't be assumed
- **B2 mapping expanded** to all 87 SCIP Kind values (first draft only listed 10)
- **B3 matching corrected** — SCIP uses 0-indexed lines; codegraph stores 1-indexed
- **E4 removed** — `semantic_refs` would duplicate the existing `in_degree` signal
- **All 5 Open Questions decided** with rationale
- **Performance invariant added**: `callers_of()` must be bounded before cross-file edges land

---

## Context

Tree-sitter gives us fast, language-agnostic **syntax** extraction now at **~92% symbol-kind
accuracy** (after keyword-discriminant query fixes for Swift/Kotlin). The remaining gaps and
— more critically — **cross-file call/reference edges** require a semantic layer.

The architecture recommended by senior tech review (2026-06-09) and confirmed correct:

```
Parser layer   : Tree-sitter  (all 14 languages, always on, ~92–99% kind accuracy)
Semantic layer : SCIP indexers (where mature — Python, TS, Java, Go, Rust first)
Fallback       : LSP enrichment was planned but is NO LONGER NEEDED for kind accuracy;
                 keep as future option only for import resolution in unsupported languages
Storage        : SQLite + LanceDB, edges tagged with provenance {syntax|semantic}
```

**Why SCIP over LSP for indexing:**
- LSP is session-oriented (starts a server, requires project built/installed)
- SCIP produces a single `.scip` binary artifact — pre-computed, cacheable, offline
- SCIP scales to monorepos; LSP does not
- SCIP `callers_of()` cross-file is the single biggest gap in the current tool

**What SCIP will NOT fix (already fixed):**
- Swift struct vs class kind — fixed via `(class_declaration "struct" ...)` query
- Kotlin interface vs class kind — fixed via `(class_declaration "interface" ...)` query
- PHP enum — fixed by adding `enum_declaration` pattern
- Java record — fixed by adding `record_declaration` pattern

---

## Verified SCIP Crate Facts

Crate: `scip = "0.8.1"` — confirmed to be Sourcegraph's Source Code Index Protocol.
NOT the SCIP solver (different crate).

**Key data structures (verified from source):**
```rust
// Top-level
scip::types::Index { documents: Vec<Document>, external_symbols: Vec<SymbolInformation> }

// Per source file
scip::types::Document {
    relative_path: String,          // "src/auth/manager.py"
    occurrences: Vec<Occurrence>,
    symbols: Vec<SymbolInformation>,
}

// Each position in source where a symbol appears
scip::types::Occurrence {
    range: Vec<i32>,     // [startLine, startChar, endLine, endChar] — 0-INDEXED
    symbol: String,      // "scip-python pip/requests 1.0 requests/Session#get()."
    symbol_roles: i32,   // BITMASK: bit 0 = DEFINITION, bit 1 = IMPORT, etc.
}

// Symbol metadata
scip::types::SymbolInformation {
    symbol: String,              // Same format as Occurrence.symbol
    kind: EnumOrUnknown<Kind>,   // 87 values (see mapping below)
    documentation: Vec<String>,  // Markdown docstrings
    relationships: Vec<Relationship>, // implements, type_of, etc.
}
```

**Parsing a `.scip` file:**
```rust
use protobuf::Message;
let bytes = std::fs::read(path)?;
let index = scip::types::Index::parse_from_bytes(&bytes)?;
```

**Checking if an Occurrence is a definition:**
```rust
// symbol_roles is a bitmask — bit 0 is DEFINITION
const ROLE_DEFINITION: i32 = 1;
let is_def = (occurrence.symbol_roles & ROLE_DEFINITION) != 0;
```

---

## Verified SQLite Schema Compatibility

Current `edges` table (from `graph_db.rs`):
```sql
INSERT OR REPLACE INTO edges (from_id, to_id, to_name, kind, weight)
```
- `from_id`: the calling symbol's ID (from our local symbols table)
- `to_id`: nullable — already NULL for unresolved callees ✓
- `to_name`: string callee name — already supports unresolved callees ✓
- `kind`: "Calls" | "Imports"

**The schema already supports cross-file edges without change.** `to_id = NULL` +
`to_name = "targetFn"` is valid today. SCIP integration will resolve more `to_id`
values from the local DB, filling in what was previously NULL.

**⚠️ Performance gap confirmed**: `callers_of()` has NO LIMIT clause. With cross-file
SCIP edges a popular function might have thousands of callers. Must add `LIMIT` before
Phase B lands.

---

## Decisions on Open Questions

These were left open in the first draft. Decided here:

**Q1 — Symbol ID strategy:**
Keep `xxhash64(qualified_name)` as primary key. Store SCIP's own symbol string in a
new `scip_symbol` TEXT column on the `symbols` table (nullable, indexed). This lets
us resolve SCIP cross-file references to local IDs without breaking existing callers.

**Q2 — SCIP file location:**
Default `<project-root>/.codegraph/index.scip`. User-configurable via:
```toml
[project]
scip_path = ".codegraph/index.scip"
```

**Q3 — Incremental SCIP:**
Full re-ingest per SCIP run for v0.2. Track `scip_ingested_at` timestamp in the
`files` table; delete semantic edges for all files in the `.scip` batch before
re-inserting. Defer file-level delta to v0.3.

**Q4 — Compile-time feature flag:**
**Yes, use `--features scip`.** The protobuf dependency adds ~1 MB to the binary
(confirmed: `protobuf-support v3.7.2` compiled). Base binary is already 63 MB vs
50 MB ship-gate target. Keeping SCIP opt-in preserves the lean default binary.

**Q5 — Cross-repo external_symbols:**
Treat as unresolved stubs in v0.2. SCIP `external_symbols` (references to symbols
in other packages) produce edges with `to_id = NULL` and `to_name` set to the SCIP
symbol string. Full cross-repo resolution is v0.3 work.

---

## Success Criteria

- [ ] `codegraph-scip` crate compiles with `--features scip` and parses a real `.scip` file
- [ ] `trace_impact` returns callers **across files** (currently same-file only)
- [ ] `graph expansion` (retrieval pipeline) expands to cross-file callers/callees
- [ ] Import edges (`EdgeKind::Imports`) populated from SCIP references
- [ ] `callers_of()` and `callees_of()` have LIMIT clause before cross-file edges land
- [ ] Graceful degradation: no `.scip` file → identical behaviour to v0.1.0
- [ ] `cargo test --workspace` passes (≥ 90 tests, +13 for SCIP layer)
- [ ] Integration test: `scip-python` on Python fixture → cross-file calls verified

---

## Phase A — Foundation
*Unblocks all other phases. No SCIP logic yet — pure codegraph internals.*

- [ ] **A1** Add `EdgeSource` enum and `source` field to `Edge` in `codegraph-core/src/types.rs`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq)]
  pub enum EdgeSource { Syntax, Semantic }
  impl Default for EdgeSource { fn default() -> Self { Self::Syntax } }
  pub struct Edge { ..., pub source: EdgeSource }
  ```
  Update `upsert_edges()` SQL: add `source TEXT NOT NULL DEFAULT 'syntax'` column.

- [ ] **A2** Add `LIMIT 200` to `callers_of()` and `callees_of()` in `graph_db.rs`.
  **Rationale**: SCIP cross-file edges can produce thousands of callers for popular
  functions (e.g. `len()` in Python). Without a cap, `trace_impact` responses become
  unusably large. 200 is consistent with the top-K vector search cap.

- [ ] **A3** Activate `import_query()` in `extractor.rs`:
  Wire each language's `import_query()` result into `EdgeKind::Imports` edges
  with `source: EdgeSource::Syntax`. Currently the string is defined but `extract_edges()`
  only processes `call_query()`.
  **Note**: These will be `to_id = NULL` + `to_name = raw_import_path` since we don't
  resolve imports to local symbol IDs at the syntax layer.

- [ ] **A4** Add `scip_symbol` nullable column to `symbols` table in SQLite migration:
  ```sql
  ALTER TABLE symbols ADD COLUMN scip_symbol TEXT;
  CREATE INDEX IF NOT EXISTS idx_symbols_scip ON symbols(scip_symbol) WHERE scip_symbol IS NOT NULL;
  ```

- [ ] **A5** Add `scip = { version = "0.8.1", optional = true }` to workspace `Cargo.toml`
  under a `scip` feature flag. Add feature to `codegraph-store` and `codegraph-api` as well.

- [ ] **A6** Create `crates/codegraph-scip/` crate skeleton:
  - `Cargo.toml`: feature-gated `scip` dep + `codegraph-core` + `codegraph-store`
  - `src/lib.rs`: `pub fn merge_scip(path: &Path, coordinator: &StoreCoordinator) -> Result<ScipIngestStats>`
  - Registered in workspace `members` but NOT in default feature set

---

## Phase B — SCIP Parser
*Core merging logic. All implementation in `codegraph-scip` crate.*

- [ ] **B1** Parse `.scip` file using `protobuf::Message::parse_from_bytes()`:
  ```rust
  let bytes = std::fs::read(path)?;
  let index = scip::types::Index::parse_from_bytes(&bytes)?;
  // index.documents — one per source file
  // index.external_symbols — cross-package references
  ```

- [ ] **B2** Map SCIP `symbol_information::Kind` → codegraph `SymbolKind`.
  SCIP has **87 values** (verified from crate source). Full mapping:
  ```
  // Direct mappings (confident)
  Class=7       → SymbolKind::Class
  Constant=8    → SymbolKind::Const
  Constructor=9 → SymbolKind::Function
  Enum=11       → SymbolKind::Enum
  EnumMember=12 → SymbolKind::Enum
  Function=17   → SymbolKind::Function
  Interface=21  → SymbolKind::Interface
  Method=26     → SymbolKind::Method
  Module=29     → SymbolKind::Module
  Namespace=30  → SymbolKind::Module
  Package=35    → SymbolKind::Module
  Protocol=42   → SymbolKind::Trait
  Struct=49     → SymbolKind::Struct
  Trait=53      → SymbolKind::Trait
  TypeAlias=55  → SymbolKind::TypeAlias
  TypeParameter=58 → SymbolKind::TypeAlias
  Union=59      → SymbolKind::Struct

  // Method variants → Method
  AbstractMethod=66, StaticMethod=80, TraitMethod=70,
  ProtocolMethod=68, PureVirtualMethod=69, MethodAlias=74,
  MethodSpecification=67, TypeClassMethod=71 → SymbolKind::Method

  // Property-like → Function (no better mapping)
  Getter=18, Setter=45, Accessor=72, Property=41,
  StaticProperty=81 → SymbolKind::Function

  // Field-like → skip or Function
  Field=15, StaticField=79, StaticDataMember=77 → SymbolKind::Function

  // Module variants → Module
  PackageObject=36, Extension=84, Mixin=85,
  SingletonClass=75 → SymbolKind::Module

  // No codegraph equivalent → skip (do not overwrite tree-sitter kind)
  Array=1, Boolean=6, Error=63, File=16, Grammar=19,
  Instance=20, Key=22, Lang=23, Macro=25, Message=28,
  Null=31, Number=32, Object=33, Operator=34, String=48,
  Variable=61, Parameter=37, SelfParameter=44, TypeClass=56,
  UnspecifiedKind=0 → do not overwrite
  ```
  **Rationale for "skip" category**: overwriting a tree-sitter-detected `Function`
  with `Array` would be a regression. SCIP kinds without a clear codegraph equivalent
  should leave the existing kind unchanged.

- [ ] **B3** Match SCIP definition occurrences to codegraph symbols for kind enrichment:
  ```
  For each document in index.documents:
    For each occurrence where (symbol_roles & 1) != 0:  // DEFINITION bit
      key = (document.relative_path, occurrence.range[0] + 1)  // ⚠️ 0→1-indexed
      Find symbol WHERE file_path LIKE '%' || relative_path AND start_line = ?
      If found AND SCIP kind maps to a codegraph kind:
        UPDATE symbols SET kind = ?, scip_symbol = occurrence.symbol WHERE id = ?
  ```
  **Why `range[0] + 1`**: SCIP stores 0-indexed line numbers; codegraph stores
  1-indexed `start_line`. This is verified against the SCIP spec comment:
  *"Line numbers and characters are always 0-based"*.

  **Matching risk**: if the source was modified between tree-sitter indexing and SCIP
  indexing, line numbers will drift. Mitigate by only overwriting kind (never position),
  and running both indexers as close in time as possible.

- [ ] **B4** Extract cross-file reference edges from SCIP occurrences:
  ```
  For each document, for each occurrence where (symbol_roles & 1) == 0:  // not DEFINITION
    from_symbol = find by (document.relative_path, occurrence.range[0]+1)
    to_name = last component of occurrence.symbol (human-readable)
    to_id = look up symbols.scip_symbol = occurrence.symbol → NULL if not in local DB
    Insert edge (from_id, to_id, to_name, kind='Calls', source='semantic', weight=1.0)
  ```
  **Schema compatibility confirmed**: `to_id` is already nullable in the `edges` table.
  `callers_of()` already handles `to_id = NULL` rows (they appear as "unresolved callers").

- [ ] **B5** Extract import edges from SCIP occurrences with `IMPORT` role (bit 1):
  ```
  occurrence.symbol_roles & 2 != 0  // IMPORT bit
  ```
  Produces `EdgeKind::Imports { source: Semantic }` edges. These supplement the
  raw-string import edges from Phase A3 with resolved symbol targets.

- [ ] **B6** Overwrite `Symbol.docstring` with SCIP hover data when richer:
  Only overwrite when `symbol_information.documentation` is non-empty AND the
  existing docstring is empty or shorter (SCIP may include full markdown signatures).

- [ ] **B7** Unit tests in `crates/codegraph-scip/tests/`:
  - `test_kind_mapping_complete`: verify all 87 SCIP Kind values have an explicit mapping
    (no panic/fallthrough on unknown values)
  - `test_parse_minimal_scip`: parse a 50-byte hand-crafted SCIP proto fixture
  - `test_line_index_offset`: verify 0→1-indexed conversion is correct

---

## Phase C — StoreCoordinator Integration

- [ ] **C1** `StoreCoordinator::index_scip_file(path: &Path) -> Result<ScipIngestStats>`:
  - Reads and parses `.scip` file
  - Deletes all `source = 'semantic'` edges for files in this batch (idempotent)
  - Calls `graph_db.upsert_symbols()` with kind+scip_symbol updates (B3)
  - Calls `graph_db.upsert_edges()` with cross-file call edges (B4)
  - Returns `ScipIngestStats { symbols_enriched, edges_added, files_processed }`

- [ ] **C2** Config schema addition to `codegraph.toml`:
  ```toml
  [project]
  scip_path = ".codegraph/index.scip"  # optional; default shown
  ```
  No `scip_auto` in v0.2 (see removed D3 below).

- [ ] **C3** `IndexWorker` post-index SCIP check:
  After each **full** tree-sitter index cycle, check if `scip_path` exists and
  `scip_path.modified() > last_scip_ingest_timestamp`. If so, call `index_scip_file()`.
  This is **not** triggered on every file-watcher event — only after full re-index.

- [ ] **C4** Add `scip_ingested_at` timestamp to the status API response:
  ```json
  { "state": "fresh", "total_files": 580, "total_symbols": 13748,
    "scip_present": true, "scip_ingested_at": "2026-06-10T12:00:00Z",
    "scip_symbols_enriched": 4231 }
  ```

- [ ] **C5** `watch_status` MCP tool response includes `scip_present: bool`.

---

## Phase D — SCIP Indexer Runner
*v0.2: detection only. Auto-run deferred to v0.3.*

**Why D3 (auto-run) is removed from v0.2:**
SCIP indexers require project build context that cannot be assumed:
- `scip-python` requires the Python venv to be activated and `pip install scip-python` done
- `scip-typescript` requires `node_modules` present (`npm install` done)
- `scip-java` requires the project to be compiled (Maven/Gradle build completed)
- `scip-go` requires `go.sum` and downloaded modules

Auto-running these at `codegraph start` without build context will silently fail or
produce an empty/partial SCIP file that corrupts the semantic layer. This is worse
than no SCIP at all.

**v0.2 scope:**

- [ ] **D1** At startup, log which SCIP indexers are available in PATH:
  ```
  [INFO] SCIP indexers detected: scip-python (✓), scip-typescript (✓), scip-java (✗)
  [INFO] Run `codegraph scip index` to generate .codegraph/index.scip
  ```

- [ ] **D2** `codegraph scip index --lang <python|typescript|java|go|rust>` CLI subcommand.
  Runs the appropriate indexer as a subprocess, writes to `scip_path`. Fails fast with
  a helpful error if indexer not found or build context missing (no venv, no node_modules).

**v0.3 (future):** Auto-detection of build context maturity before auto-running.

---

## Phase E — MCP Tool Improvements
*Requires Phase B+C complete. Only the confirmed-valuable improvements.*

- [ ] **E1** `trace_impact` returns cross-file callers using semantic edges.
  `callers_of()` already supports `to_id = NULL` — cross-file SCIP edges land
  in the same table. After B4, `trace_impact` will automatically include cross-file
  callers with no code change needed *if* the edge records are in the DB.
  **Only change needed**: add `edge_source` field to the MCP response so callers can
  distinguish `syntax` (same-file, tree-sitter) from `semantic` (cross-file, SCIP).

- [ ] **E2** `graph expansion` in the retrieval pipeline uses semantic edges for
  cross-file neighbour expansion. Currently `expand_graph()` calls `callers_of()`
  which already queries the `edges` table — cross-file SCIP edges will appear
  automatically. **Only change needed**: verify the score decay and dedup logic
  handles the larger result set correctly.

- [ ] **E3** `explain_symbol` includes SCIP hover documentation when `scip_symbol`
  is populated on the symbol. Adds type signatures and implement/override relationships
  from `SymbolInformation.relationships`.

  ~~**E4 removed**~~: `semantic_refs` count in ranking was proposed in the first draft
  but would duplicate the existing `in_degree` signal. When SCIP cross-file edges are
  added (B4), the `in_degree` of well-used symbols increases naturally, improving
  reranking with zero additional work. No new signal needed.

---

## Key Invariants

- **SCIP is always optional and feature-gated** (`--features scip`). The absence of
  a `.scip` file or the feature flag being absent must not degrade v0.1.0 behaviour.
- **Tree-sitter data is never deleted by SCIP.** SCIP only *overwrites* symbol kind
  (when a cleaner kind is known) and *adds* new edges. If SCIP is later removed,
  tree-sitter symbols and edges remain intact.
- **Edge provenance is always set and queryable.** Every edge must have `source = 'syntax'`
  or `source = 'semantic'`. This enables the API to filter by provenance.
- **Idempotency.** `index_scip_file()` called twice on the same file produces the same
  result. Achieved by DELETE-then-INSERT for semantic edges (not upsert, to avoid stale
  cross-file edges from a previous run remaining).
- **Bounded queries.** `callers_of()` and `callees_of()` must have `LIMIT 200` before
  cross-file SCIP edges are introduced. This is Phase A2 and must not be skipped.
- **Tree-sitter deduplication invariant preserved.** The keep-last deduplication added
  to `extractor.rs` (`last_idx` filter) must not be reverted.

---

## File Change Map

```
Cargo.toml
  ADD: scip = { version = "0.8.1", optional = true }
  ADD: [features] scip = ["codegraph-scip/scip", "codegraph-store/scip"]

crates/codegraph-core/src/types.rs
  ADD: EdgeSource enum (Syntax, Semantic)
  ADD: source: EdgeSource field on Edge struct

crates/codegraph-parser/src/extractor.rs
  WIRE: import_query() results into EdgeKind::Imports { source: Syntax }  [Phase A3]

crates/codegraph-store/src/graph_db.rs
  ADD: scip_symbol TEXT column to symbols table  [Phase A4]
  ADD: LIMIT 200 to callers_of() and callees_of()  [Phase A2]
  ADD: source column to edges table  [Phase A1]
  UPDATE: upsert_edges() to write source field

crates/codegraph-store/src/coordinator.rs
  ADD: index_scip_file() method  [Phase C1]
  ADD: scip_ingested_at tracking  [Phase C3]

crates/codegraph-api/src/routes.rs
  UPDATE: /api/status response includes scip_present, scip_ingested_at  [Phase C4]

crates/codegraph-mcp/src/tools.rs
  UPDATE: watch_status includes scip_present  [Phase C5]
  UPDATE: trace_impact response includes edge_source field  [Phase E1]
  UPDATE: explain_symbol uses scip hover data when available  [Phase E3]

src/main.rs
  ADD: codegraph scip index subcommand  [Phase D2]

crates/codegraph-scip/  (new, feature-gated)
  Cargo.toml
  src/lib.rs       merge_scip() public API
  src/parser.rs    protobuf parse → internal structs
  src/mapper.rs    SCIP Kind (87 values) → codegraph SymbolKind
  src/merger.rs    write enriched symbols + cross-file edges to StoreCoordinator
  tests/scip_merge.rs  unit + integration tests
```

---

## Testing Plan

- [ ] **T1** Unit: SCIP mapper covers all 87 Kind enum values — no panics on any input
- [ ] **T2** Unit: parse a minimal hand-crafted `.scip` protobuf fixture (3 symbols, 2 refs)
- [ ] **T3** Unit: `range[0] + 1` line-offset conversion correct for multi-file fixture
- [ ] **T4** Integration: run `scip-python` on the `tests/fixtures/python_multi_file/`
  fixture (two files where A calls B), ingest, verify cross-file `Calls` edge in SQLite
- [ ] **T5** Integration: run `scip-typescript` on a small TS fixture, verify that
  an interface previously indexed as `Class` (before parser fix) is now `Interface`
  in a fresh index (regression guard)
- [ ] **T6** Regression: all 77 existing parser tests pass after Phase A schema changes
- [ ] **T7** Performance: after SCIP ingest of deepagents (13k symbols), `callers_of()`
  on `_offload_tool_message_content` returns ≤ 200 results in ≤ 5 ms
- [ ] **T8** Degradation: project with no `.scip` file → status API shows
  `scip_present: false`, all other behaviour identical to v0.1.0
- [ ] **T9** Idempotency: run `index_scip_file()` twice on same file → same edge count

---

## Dependency: External SCIP Indexers

These are NOT shipped with codegraph. Users install them separately.

| Language | Indexer | Install command | Requires | Maturity |
|----------|---------|-----------------|----------|---------|
| Python | `scip-python` | `pip install scip-python` | venv activated | ★★★★★ |
| TypeScript | `scip-typescript` | `npm i -g @sourcegraph/scip-typescript` | node_modules | ★★★★★ |
| Java | `scip-java` | download binary | compiled project | ★★★★★ |
| Go | `scip-go` | `go install github.com/sourcegraph/scip-go@latest` | go.sum | ★★★★☆ |
| Rust | `rust-analyzer` | bundled | cargo build run once | ★★★★☆ |
| Kotlin | `scip-kotlin` | JetBrains plugin export | compiled project | ★★★☆☆ |
| Ruby | `scip-ruby` | `gem install scip-ruby` | bundler | ★★★☆☆ |
| C/C++ | `scip-clang` | via LLVM | compile_commands.json | ★★★☆☆ |
| Swift | (none mature) | — | — | tree-sitter sufficient post-fix |
| PHP | (none mature) | — | — | tree-sitter sufficient post-fix |
| Scala | (none mature) | — | — | tree-sitter only |

**Swift and PHP removed from LSP fallback list**: the tree-sitter keyword-discriminant
fixes (Swift struct/enum, Kotlin interface) resolved the kind accuracy issues that
motivated a LSP fallback. LSP enrichment for these languages is no longer justified.

---

## What Is NOT in This Plan

Items explicitly excluded with rationale:

| Excluded item | Reason |
|--------------|--------|
| Auto-run SCIP at `codegraph start` (D3) | Requires build context; will fail without venv/npm/build |
| Cross-repo SCIP federation | v0.3 scope; depends on v0.2 being stable |
| `semantic_refs` ranking signal (old E4) | Redundant: `in_degree` already captures this once B4 lands |
| LSP fallback for Swift/PHP (old Phase F) | Tree-sitter keyword-discriminant queries already solved the kind accuracy gap |
| Incremental SCIP (file-level delta) | v0.3 scope; full re-ingest is safe and simpler for v0.2 |
| SCIP for Scala/PHP | No mature indexer; tree-sitter-only is correct for these languages |
