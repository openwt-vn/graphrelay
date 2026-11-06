# Third-party notices

GraphRelay itself is MIT-licensed (see `LICENSE`). This repository also refers to, or was evaluated against, the following third-party projects. They are not part of GraphRelay and keep their own licences.

## Benchmark corpora

The agent benchmark in `benchmarks/dataset/` asks questions about these open-source repositories. They are cloned at benchmark time and are **not** redistributed here:

| Project | Licence |
|---|---|
| ERPNext, Frappe | GPL-3.0 / MIT (see each upstream repo) |
| Gitea | MIT |
| Keycloak | Apache-2.0 |
| Node.js | MIT and others (see upstream) |

Raw agent transcripts, which quote these projects' source, are deliberately **not** committed (see `.gitignore`); only scores, grades and summaries are kept. Short excerpts that remain in reports are quoted for analysis and stay under their upstream licence.

## Compared tools

`graphify` and `codegraph-colby` (https://github.com/colbymchenry/codegraph, an unrelated project that shares an earlier name of this one) are third-party tools used as benchmark arms. Names belong to their owners; results are reported with their confidence intervals and the caveat that no arm is reliably better than the no-tool baseline.

## Dependencies

Rust dependencies are checked with `cargo deny` (see `deny.toml`). `internal/video/` uses Remotion, whose licence is not plain MIT; see https://www.remotion.dev/license before using or redistributing that directory (or remove it before publishing if your organisation is not covered).
