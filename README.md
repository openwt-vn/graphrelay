<img src="docs/brand/hero.svg" alt="GraphRelay — graph-aware code search for AI coding agents, one binary, fully local">

[![License: MIT](https://img.shields.io/badge/license-MIT-111111)](LICENSE)
[![CI](https://github.com/openwt-vn/graphrelay/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/openwt-vn/graphrelay/actions/workflows/ci.yml)
[![Platforms](https://img.shields.io/badge/platform-macOS%20Apple%20Silicon%20%7C%20Linux%20x86__64%20%7C%20Linux%20ARM64-111111)](#status)

# GraphRelay

GraphRelay indexes your codebase into a symbol and call graph and serves graph-aware code search to AI coding agents over [MCP](https://modelcontextprotocol.io), REST and a small web UI. It is one Rust binary (`gr`): no Python, no Node, no database server, no cloud account, no API key. Your code never leaves your machine.

Search fuses SQLite FTS5/BM25 lexical matching with graph expansion (callers and callees) and query-seeded personalized PageRank in a single ranked list. There are no embeddings; see [how retrieval works](docs/how-it-works.md) for what that does and does not give you.

## Does it help? An honest status

We measure one thing: whether a coding agent answers hard questions about a large codebase (ERPNext, ~1M LOC) better with GraphRelay's MCP tools than with Grep/Read/Glob alone. The base reference is the latest full run: Sonnet 5.5, 30 questions × 3 repeats, four arms. **No arm was reliably better than the no-tool baseline:**

| Mean key-fact coverage (Sonnet 5.5, 30 questions × 3 repeats) | Coverage | vs baseline (95% CI) | Tool calls / question | $ / question |
|---|---|---|---|---|
| baseline (no MCP) | 70.6% | | 5.6 | 0.076 |
| **GraphRelay** | 71.2% | +0.6 (−4.0…+5.0) | 4.0 | 0.099 |
| graphify | 66.9% | −3.7 (−8.6…+0.7) | 6.2 | 0.091 |
| codegraph-colby (colbymchenry/codegraph, an unrelated project) | 67.1% | −3.5 (−8.5…+1.3) | 4.6 | 0.122 |

The same four arms on GLM 5.3 Flash (released v0.1.0 binary, 30 questions × 2 repeats) point the same way, with GraphRelay nominally ahead and its interval still spanning zero:

| Mean key-fact coverage (GLM 5.3 Flash, 30 questions × 2 repeats) | Coverage | vs baseline (95% CI) | Tool calls / question | $ / question |
|---|---|---|---|---|
| baseline (no MCP) | 76.6% | | 22.9 | 0.499 |
| **GraphRelay** | 79.9% | +3.3 (−1.0…+8.0) | 18.7 | 0.533 |
| graphify | 77.5% | +0.9 (−3.8…+6.3) | 21.0 | 0.478 |
| codegraph-colby (colbymchenry/codegraph, an unrelated project) | 77.1% | +0.5 (−6.0…+7.4) | 13.9 | 0.499 |

The baseline hit the turn cap on 2 of 60 answers and GraphRelay on none, so part of that gap is reaching an answer within the budget rather than answering better. Per-question tool-flow reports: [repeat 1](docs/benchmark-report-glm53flash-4-arms-rep1.html) · [repeat 2](docs/benchmark-report-glm53flash-4-arms-rep2.html).

Per-question tool-flow reports for each repeat, with every tool call and the judge's verdict per key fact: [repeat 1](docs/benchmark-report-sonnet55-4-arms-rep1.html) · [repeat 2](docs/benchmark-report-sonnet55-4-arms-rep2.html) · [repeat 3](docs/benchmark-report-sonnet55-4-arms-rep3.html) (open in a browser; repeats score 69.7 / 72.0 / 71.7% for GraphRelay and 71.6 / 69.6 / 70.6% for the baseline, so one repeat alone can point either way). Builds since this run (decorated-Python merge, string-dispatch and self-call edges, v0.1.0) all tie with it, 69.0% to 70.5%. An earlier single-repeat run with all six arms, including the two judge arms, is [here](docs/benchmark-report-sonnet55-6-arms.html); an earlier single GLM 5.3 Flash repeat gave 77.6% (baseline) vs 80.2% (GraphRelay, CI −1.4…+6.6).

GraphRelay ties the baseline on coverage (better on 12 questions, worse on 14), uses about 30% fewer tool calls and costs about 30% more per question. Broken down by question type there is no consistent pattern (the caller/impact group shows no lift; see [docs/benchmarks.md](docs/benchmarks.md#where-it-might-help-exploratory-not-a-claim)). What we can state without a benchmark is the design: your code stays on your machine, there is nothing to sign up for, and `trace_impact` returns the callers of a function from a call graph (on 12-question checks against independent keys it recovered every caller: 91% precision on ERPNext and 75% on Gitea, where every extra result was a test caller the key excludes by rule). Whether that helps an agent answer better is not shown: a preregistered test on caller/impact questions in ERPNext and Gitea (Go) was a tie: the baseline already scored 96% to 99%, and the differences (+0.4 to +4.4 points, one of them driven by a single baseline failure) all had intervals touching zero. In those runs the agents used fewer tool calls and spent somewhat less with the tool; the 30-question runs show fewer tool calls but a higher cost ([ROADMAP.md](ROADMAP.md)). The project is early. Method, caveats and raw data: [docs/benchmarks.md](docs/benchmarks.md).

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/openwt-vn/graphrelay/main/scripts/install.sh | sh
```

> **Note:** the installer downloads release archives from GitHub; if you hit API rate limits, pass `--token=…` or set `GITHUB_TOKEN` (a `gh` login also works). From a local checkout, run `sh scripts/install.sh`; pin a release with `--version=v0.1.0`. Releases ship `graphrelay-*` archives (macOS arm64, Linux arm64, Linux x86_64). No Intel macOS build yet: [build from source](#build-from-source).

The installer downloads the release archive for your platform, verifies its sha256 checksum and puts `gr` in `~/.local/bin`. Read [scripts/install.sh](scripts/install.sh) first if you prefer, or [build from source](#build-from-source).

## Quick start

```sh
cd /path/to/your-project
gr init      # register the project
gr start     # start the daemon and index
gr setup     # wire up Claude Code, Cursor, OpenCode, Amp or Codex
```

Open `http://localhost:3030/{project}/` for the web UI: a search box, an interactive call graph (Cytoscape) with a language filter, and an inspector that shows a symbol's source, callers and callees. It is useful for people exploring a codebase, not only for agents.

Your agent now has four MCP tools. More detail: [docs/setup-guide.md](docs/setup-guide.md).

## MCP tools

| Tool | What it does |
|---|---|
| `search_code(intent?, implements?, top_n?)` | Fused lexical+graph retrieval. `intent` for natural-language search; `implements` to find everything implementing or inheriting a named interface, trait or base class. Exactly one is required. |
| `inspect(symbol?, path?, focus_line?, include_source?, max_symbols?)` | `symbol`: definition, callers, callees, docstring. `path`: file context, or a module summary if it resolves to a directory. Exactly one of `symbol`/`path` is required. |
| `trace_impact(symbol, depth?)` | Blast radius: what breaks if this changes? |
| `watch_status()` | Index freshness, so agents can detect stale results. |

Any MCP client can use `POST http://localhost:3030/{project}/mcp` (JSON-RPC 2.0 over HTTP). `{project}` is the project directory name.

## CLI

| Command | Description |
|---|---|
| `gr init` / `gr unregister` | Register / remove the current directory as a project |
| `gr start` / `gr stop` / `gr restart` | Control the daemon |
| `gr status` | Daemon state and per-project file/symbol counts |
| `gr index` | Force a full re-index (daemon must be stopped) |
| `gr setup` | Configure MCP server entry and instruction hook for supported tools |
| `gr scip` | Run an optional SCIP indexer for compiler-backed cross-file edges |
| `gr update` | Download and install the latest release |
| `gr bench`, `gr compare` | Developer self-checks on retrieval quality |

## Configuration

`gr init` writes `.graphrelay/config.toml` into your project with every option commented out; the defaults work. Examples:

```toml
# [index]
# ignore = [".git/**", "node_modules/**", "target/**"]

# [retrieval]
# final_top_n = 5
```

Environment: `GRAPHRELAY_DATA_DIR` (data location, default `~/.graphrelay`), `RUST_LOG`. Optional relevance judges (Jev, Laya) are off by default: [docs/relevance-judges.md](docs/relevance-judges.md).

## Languages

31 languages via tree-sitter, including Rust, TypeScript/JavaScript, Python, Go, Java, C/C++, C#, Ruby, Kotlin, Swift, PHP and Scala. Full list and how to add one: [docs/languages.md](docs/languages.md).

## Build from source

```sh
git clone https://github.com/openwt-vn/graphrelay graphrelay
cd graphrelay
cargo build --release     # binary at target/release/gr
cargo test --workspace
```

Requires Rust 1.78+.

## Status

Early and under active development. Supported platforms: macOS Apple Silicon, Linux x86_64, Linux ARM64. macOS Intel is not in the official release artifacts; building from source there should work.

## Documentation

[Setup guide](docs/setup-guide.md) · [How retrieval works](docs/how-it-works.md) · [Languages](docs/languages.md) · [Relevance judges](docs/relevance-judges.md) · [Benchmarks](docs/benchmarks.md)

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) and our [Code of Conduct](CODE_OF_CONDUCT.md). To report a vulnerability, see [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE) © OpenWTVN
