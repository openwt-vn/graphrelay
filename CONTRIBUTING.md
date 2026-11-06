# Contributing to GraphRelay

Thanks for helping. Bug reports, fixes, new language support and docs improvements are all welcome.

## Getting set up

```sh
git clone https://github.com/openwt-vn/graphrelay graphrelay
cd graphrelay
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Requires Rust 1.78+. CI runs the same test and clippy commands.

## Before you open a pull request

1. Open an issue first for anything larger than a small fix, so we can agree on the approach.
2. Keep the change focused. One logical change per pull request.
3. Add or update tests. Retrieval changes need evidence, not an anecdote: say what you measured and on which corpus (see [docs/benchmarks.md](docs/benchmarks.md)).
4. `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` must pass.
5. Update the docs if you change user-visible behaviour (CLI, config, MCP tools).

## Project rules worth knowing

- Crate responsibilities and the invariants that must not break (write ordering between SQLite and the FTS5 index, stable symbol IDs, exactly four MCP tools, rerank weights) are documented in [CLAUDE.md](CLAUDE.md). Despite the name it is a good architecture overview for people too.
- Code style: no comments unless the *why* is non-obvious; `anyhow` in application code, `thiserror` in libraries; async only at the boundaries; `graphrelay-core` stays free of I/O and `tokio`.
- Adding a language needs a tree-sitter grammar with ABI 14 (check `LANGUAGE_VERSION` in its `src/parser.c`), a `LanguageSupport` implementation and a registry entry. See [docs/languages.md](docs/languages.md).

## Commits and pull requests

Write clear commit messages that say why, not only what. By contributing you agree that your work is licensed under the [MIT License](LICENSE).

## Conduct and security

Be kind: see the [Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md), not in public issues.
