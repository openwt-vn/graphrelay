# Plan 01 — Project Setup & Workspace

## Goal
Bootstrap the Rust workspace, configure all crates and their dependencies, and establish CI so every subsequent plan starts from a compiling baseline.

## Success Criteria
- `cargo build --release` produces a single binary at `target/release/codegraph`
- `cargo test --workspace` passes (even if test bodies are empty stubs)
- `cargo clippy --workspace -- -D warnings` is clean
- GitHub Actions CI runs on every push

---

## 1. Workspace `Cargo.toml`

```toml
[workspace]
resolver = "2"
members = [
    "crates/codegraph-core",
    "crates/codegraph-parser",
    "crates/codegraph-store",
    "crates/codegraph-embed",
    "crates/codegraph-watcher",
    "crates/codegraph-mcp",
    "crates/codegraph-api",
    "crates/codegraph-ui",
]
default-members = ["."]

[workspace.package]
version     = "0.1.0"
edition     = "2021"
rust-version = "1.78"
license     = "MIT OR Apache-2.0"
authors     = ["codegraph contributors"]

[workspace.dependencies]
# Async runtime
tokio          = { version = "1", features = ["full"] }
# HTTP server
axum           = { version = "0.7", features = ["macros"] }
tower          = "0.4"
tower-http     = { version = "0.5", features = ["cors", "fs"] }
# Serialization
serde          = { version = "1", features = ["derive"] }
serde_json     = "1"
# Error handling
anyhow         = "1"
thiserror      = "1"
# Logging
tracing        = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
# Tree-sitter
tree-sitter    = "0.22"
# SQLite
rusqlite       = { version = "0.31", features = ["bundled"] }
# LanceDB
lancedb        = "0.8"
arrow-array    = "51"
arrow-schema   = "51"
# ONNX Runtime (embeddings)
ort            = { version = "2", features = ["download-binaries"] }
# File watching
notify         = "6"
# Config
toml           = "0.8"
# Static asset embedding
rust-embed     = { version = "8", features = ["include-exclude"] }
# CLI args
clap           = { version = "4", features = ["derive"] }
# Utils
uuid           = { version = "1", features = ["v4"] }
chrono         = { version = "0.4", features = ["serde"] }
dashmap        = "5"
crossbeam-channel = "0.5"
rayon          = "1"
```

---

## 2. Root Binary `Cargo.toml`

```toml
[package]
name    = "codegraph"
version.workspace = true
edition.workspace = true

[[bin]]
name = "codegraph"
path = "src/main.rs"

[dependencies]
codegraph-core    = { path = "crates/codegraph-core" }
codegraph-parser  = { path = "crates/codegraph-parser" }
codegraph-store   = { path = "crates/codegraph-store" }
codegraph-embed   = { path = "crates/codegraph-embed" }
codegraph-watcher = { path = "crates/codegraph-watcher" }
codegraph-mcp     = { path = "crates/codegraph-mcp" }
codegraph-api     = { path = "crates/codegraph-api" }
codegraph-ui      = { path = "crates/codegraph-ui" }

tokio.workspace      = true
axum.workspace       = true
tower-http.workspace = true
tracing.workspace    = true
tracing-subscriber.workspace = true
clap.workspace       = true
anyhow.workspace     = true
toml.workspace       = true
serde.workspace      = true
```

---

## 3. `src/main.rs` Skeleton

```rust
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "codegraph", about = "Local code graph & semantic search MCP server")]
struct Cli {
    #[arg(long, default_value = "codegraph.toml")]
    config: std::path::PathBuf,

    #[arg(long, default_value = "3030")]
    port: u16,

    /// Index the workspace root and exit (no server)
    #[arg(long)]
    index_only: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = codegraph_core::Config::load(&cli.config)?;

    // 1. Open / create stores
    // 2. Run initial index
    // 3. Start file watcher
    // 4. Start HTTP server
    // 5. Wait for shutdown signal

    Ok(())
}
```

---

## 4. Per-Crate `Cargo.toml` Templates

### `crates/codegraph-core/Cargo.toml`
```toml
[package]
name    = "codegraph-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace    = true
thiserror.workspace = true
anyhow.workspace   = true
toml.workspace     = true
```

### `crates/codegraph-parser/Cargo.toml`
```toml
[package]
name    = "codegraph-parser"
version.workspace = true
edition.workspace = true

[dependencies]
codegraph-core = { path = "../codegraph-core" }
tree-sitter.workspace = true
anyhow.workspace      = true
rayon.workspace       = true

[build-dependencies]
cc = "1"    # compiles grammar .c files
```

### `crates/codegraph-store/Cargo.toml`
```toml
[package]
name = "codegraph-store"
version.workspace = true
edition.workspace = true

[dependencies]
codegraph-core = { path = "../codegraph-core" }
rusqlite.workspace   = true
lancedb.workspace    = true
arrow-array.workspace = true
arrow-schema.workspace = true
tokio.workspace      = true
anyhow.workspace     = true
tracing.workspace    = true
```

### `crates/codegraph-embed/Cargo.toml`
```toml
[package]
name = "codegraph-embed"
version.workspace = true
edition.workspace = true

[dependencies]
codegraph-core = { path = "../codegraph-core" }
ort.workspace    = true
anyhow.workspace = true
tracing.workspace = true
tokenizers = "0.19"   # HuggingFace tokenizers
ndarray    = "0.15"
```

### `crates/codegraph-watcher/Cargo.toml`
```toml
[package]
name = "codegraph-watcher"
version.workspace = true
edition.workspace = true

[dependencies]
codegraph-core    = { path = "../codegraph-core" }
codegraph-parser  = { path = "../codegraph-parser" }
codegraph-store   = { path = "../codegraph-store" }
codegraph-embed   = { path = "../codegraph-embed" }
notify.workspace           = true
crossbeam-channel.workspace = true
tokio.workspace            = true
tracing.workspace          = true
anyhow.workspace           = true
dashmap.workspace          = true
```

### `crates/codegraph-mcp/Cargo.toml`
```toml
[package]
name = "codegraph-mcp"
version.workspace = true
edition.workspace = true

[dependencies]
codegraph-core  = { path = "../codegraph-core" }
codegraph-store = { path = "../codegraph-store" }
axum.workspace         = true
serde.workspace        = true
serde_json.workspace   = true
anyhow.workspace       = true
tracing.workspace      = true
tokio.workspace        = true
```

### `crates/codegraph-api/Cargo.toml`
```toml
[package]
name = "codegraph-api"
version.workspace = true
edition.workspace = true

[dependencies]
codegraph-core  = { path = "../codegraph-core" }
codegraph-store = { path = "../codegraph-store" }
axum.workspace       = true
tower-http.workspace = true
serde.workspace      = true
serde_json.workspace = true
anyhow.workspace     = true
tracing.workspace    = true
```

### `crates/codegraph-ui/Cargo.toml`
```toml
[package]
name = "codegraph-ui"
version.workspace = true
edition.workspace = true

[dependencies]
rust-embed.workspace = true
axum.workspace       = true
```

---

## 5. `.gitignore`

```
/target
.codegraph/          # runtime data (SQLite, LanceDB, model cache)
codegraph.toml       # user-local config (committed as codegraph.toml.example)
*.onnx               # model weights — too large for git
```

---

## 6. CI — `.github/workflows/ci.yml`

```yaml
name: CI
on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy --workspace -- -D warnings
      - run: cargo test --workspace
      - run: cargo build --release
```

---

## 7. Directory Creation Script

```bash
#!/usr/bin/env bash
set -e

CRATES=(codegraph-core codegraph-parser codegraph-store codegraph-embed codegraph-watcher codegraph-mcp codegraph-api codegraph-ui)

for c in "${CRATES[@]}"; do
  mkdir -p "crates/$c/src"
  cat > "crates/$c/Cargo.toml" << EOF
[package]
name    = "$c"
version.workspace = true
edition.workspace = true
EOF
  echo "pub fn placeholder() {}" > "crates/$c/src/lib.rs"
done

mkdir -p src
echo 'fn main() {}' > src/main.rs
```

---

## 8. Implementation Steps

1. Run directory creation script
2. Write root `Cargo.toml` (workspace manifest)
3. Write per-crate `Cargo.toml` files (use templates above)
4. Write `src/main.rs` skeleton
5. Verify: `cargo build --workspace` (all crates compile as stubs)
6. Add `.github/workflows/ci.yml`
7. Add `.gitignore`
8. First commit: `chore: init workspace with crate stubs`

---

## 9. Open Decisions

| Decision | Options | Recommendation |
|----------|---------|----------------|
| LanceDB Rust version | 0.7 vs 0.8 | Use latest (0.8) — API more stable |
| ONNX provider | `ort` vs `candle` | `ort` — wider model support, proven in prod |
| Tokenizer crate | `tokenizers` vs `tiktoken-rs` | `tokenizers` — same library HuggingFace uses |
| Config format | TOML vs YAML | TOML — Cargo ecosystem standard |
| async runtime | tokio vs async-std | tokio — required by axum and LanceDB |
