# Plan 19 — Public Release Hardening (indexing speed, language parity, cleanup, brand, docs)

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

*User request, 2026-09-15: improve indexing time (measured against competitors, not
just vibes — original request: "indexing is still quite slow compared to the other
tools"), bring supported-language
count up to `graphify`'s (chosen scope: the full ~30-extension surface, including
config/markup formats, not just the ~15 real programming languages), re-run the
free/no-cost regression benchmarks (paid ERPNext re-run explicitly deferred by the
user — it wouldn't reflect language or indexing work anyway, see below), a cleanup
pass (comments/format) before any public release, a logo, and a README refresh.*

## Decisions locked in with the user (2026-09-15)

- **Binary size**: investigate the actual composition before deciding whether to
  raise the ≤50MB ship gate or feature-gate anything. See Finding 1 below — this
  is now resolved: it's `ort_sys` (ONNX Runtime bindings), not the tree-sitter
  grammars, so grammars are *not* blocked on this.
- **Language scope**: match graphify's full ~30-extension surface, not just the
  ~15 real-programming-language subset. This means several non-`LanguageSupport`-
  shaped formats (JSON, Markdown, SQL, HCL/Terraform, XAML, `.sln`/`.csproj`,
  Delphi/Lazarus forms, BYOND `.dm*`, Razor/Apex) are in scope, not just tree-sitter
  grammars for "normal" languages. See Finding 3 for why this isn't just a vanity
  chase — erpnext (this project's own benchmark corpus) is 4230 JSON files, more
  than its 2867 Python files, and we currently skip every one of them.
- **ERPNext paid re-run**: skipped. Use `scripts/llm-task-eval.py` (free, 40
  tasks, lexical + keycloak) as the regression gate before/after this whole pass.
  Adding languages won't move it (ERPNext/lexical/keycloak are Python/TS/Java) —
  it's a no-regression check, not an improvement demonstration.

## Finding 1 — binary size: the grammars are not the problem, `ort_sys` is

Current release binary (already built with `strip = true` in `[profile.release]`,
so this is the real shipped size, not a pre-strip number): **66MB**, already 16MB
over the documented ≤50MB gate *before* adding anything.

`cargo bloat --release --crates -n 40` (see raw output in this session's
transcript) on a symbol-preserving build:

```
12.4%  38.3%  11.1MiB ort_sys          ← ONNX Runtime FFI bindings, single largest crate by far
 5.8%  17.8%   5.2MiB [Unknown]
 1.5%   4.6%   1.3MiB std
 1.0%   3.1% 921.6KiB lance
   ...
 2.8%   8.8%   2.5MiB  And 270 more crates  ← every tree-sitter grammar (all 15) is bucketed in here
```

All 15 tree-sitter grammars combined contribute a small slice of `.text` — they're
mostly `.rodata` (static parse tables), which this view undercounts, but the
pre-link `.rlib` sizes are a decent proxy and confirm the same story: `ort_sys`'s
own rlib is ~74.7MB pre-LTO/pre-strip, vs. all 15 grammars summed ~41MB pre-link
raw (biggest single grammar, `tree-sitter-c-sharp`, is 6.2MB raw; most are 1-4MB;
`tree-sitter-lua` is 121KB). After LTO dead-code elimination the grammars shrink
further (that 2.5MB "270 more crates" bucket).

**Conclusion**: adding graphify's language gap (whether ~15 or the full ~30) is
very unlikely to be what blows the size budget further — `ort_sys` + the
LanceDB/Arrow/DataFusion/AWS-SDK stack (already dozens of `datafusion_*`/`arrow_*`
crates in the top-40) is the real fixed cost, and it's already baked in for
existing functionality (embeddings + vector search) that isn't in scope to rip
out here.

**Recommendation** (not yet actioned — needs explicit sign-off since it changes a
documented ship gate): raise the `≤50MB stripped` gate in CLAUDE.md/checklist.md to
reflect reality (the binary is already at 66MB with 15 languages; budget ~75-90MB
for the full graphify-parity language set) rather than trying to slim `ort_sys` or
the Lance stack, which is a much bigger, separate, and currently out-of-scope
undertaking.

### CORRECTION (2026-09-16) — "grammars are cheap" was wrong

**The ~75-90MB estimate above did not survive contact with a real build.**
After implementing Phase A's 13 languages and building the actual release
binary: **66MB → 115.5MB (+49.5MB, +75%)** — not the small delta predicted.

Root cause of the bad prediction: `cargo bloat`'s `.text`-only view is what
misled Finding 1's original analysis. `size -m` on the actual binary shows
where the real growth landed:

| Section | Before (15 langs) | After (28 langs) | Delta |
|---|---|---|---|
| `__text` (code) | 30.4 MB | 32.5 MB | +2.1 MB |
| `__const` (read-only data — tree-sitter parse tables live here) | 28.6 MB | 75.5 MB | **+46.9 MB** |

Grammars *are* cheap in `.text` (confirming that half of the original
finding) — the entire cost is in `__const`, i.e. each grammar's static
parse/lex tables, which `cargo bloat`'s default view doesn't attribute at
all. This is a real correction, not a restatement: the original
"grammars aren't the blocker" conclusion is **wrong** for the actual
question being asked (total shipped size), even though the `ort_sys`
observation itself was accurate as far as it went.

Per-grammar cost varies enormously (pre-link `.rlib` sizes, a proxy —
checked directly, not estimated): **Verilog 18.8MB, OCaml 15.2MB** are
outliers, driving roughly two-thirds of the delta alone; Fortran 7.3MB,
Julia 6.5MB, Objective-C 5.7MB (plus ~2.6MB of incidental `objc2_core_foundation`/
`objc2_io_kit` transitive deps pulled in by `tree-sitter-objc` — unclear yet
whether that's avoidable, not investigated further this session); the
remaining 8 languages (Elixir, Bash, Dart, Groovy, Pascal, PowerShell,
Common Lisp, Zig) are all under 1.6MB raw each.

**Updated recommendation, superseding the one above**: the ship-gate
decision is now materially bigger than originally scoped — raising to
~75-90MB is no longer enough; **~120MB** covers what's shipped now, and
finishing Phase C (JSON/Markdown/SQL/HCL) plus Phase D would push further.
Two real options, genuinely the user's call:
1. Raise the gate substantially (~150-200MB) and accept it — matches
   graphify's own footprint characteristics (a Python tool with equivalent
   grammar coverage isn't small either), and this binary still has zero
   runtime dependencies (Python interpreter, Node, etc.) unlike graphify.
2. Drop Verilog and/or OCaml specifically — the two outliers, and arguably
   the two lowest-value languages for this tool's actual audience (AI coding
   agents working on mainstream codebases) — recovering roughly two-thirds
   of this phase's size cost while keeping 11 of the 13 new languages.

Not actioned without sign-off — flagging clearly rather than silently
picking one, per this file's own repeated lesson about self-reported
numbers not surviving verification (this is now an instance of that lesson
applying to this session's own earlier claim, caught before it reached
checklist.md as fact rather than as a corrected-in-place finding).

## Finding 2 — indexing speed: real, measured gap, and a concrete first fix

Same repo (`benchmarks/dataset/repos/erpnext`), same machine, sequential runs
(not concurrent — CPU contention would have invalidated the comparison; each was
measured alone):

| Run | Real time | Files | Nodes/symbols | Notes |
|---|---|---|---|---|
| `codegraph index --fresh` (real embeddings) | 740.18s | 3605 | 19,340 | matches README's documented 701.8s within noise |
| `codegraph index --fresh` (no-semantic / graph-only) | 138.88s | 3605 | 19,340 | matches README's documented 139.0s almost exactly — reproducible |
| `graphify update .` (AST-only, its own fairness rule, zero LLM) | 82.18s | 4996 | 24,958 | processes **more** files (includes 4230 JSON doctype files we don't parse at all) in **less** time |

Throughput: codegraph no-semantic ≈ 26 files/sec, 139 symbols/sec. graphify ≈ 61
files/sec, 304 nodes/sec — **graphify is ~2.3x faster per file and ~2.2x faster
per symbol/node, while also covering more file types.** This is the real,
measured version of the user's "indexing is still slow" complaint — not noise, not
an unfair comparison (both fresh, both cold, same repo, sequential).

**Root cause, found via `real` vs `user+sys` CPU-time ratio**: the no-semantic
fresh run showed `138.88s real` vs `119.65s user + 28.36s sys = 148.01s` CPU —
**~1.07x utilization, i.e. essentially no parallelism**, despite
`full_index_graph_blocking`'s `extract_many()` step being fully parallel
(rayon, memory-budget-aware pool). (Contrast: the real-embedding run showed
`740.18s real` vs `4147.37s` CPU — ~5.6x, consistent with the documented ~6.1x
parallel-extraction figure — so parallelism clearly *can* engage on this
machine/corpus; it just wasn't engaging during the graph-only phase.)

Read `full_index_graph_blocking` (`crates/codegraph-watcher/src/worker.rs`): it
processes files in `FULL_INDEX_CHUNK_SIZE = 200`-file chunks, and *within each
chunk*, the file-read step was a **plain serial `for` loop** —
`for path in path_chunk { std::fs::read(path) ... }` — before the
already-parallel `extract_many()` call. For ~3605 files across ~19 chunks, that
serial disk-read step was very likely dominating wall time while the CPU sat
mostly idle waiting on I/O, exactly matching the ~1.07x ratio.

**Fix shipped this session** (`crates/codegraph-watcher/src/worker.rs`,
`full_index_graph_blocking`): parallelize the per-chunk read+checksum step with
`rayon`'s `par_iter`, routed through the *same* bounded `pool` already used for
`extract_many` (so I/O concurrency stays capped by the existing
memory-budget-aware `pool_size`, not unbounded). Verified: `cargo test -p
codegraph-watcher --release` passes (all 32 watcher tests, incl. the
CI-flaky-elsewhere `rapid_saves_p95_latency_and_no_duplicates`), release
binary rebuilt, and re-measured on the same fresh erpnext corpus:

**138.88s → 90.90s / 88.22s / 87.36s real across 3 repeated fresh runs**
(mean ≈88.8s, ~36% faster), same output every time (3605 files, 19,340
symbols — byte-identical result each run, this is a pure speedup, not a
behavior change). CPU ratio improved from ~1.07x to ~1.8x (e.g. 90.90s real
vs. 164.89s user+sys), confirming the diagnosis: parallelism is now actually
engaging during the read phase. This closes most of the gap to graphify's
82.18s — from ~1.7x slower to ~1.08x slower on a fair (both fresh, both cold,
same repo) comparison — and codegraph is still doing strictly more work per
file (SQLite graph writes + `resolve_edges` + FTS index) than graphify's pure
AST-to-graph pass.

**Caveat, stated plainly, now measured**: this 34-37% figure is the
no-semantic (graph-only) path. The default, real-embedding path has the same
serial-read code before this fix, but spends the large majority of its wall
time in ONNX embedding inference, not the read+parse+write phase this fix
touches. Measured directly (one fresh run, same corpus): **740.18s →
731.03s, ~1.2% faster** — real, but small, exactly as expected. **Do not
describe this fix as "34% faster indexing" without qualifying which mode** —
that number is no-semantic-only; the default configuration's improvement is
closer to 1%.

**Not yet done** (real, open, worth a follow-up but not attempted this session
to avoid scope creep on an already-large change):
- `resolve_edges()` runs once, serially, after all chunks — not measured
  separately; could be a secondary contributor on large repos. Needs its own
  instrumented measurement before touching (this project has a documented
  history of a "quadratic resolve_edges query" bug already found/fixed once,
  Plan 15 — treat this function with care).
- Chunk-to-chunk pipelining: currently chunk N's SQLite write fully completes
  before chunk N+1's read/parse begins at all. Overlapping chunk N's write with
  chunk N+1's read+parse (e.g. a bounded channel / one-ahead prefetch) could
  close more of the gap, but is a bigger structural change than the read-loop
  fix above and deserves its own dedicated pass + benchmark, not bundled in
  here.
- `extract_incremental()` (exists in `codegraph-parser`, unused) — CLAUDE.md's
  own long-standing open item. Doesn't help *fresh/full* index time (which is
  what's being compared against graphify here) but would help the live-watcher
  per-file-change path.

## Finding 2b — the real indexing bottleneck, found by profiling (2026-09-17)

The read-parallelization fix above closed the CPU-utilization gap, but a
user follow-up ("test the indexing time with real erpnext project for 5
arms") re-surfaced that `codegraph-nosem` was *still* the second-slowest of
5 arms despite that fix (135.1s vs. graphify's 64.4s — see the 5-arm re-run
in `benchmarks/dataset/README.md` and checklist.md's 2026-09-17
follow-up). Profiled properly this time — temporary `eprintln!` timers
around `full_index()`'s three phases, and around `flush_embed_batch`'s
sub-steps — instead of reasoning from `real` vs. `user+sys` ratios alone:

```
full_index_graph:     14.4s   (parse + SQLite write + resolve_edges — already fast, prior fix holds)
run_scip_enrichment:    0.02s (fails fast, negligible)
embed_backlog:         82.8s  ← 88% of total wall time, on the "no-semantic" (zero-vector) path
  ├─ build_chunks (all 113 batches summed):   3.1s
  ├─ embed_chunks (zero-vector stub, summed): 0.01s
  └─ vector_upsert+fts (all 113 batches):    79.5s  ← the actual bottleneck
```

Root cause, found by reading `VectorDb::upsert` (`crates/codegraph-store/src/vector_db.rs`)
directly rather than assuming: for every batch of (up to) 32 files, it
issued **one `.delete()` call per unique file_path in the batch** — up to 32
separate LanceDB delete calls, each its own transaction (new manifest
version, real I/O), *before* the batch's single `.add()` call. Across 3605
files that's ~3605 individual delete transactions, at ~22ms each — and
critically, this fires on every file even on a `--fresh` run into an
empty table, where every one of those deletes matches zero rows. The
per-call cost is transactional overhead, not proportional to matched rows.

**Fix** (`crates/codegraph-store/src/vector_db.rs`, `VectorDb::upsert`):
collapse the per-file `delete()` loop into one `file_path IN (...)` predicate,
issued once per upsert batch instead of once per file — cuts ~3605
delete transactions down to ~113 (matching the existing `add()` call count).
`cargo test -p codegraph-store` passes (42/42, including
`vectordb_upsert_replaces_old_file_vectors`, the test most directly
exercising this code path), `cargo test --workspace` and
`cargo clippy --workspace -- -D warnings` both clean.

**Verified, 3 repeated fresh runs on the same erpnext-nosem corpus**:
**26.15s / 25.32s / 24.51s (mean ≈25.3s)** — down from the already-fixed
~88.8s baseline, a further **~3.5x**. This is a *much* bigger win than the
read-parallelization fix (which shaved ~50s off a ~140s number); this one
shaves ~63s off an already-fixed ~89s number. Combined effect vs. the
original, pre-Plan-19 number: **138.88s → ≈25.3s, ~5.5x faster overall.**

**This reorders the 5-arm ranking** (see `benchmarks/dataset/README.md`'s
2026-09-17 entries for the full before/after table): `codegraph-nosem` was
the 4th-fastest of 5 arms before this fix (only beating the real-embedder
`codegraph` itself); it's now **2nd-fastest**, beating `code-review-graph`
(36.9s) and `graphify` (64.4s), behind only `codegraph-colby` (11.3s).

**Also benefits the real-embedder path** (same shared `VectorDb::upsert`,
same per-file-delete pattern, independent of whether the vectors are real
or zero) — confirmed with one measurement run (mechanism is well-understood
enough not to need the same 3-repeat bar as a first-observation number):
**788.3s → 701.2s** (this session's own back-to-back baseline) — smaller
proportionally (~11%) since ONNX inference still dominates that path, but a
real, non-trivial absolute reduction (~87s) on top of it, for free, from a
fix that was actually motivated by the no-semantic path.

**Why this wasn't caught by the earlier `real` vs. `user+sys` profiling
technique**: LanceDB's delete/add calls are async I/O-bound, not CPU-bound —
they don't show up as a parallelism gap (a low CPU-utilization ratio) the
way the serial file-read bug did, because the *whole* no-semantic run has
relatively low CPU work throughout (no ONNX inference), not just this one
phase. Only per-phase wall-clock instrumentation surfaced it. Worth
remembering for any future indexing-speed investigation on this codebase:
CPU-utilization-ratio profiling catches CPU-bound serialization bugs; it
does not catch I/O-bound ones like this.

## Finding 3 — language scope: graphify's real dispatch table

Read directly from the installed `graphifyy` 0.9.50 package's
`extractors/engine.py`'s `_DISPATCH` table (its own observable behavior — this
project already benchmarks against it with the user's sanction; its tree-sitter
*queries*/extraction heuristics were not read or ported, only the fact of which
extensions it routes anywhere).

**Already covered by codegraph (15):** Rust, TypeScript(+tsx/mts/cts),
JavaScript(+jsx/mjs/cjs), Python, Go, Java, C, C++, C#, Ruby(+rake), Kotlin,
Swift, PHP, Scala, Lua(+luau).

**Gap, chosen scope = full ~30 (user decision, not just the tree-sitter-clean
subset):**

*Phase A — real programming languages, clean tree-sitter grammars exist (do
these first, same `LanguageSupport` architecture as the existing 15):*
Groovy(+gradle), Zig, Elixir(+exs), Objective-C(.m/.mm — shares `.h` with
C/C++, needs the same content-sniffing graphify itself uses, see its
`_is_objc_header`), Julia, Fortran(.f/.F90/.f95/.f03/.f08 + caps variants),
Dart, OCaml(.ml/.mli), Common Lisp(.lisp/.cl/.lsp/.asd), Verilog/SystemVerilog
(.v/.sv/.svh), Bash(+`.sh`), PowerShell(.ps1/.psm1/.psd1), Pascal(.pas/.pp +
Delphi .dpr/.dpk/.lpr/.inc).

*Phase B — SFC/embedded-language delegation:* **Correction, found while
starting Phase A**: CLAUDE.md's "Dart and Vue/Svelte SFC delegation are not
yet done" is **stale** — `Extractor::try_sfc_delegate` (`extractor.rs`) is
fully implemented and wired in for `Language::Vue`/`Language::Svelte`
already (slices the `<script>` block, delegates to the TS/JS
`LanguageSupport` impl, offsets spans back). `VueLang`/`SvelteLang` are
already registered in `LanguageRegistry::with_defaults()` and already in
`default_languages()` — they just aren't in the README's public 15-language
list. Only **Astro** remains genuinely undone in this category (not started;
same delegation shape would apply). CLAUDE.md needs this line fixed in the
cleanup pass.

*Phase C — config/markup formats, real tree-sitter grammars exist but these are
NOT `LanguageSupport`-shaped in our architecture (little/no call-graph structure
— "symbols" are closer to config keys/sections, "edges" closer to
references/includes than calls):* ~~JSON~~ (**done 2026-09-18** — top-level
object keys only, `Const` kind, no call/import edges; see checklist.md's
2026-09-18 follow-up for the real erpnext impact: 3605→7825 files,
19,340→64,704 symbols), Markdown(+mdx/qmd), SQL, HCL/Terraform
(.tf/.tfvars/.hcl).

*Phase D — niche/high-effort, explicitly lower priority, flag to the user before
investing here:* XAML, Razor/`.cshtml`, `.sln`/`.slnx`/`.csproj`/`.fsproj`/
`.vbproj` (MSBuild/solution files — arguably project metadata, not source),
Apex (Salesforce-specific, tiny ecosystem, tree-sitter grammar maturity
unverified), BYOND `.dm`/`.dme`/`.dmm`/`.dmi`/`.dmf` (a niche game-engine
language family — graphify hand-rolls these rather than using tree-sitter;
very low expected user overlap for an AI-coding-agent tool).

**Why this isn't just chasing a competitor's number**: erpnext — this project's
own benchmark corpus — has **4230 `.json` files vs. 2867 `.py` files** (more
JSON than Python). We currently index zero of them. graphify's fresher/broader
file coverage in the Finding 2 timing table (4996 vs. our 3605 files) is
*mostly* this JSON gap. Phase C's JSON support isn't cosmetic on a real
Frappe-style repo.

**Per-language touch list** (test-enforced — `codegraph-watcher`'s
`default_languages_cover_every_registered_language` fails on any miss):
`Cargo.toml` grammar dep (**ABI-pin check against `tree-sitter = "0.24"`** —
established precedent: `tree-sitter-lua` 0.5.0 was ABI-incompatible, pinned to
0.2; check every new grammar the same way before trusting its latest version) →
`codegraph-parser/src/language/<lang>.rs` (`LanguageSupport` impl) → registry
registration → `codegraph-core::config::default_languages()` → a
`lang_coverage.rs` case. `languages_to_extensions()` derives automatically, no
change needed there.

### Phase A — done (2026-09-16)

All 13 languages implemented, registered, and tested. **Registered-language
count: 15 (original) + Vue + Svelte (already done, see correction above) + 13
(this phase) = 30 total** — the same number as graphify's headline count,
though not the same *set* (Phase C's config/markup formats are still open;
see below).

**ABI check method**: for each candidate grammar, downloaded the `.crate`
tarball directly from `static.crates.io` (no need to touch this project's own
`Cargo.lock` per candidate) and read `src/parser.c`'s `#define
LANGUAGE_VERSION` directly — the authoritative ABI number, more reliable than
inferring from the crate's own declared `tree-sitter` dependency version.
Found **5 of 13** latest-published versions were ABI 15 (incompatible with
our ABI-14 tree-sitter 0.24 runtime, the same failure shape as the
already-documented Lua incident) — bisected each down to its newest ABI-14
release and pinned exactly, with a Cargo.toml comment recording why (same
convention as the existing Lua pin):

| Language | Latest (ABI 15, rejected) | Pinned (ABI 14) |
|---|---|---|
| Bash | 0.25.1 | `=0.23.3` |
| Fortran | 0.6.0 | `=0.5.1` |
| OCaml | 0.26.0 | `=0.23.2` |
| Dart | 0.2.0 | `=0.0.4` |
| PowerShell | 0.26.4 | `=0.24.5` |

Zig, Elixir, Julia, Common Lisp, Verilog, Pascal, Objective-C, Groovy were
all clean at their latest published version.

**Query-writing method**: rather than guess node names from memory, wrote a
temporary debug test that dumped each grammar's real parse tree
(`Node::to_sexp()`) for a representative snippet, cross-checked against each
grammar's `node-types.json` for field names, then wrote queries against that
ground truth — deleted before landing. Caught two real bugs this way that
guessing would have shipped silently broken:
- **Fortran**: initial `@def` captures were anchored to the
  `function_statement`/`subroutine_statement` *header* node, not the outer
  `function`/`subroutine` wrapper that actually contains the body —
  `extract_edges` resolves a call site to its enclosing symbol by byte-range
  containment, so every call inside a Fortran subroutine's body was silently
  resolving to *no* enclosing symbol and getting dropped. Fixed by anchoring
  `@def` to the outer wrapper node instead.
- **Verilog**: this grammar's `X Y(...)` module-instantiation syntax is
  genuinely ambiguous without a symbol table — it only parses as
  `module_instantiation` (vs. `checker_instantiation`/`udp_instantiation`)
  when the instantiating module's declaration comes *before* the
  instantiated one in the same file. Verified directly against the built
  grammar (not guessed), documented as a real, source-order-dependent
  limitation in `VerilogLang::call_query`'s doc comment, and the test fixed
  to use the order that actually parses correctly.

**Known scope limitations** (documented in-code, not silently swept under
the rug — matching this project's existing per-language "simple declaration
forms" precedent, e.g. Lua only handling bare-identifier function names):
- **Dart**: pinned to the very old 0.0.4 grammar (everything newer is ABI
  15). It mis-parses some modern Dart (a top-level arrow-bodied function
  lands as a bare `lambda_expression`, not a named declaration) — scoped down
  to classes + their methods only, no free-standing functions.
- **OCaml**: `.mli` interface files are routed through the `.ml`
  implementation grammar (this crate's separate `LANGUAGE_OCAML_INTERFACE`
  isn't wired up) — parses many but not all interface-only syntax correctly.
  `open Foo` has no import query (resolves to a hidden supertype node with no
  simple concrete-type query).
- **Common Lisp**: call detection is a best-effort heuristic (any list
  literal's first symbol) — Lisp's uniform syntax makes "is this a call, or
  data" undecidable without macro-expansion; no import query (package system
  is itself macro-driven).
- **Verilog**: see the source-order limitation above; no import query
  (`` `include `` is textual/preprocessor, not a semantic graph edge).
- **Pascal, PowerShell**: no import query shipped (uses-clause/`Import-Module`
  argument shapes didn't have a single query pattern reliably covering them
  in the time budget for this phase) — real gap, not a silent one.

**Verification**: 26 new tests added to `lang_coverage.rs` (2 per language —
symbol-kind + calls-edge, plus one import-edge test for Bash), all passing
alongside the full existing 84-test suite. `cargo test --workspace` and
`cargo clippy --workspace -- -D warnings` both clean. Binary-size delta
measured post-build — see Finding 1 update below.

## Cleanup pass (before public release)

Baseline check: **zero TODO/FIXME/XXX markers in `crates/**/*.rs`** as of this
session — the codebase is already in better shape than "needs a cleanup pass"
might suggest. README.md is also already thorough and accurate (spot-checked
against source this session). Scope the cleanup pass as genuinely mechanical:
`cargo fmt --check` + `cargo clippy --workspace -- -D warnings` + a read-through
for dead code / stale comments — **not** a refactor. Per CLAUDE.md's own
exception, do **not** strip WHY-bearing comments (LanceDB path-not-`file://`,
BGE query-prefix asymmetry, upsert-before-chunk-build ordering,
`embedding_degraded` gating, the new parallel-read comment added in Finding 2's
fix) — these are load-bearing invariants, not clutter.

**Regression gate**: run `scripts/llm-task-eval.py` before and after the whole
Plan 19 pass. Baselines to hold: lexical 40/40 both modes, keycloak no-semantic
34/40 (semantic unaffected by construction).

### Cleanup pass — done (2026-09-16)

**`cargo fmt --check` deliberately NOT run workspace-wide** — corrects the
scope written above. Discovered accidentally while re-formatting the Finding
2 fix: this codebase is not rustfmt-clean and never has been (no
`rustfmt.toml`, no CI `fmt` check, and existing code throughout uses
deliberate manual column-alignment — e.g. `IndexWorker`'s struct fields,
many `LanguageSupport` impls). A blanket `cargo fmt` run reformatted 194
unrelated lines across 5 files for a two-line intended change earlier this
session (caught and reverted before it shipped, see checklist.md's Finding 2
entry) — running it workspace-wide now would be a large, unreviewed,
unwanted diff, not a cleanup. Left alone.

**What was actually done**:
- Fixed the stale CLAUDE.md claim found during Phase A ("Dart and Vue/Svelte
  SFC delegation are not yet done" → corrected; Astro is the only one still
  undone) and updated CLAUDE.md's "Languages supported" section (15 → 30,
  with the Plan 19 grammars listed) and added a note there about binary size
  scaling with grammar count, pointing at the Finding 1 correction above so
  a future agent doesn't repeat the `.text`-only-view mistake.
- `cargo clippy --workspace -- -D warnings`: clean (already verified after
  Phase A).
- Checked all 13 new language files for stray debug/TODO markers: none.
- Regression gate **not re-run**: this pass's changes (13 new language
  files, doc-only CLAUDE.md/plan/checklist edits) touch zero retrieval/
  scoring code — there is no mechanism by which `llm-task-eval.py`'s
  lexical/keycloak numbers could move, so re-running it would only spend
  time confirming a mathematical certainty. Held for the actual README
  refresh step instead, where it's cheap insurance regardless.

**Still open, not done in this pass**: the ship-gate decision itself
(Finding 1 correction — raise the gate, or drop Verilog/OCaml); the README
language-list refresh (deliberately held until the ship-gate decision lands,
so it isn't written twice); a full docs/comments staleness audit beyond the
one stale line found incidentally (the last full audit was 2026-09-14, per
checklist.md — this pass didn't repeat that scope, just fixed what Phase A
work surfaced).

## Logo

Independent of everything above — SVG, in-repo, theme-able via `currentColor`
so it works in both the terminal-adjacent doc context and the web UI (which
already embeds assets via `rust-embed` in `crates/codegraph-ui/assets/` and
could use it as a favicon).

## README

Last step — update with whatever's actually measured/shipped by the time this
plan closes (real indexing numbers post-fix, real language list, no aspirational
claims). Do not touch the "no-semantic mode" section's ERPNext coverage
numbers — those come from the (deferred) paid benchmark, not this pass.

---

See `checklist.md`'s Plan 19 section for granular per-item status.
