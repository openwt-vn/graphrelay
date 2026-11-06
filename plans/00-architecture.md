# Plan 00 — System Architecture

## Goal
Define the component boundaries, data flow, and crate structure for the `codegraph` binary before writing any production code.

## Success Criteria
- Every component maps to exactly one crate or module
- No circular dependencies between crates
- A single `cargo build --release` produces one statically-linked binary
- All inter-component contracts (traits, types) are defined before implementation begins

---

## 1. Process Model

```
┌─────────────────────────────────────────────────────────────┐
│                   codegraph (single process)                │
│                                                             │
│  ┌───────────┐   ┌───────────┐   ┌───────────────────────┐ │
│  │  Watcher  │──▶│  Indexer  │──▶│  Graph + Vector Store │ │
│  │(notify-rs)│   │(tree-sit) │   │  (SQLite + LanceDB)   │ │
│  └───────────┘   └───────────┘   └───────────────────────┘ │
│                                           │                 │
│  ┌────────────────────────────────────────▼──────────────┐  │
│  │                  Query Engine                         │  │
│  │   vec-search → graph-expand → rerank                 │  │
│  └──────────────┬────────────────────────────────────────┘  │
│                 │                                           │
│  ┌──────────────▼──────────────────────────────────────┐   │
│  │         HTTP Router  (axum, port 3030)              │   │
│  │   /mcp  │  /api/*  │  / (embedded UI assets)        │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Crate / Module Layout

```
codegraph/                          (Cargo workspace root)
├── Cargo.toml                      (workspace manifest)
├── crates/
│   ├── codegraph-core/             (pure logic, no I/O)
│   │   ├── src/
│   │   │   ├── graph/              (symbol graph types, edges)
│   │   │   ├── chunk/              (graph-aware chunk builder)
│   │   │   ├── retrieval/          (two-stage retrieval pipeline)
│   │   │   └── types.rs            (shared domain types)
│   │   └── Cargo.toml
│   │
│   ├── codegraph-parser/           (tree-sitter extraction)
│   │   ├── src/
│   │   │   ├── language/           (per-language grammar wrappers)
│   │   │   ├── extractor.rs        (symbol/call/import visitor)
│   │   │   └── incremental.rs      (diff-only re-parse)
│   │   └── Cargo.toml
│   │
│   ├── codegraph-store/            (SQLite + LanceDB I/O)
│   │   ├── src/
│   │   │   ├── graph_db.rs         (SQLite: symbols, edges, files)
│   │   │   ├── vector_db.rs        (LanceDB: embeddings table)
│   │   │   ├── migrations/         (SQL migration files embedded)
│   │   │   └── transaction.rs      (coordinated dual-write)
│   │   └── Cargo.toml
│   │
│   ├── codegraph-embed/            (embedding model interface)
│   │   ├── src/
│   │   │   ├── model.rs            (EmbeddingModel trait)
│   │   │   ├── bge_small.rs        (bge-small-en-v1.5 via ort)
│   │   │   └── batch.rs            (batched inference with queue)
│   │   └── Cargo.toml
│   │
│   ├── codegraph-watcher/          (file system watching)
│   │   ├── src/
│   │   │   ├── watcher.rs          (notify-rs wrapper + debounce)
│   │   │   ├── queue.rs            (bounded change queue)
│   │   │   └── status.rs           (IndexStatus for watch_status())
│   │   └── Cargo.toml
│   │
│   ├── codegraph-mcp/              (MCP HTTP server)
│   │   ├── src/
│   │   │   ├── server.rs           (axum router for /mcp)
│   │   │   ├── tools/              (one file per MCP tool)
│   │   │   └── schema.rs           (JSON-RPC types)
│   │   └── Cargo.toml
│   │
│   ├── codegraph-api/              (REST API for UI)
│   │   ├── src/
│   │   │   ├── routes/
│   │   │   └── models.rs
│   │   └── Cargo.toml
│   │
│   └── codegraph-ui/               (embedded static assets)
│       ├── assets/                 (HTML/JS/CSS compiled in)
│       └── Cargo.toml
│
└── src/
    └── main.rs                     (binary: wires all crates, starts axum)
```

---

## 3. Key Shared Types (`codegraph-core/src/types.rs`)

```rust
pub type SymbolId = u64;
pub type FileId   = u64;

pub struct Symbol {
    pub id:        SymbolId,
    pub file_id:   FileId,
    pub name:      String,
    pub kind:      SymbolKind,      // Function | Class | Struct | Enum | Trait | Const | ...
    pub span:      Span,            // (start_byte, end_byte, start_line, end_line)
    pub docstring: Option<String>,
    pub language:  Language,
}

pub struct Edge {
    pub from:  SymbolId,
    pub to:    SymbolId,
    pub kind:  EdgeKind,            // Calls | Imports | References | Inherits | Implements
}

pub struct Chunk {
    pub symbol_id:    SymbolId,
    pub text:         String,       // symbol body + neighbors as context prefix
    pub embedding:    Option<Vec<f32>>,
    pub graph_meta:   GraphMeta,    // caller_names, callee_names, file_path, symbol_path
}

pub struct RetrievalResult {
    pub symbol:    Symbol,
    pub score:     f32,             // fused rank score
    pub callers:   Vec<Symbol>,
    pub callees:   Vec<Symbol>,
    pub file_path: PathBuf,
    pub snippet:   String,
}
```

---

## 4. Data Flow — Query Path

```
User query (natural language)
        │
        ▼
  embed(query) → f32[384]
        │
        ▼
  LanceDB ANN search  →  top-K chunks (default K=20)
        │
        ▼
  Resolve chunk → SymbolId[]
        │
        ▼
  SQLite: pull callers + callees for each SymbolId  (graph expansion)
        │
        ▼
  Score = α·vec_score + β·graph_centrality + γ·recency_bonus
        │
        ▼
  Rerank → top-N results (default N=5)
        │
        ▼
  Serialize to RetrievalResult[]  →  MCP tool response / REST response
```

---

## 5. Data Flow — Index Path

```
File change event (notify-rs)
        │
        ▼
  Debounce (50 ms window)
        │
        ▼
  tree-sitter parse (incremental edit if possible)
        │
        ▼
  Extract symbols + edges (diff vs. previous snapshot)
        │
        ▼
  SQLite: upsert symbols, delete stale edges, insert new edges
        │
        ▼
  Build graph-aware chunks for changed symbols
        │
        ▼
  Batch embed (up to 32 chunks per ort inference call)
        │
        ▼
  LanceDB: delete old vectors by symbol_id, insert new
        │
        ▼
  Emit IndexStatus(fresh, last_updated_at)
```

---

## 6. HTTP Layer (axum)

All three surfaces share one `TcpListener` on `:3030`:

```
GET  /              → serve embedded UI index.html
GET  /assets/*      → serve embedded JS/CSS
POST /mcp           → MCP JSON-RPC handler
GET  /api/graph     → symbol graph as JSON
GET  /api/search    → REST search endpoint
GET  /api/symbol/:id → symbol + neighbors
GET  /api/status    → index freshness
```

---

## 7. Dependency Graph (crate level)

```
main
 ├── codegraph-mcp       → codegraph-core, codegraph-store
 ├── codegraph-api       → codegraph-core, codegraph-store
 ├── codegraph-watcher   → codegraph-parser, codegraph-store, codegraph-embed
 ├── codegraph-parser    → codegraph-core
 ├── codegraph-embed     → codegraph-core
 ├── codegraph-store     → codegraph-core
 └── codegraph-ui        (no internal deps — pure assets)
```

No cycles. `codegraph-core` has zero internal deps.

---

## 8. Configuration (`codegraph.toml`)

```toml
[server]
port = 3030
bind = "127.0.0.1"

[index]
root = "."                        # workspace root to index
ignore = [".git", "node_modules", "target", "dist"]
languages = ["rust", "typescript", "python", "go"]  # subset of supported 14

[embedding]
model = "bge-small-en-v1.5"       # | "bge-code-v1" | "voyage-code-3"
model_path = ""                   # empty = use bundled ONNX weights
batch_size = 32
device = "cpu"                    # | "cuda" (future)

[retrieval]
vector_top_k   = 20
expand_hops    = 1                # graph hops after vector stage
final_top_n    = 5
alpha          = 0.7              # weight: vector score
beta           = 0.2              # weight: graph centrality
gamma          = 0.1              # weight: recency bonus

[watcher]
debounce_ms = 50
max_queue   = 1000
```

---

## 9. Static Binary Strategy

- Compile tree-sitter grammars as `.c` sources via `build.rs` (no dynamic linking)
- Bundle ONNX model weights via `include_bytes!` or a pre-download step that writes to `~/.codegraph/models/`
- Bundle UI assets via `rust-embed` crate (`#[derive(RustEmbed)]`)
- LanceDB uses its embedded C++ backend; link statically
- Target: `x86_64-unknown-linux-musl` (Linux), universal binary (macOS)

---

## 10. Milestones Sequence

| # | Plan | Depends on |
|---|------|------------|
| 01 | Project setup & workspace | — |
| 02 | Tree-sitter parser | 01 |
| 03 | Storage backend | 01, 02 |
| 04 | Embedding layer | 01 |
| 05 | Hybrid retrieval | 02, 03, 04 |
| 06 | MCP server | 05 |
| 07 | File watcher | 02, 03, 04 |
| 08 | REST API | 05 |
| 09 | Web UI | 08 |
| 10 | Benchmarking | 06, 07, 08 |
