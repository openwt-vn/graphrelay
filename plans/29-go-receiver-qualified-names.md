# Plan 29 — Go receiver types in qualified names (2026-10-30)

## Context
The Gitea structural run with Sonnet 5.5 (`results/sot/structural-gitea-2026-10-30-sonnet55/`) tied the baseline on coverage (98.7% vs 99.2%, CI [-1.9, +0.6]); the whole gap was S07. `trace_impact` returned the five factory methods as bare `services/migrations/<file>.go::New`. Go methods are top-level declarations, so the span-containment "enclosing type" lookup in `Extractor::extract_symbols` found nothing, and the receiver type (`GiteaDownloaderFactory`) never reached the qualified name. The agent had to guess the type names, said so in its answer, and lost the facts the key phrases as `GiteaDownloaderFactory.New`.

## Change
- `LanguageSupport::receiver_type(def_node, source)` (default `None`); `GoLang` reads the `method_declaration` receiver (`T`, `*T`, `T[K]`, `*T[K]`, unnamed).
- `extract_symbols` uses the receiver as the enclosing type: `path::Receiver::Method`, was `path::Method[#N]`. Same-named methods on different types in one file no longer need a `#N` suffix.
- Migration `016_go_receiver_qualified_names.sql`: symbol ids change for Go methods, so Go files are reset to the pending sentinel (re-indexed once) and the SCIP ingest marker is cleared for projects with Go files.
- Side effect, intended: `resolve_type_key` (edge resolution locality) now sees the receiver type for Go, as it already did for Java and others.

## Verification (2026-10-30)
- `cargo test --workspace` 542 passed, clippy clean. New: receiver naming (pointer, value, unnamed, generic, top-level `New`), id stability/uniqueness, migration 016.
- Gitea indexed with the new binary in an isolated `$HOME`: `trace_impact(basicAuthorization, depth 2)` now lists `CodebaseDownloaderFactory::New`, `GiteaDownloaderFactory::New`, `GithubDownloaderV3Factory::New`, `GogsDownloaderFactory::New`, `OneDevDownloaderFactory::New` (was bare `New` x5).
- `impact_recall.py` on all 12 Gitea tasks: recall 1.00, precision 0.755, identical to the pre-change figures.

## Not done
- No agent re-run: the effect on S07 coverage with Sonnet 5.5 is expected (the names are now in the tool output) but unmeasured. `run-sot.sh --repo gitea --structural --models claude-sonnet-5-5 --repeats 3` would cost about $1.4.
- Other languages where methods live outside their type's span (Rust `impl` blocks, C++ out-of-line definitions) were not checked.
