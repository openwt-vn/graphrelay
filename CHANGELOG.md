# Changelog

All notable changes are recorded here. Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [SemVer](https://semver.org) (pre-1.0: minor versions may break).

## [0.1.0]

First public release.

### Added
- GraphRelay: graph-aware code search for AI coding agents. A single local binary (`gr`) serves MCP, REST and a web UI on one port, using fused lexical (SQLite FTS5/BM25) and graph retrieval with personalized PageRank and reranking. See `docs/how-it-works.md`.
- Four MCP tools: `search_code`, `inspect`, `trace_impact`, `watch_status`.
- 30 tree-sitter languages, incremental live indexing, optional SCIP enrichment.
- Release archives named `graphrelay-<tag>-<target>.tar.gz` (macOS arm64, Linux arm64, Linux x86_64).
- `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `THIRD_PARTY/NOTICE.md`.
