# Plan 26 — A retrieval benchmark that can carry decisions

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

Status: **DECIDED 2026-09-30 — the agent benchmark is the source of truth; the Tier 1 mined benchmark below was NOT built.**

## 0. Decision (user, 2026-09-30)

The agent benchmark in `benchmarks/dataset/` (LLM agent + blind judge, ERPNext) is the **source of truth**, with
**Claude Sonnet 5** and **GLM 5.3 Flash** as the agents and **our tool** measured (2026-10-01 addendum: `graphify`, `codegraph-colby`, `codegraph-jev`, `codegraph-laya` re-added as optional arms at the user's request; see `CLAUDE.md` "Benchmarking" for the 3-repeat result, which is a tie). Arms: `codegraph` plus a
`baseline` no-tool control (chosen 2026-09-30 so a coverage number has something to compare against). All other
benchmarks were retired and deleted: the retrieval-only suites and their scripts, the old `benchmarks` binary,
`offline_eval`, `bench-reports/`, every earlier agent-benchmark run, all competitor arms, and the untracked
competitor/indexing clones (~15 GB). Only `benchmarks/src/bin/offline_index.rs` (indexing time / memory /
upgrade behaviour) is kept as a measurement tool. Recoverable from git at commit `5d413ea`.

Implemented: `benchmarks/dataset/run-sot.sh` (build, isolated-HOME index, daemon, GLM proxy, run, grade,
report), `tasks/sot_questions.txt` (frozen), trimmed `run.py` / `providers.yaml` / `proxy/config.yaml` /
report scripts, `CLAUDE.md` "Benchmarking", a new `benchmarks/dataset/README.md`, and the README benchmark
section. The old ship gates (MRR ≥ 0.50, p95 ≤ 100 ms) were defined on retired suites and are now
**undefined**; `CLAUDE.md` says what applies until the owner sets numbers.

**Still open:** a GLM 5.3 Flash run (needs `OPENROUTER_API_KEY`); `--repeats 3` runs for both agents (the
reference run is n = 6, one repeat, Sonnet only); a larger frozen question set if small effects must be
detected; numeric gates. Sections 2-4 below describe the Tier 1 alternative that was **not** pursued and are
kept for the reasoning; section 1 (why the old suites could not carry decisions) and sections 5-6 stay valid.

## 1. Why the current benchmark cannot carry decisions (measured 2026-09-30)

The three suites (`benchmarks/llm_tasks_{lexical,keycloak,erpnext_90}.yaml`, 40 / 40 / 90 tasks)
drove every retrieval decision in Plans 16-25. Checked against the data already on disk:

| Weakness | Evidence |
|---|---|
| **Name leakage** | The query contains the target's exact identifier in **70%** (lexical) and **65%** (keycloak) of tasks, and the identifier or all its words in **88%** / **80%** — including 12 of 15 lexical "semantic_gap" tasks. erpnext (extractor-enforced) has 0%. These suites largely test lookup by name; whole-identifier matching (Plan 24 item 3.1) exploits exactly that. |
| **Ambiguous gold** | Gold is a bare function name, so any same-named symbol scores a hit. **22-25%** of golds match more than one indexed symbol (worst cases: 33, 16, 57 same-named symbols). 2 erpnext golds are absent from the index (invalid tasks). |
| **Too few tasks** | Paired bootstrap on tasks (95% CI) of effects already reported: Plan 24 total, keycloak Δ +0.038 [−0.048, +0.129]; erpnext Δ +0.015 [−0.009, +0.041]; Laya English w=0.05 erpnext −0.007 [−0.021, +0.001]. Only lexical's Plan 24 gain [+0.026, +0.180], Laya English w=0.60 on lexical and keycloak, and Laya multilingual w=0.30 on keycloak exclude 0. Decision thresholds of ±0.01 (used in Plans 24-25) are far below the noise: a paired-delta SD of ~0.28 needs ~350 tasks per corpus for ±0.03 and ~750 for ±0.02. |
| **Tuned on the test set** | `TEST_FILE_PENALTY`, the damping value, PPR settings, the 0.2 df cap and the Laya weight were all chosen on these same 170 tasks. Reported numbers are optimistic by construction. |
| **Scoring** | Lenient bare-name matching (above); `#N` / `$` normalization credits any overload; one gold per task although a change often touches several functions. |
| **Agent tier is noisy** | `benchmarks/dataset/` (LLM agent + LLM judge, 30 questions, one run, one corpus) reports arm differences of 66-72% coverage that are not statistically distinguishable (McNemar p=0.289); the judge and agent are non-deterministic and a run costs $20-30/arm. Fine as a periodic end-to-end check, wrong tool for 0.01-0.05 deltas. |

What is already good and kept: determinism (within and across processes), `offline_index`/`offline_eval`
harness, pinned commits, calibration against recorded baselines.

## 2. Design

**Two tiers.** *Tier 1* is this plan: a deterministic retrieval benchmark used for engineering
decisions. *Tier 2* is the existing `benchmarks/dataset/` agent harness, kept as a periodic
end-to-end check and never used to decide small deltas.

**Tasks mined from git history** (extends `scripts/extract-erpnext-tasks.py`, which already does
this for one corpus, Python only, one "dominant function" chosen by a diff regex):

- Query = commit subject + body (cleaned). Gold = the **set** of functions whose bodies changed,
  found by running the codegraph extractor on the post-commit blob and intersecting hunks with symbol
  spans (not a regex on `def`).
- Gold identity = **root-relative qualified name** (stable since Plan 24 Phase 4), not a bare name.
  A task is kept only if every gold symbol exists under that name at the pinned SHA.
- Filters: 1-3 non-test gold functions; drop chore/bump/lint/revert/translation/merge commits; dedupe
  backports by normalized subject; cap tasks per file so hot files do not dominate; minimum query
  length.
- **Slices reported separately, never dropped:** name-leak vs no-leak (real agents paste identifiers,
  so the leak slice is realistic), single vs multi-gold, per language.
- **Answerability audit:** hand-check ~30 random generated tasks and report the valid rate (commit
  messages are noisy).

**Split.** Hash each commit into dev / test. The test split is frozen (its hash recorded here) and run
only at decision points; all tuning uses dev. The old 170 tasks stay as a legacy dev slice for
continuity, explicitly labelled tuned-on.

**Scoring.** One pre-declared primary metric: MRR@30 over the gold set (best-ranked gold symbol,
qualified-name match). Secondary: recall@5, file-level recall@5. Every comparison prints a paired
bootstrap CI and win/loss counts; decision rules use the CI lower bound, not point deltas.

**Protocol.** One command indexes each corpus at its pinned SHA into scratch databases and prints
the table, including the calibration check and a **cross-process** determinism check (a same-process
repeat missed the PageRank float-order bug). Latency: warm-up excluded, idle machine, fixed repetitions.

**Corpora.** erpnext first (full history, ~8,900 non-merge commits since 2024). keycloak and
lexical are depth-1 clones and need deepening (network, disk). Further diversity (a Go or Rust repo)
optional.

**Optional external track.** SWE-bench Lite/Verified localization (issue text -> functions in the
gold patch): outside data nobody here has tuned on. Needs one checkout and index per instance (hours);
verify the dataset fields and size before scoping.

## 3. Sequencing

1. Approve scope (open questions below).
2. Build the miner and the erpnext benchmark end to end; audit answerability.
3. Deepen keycloak / lexical (if approved) and mine them.
4. Freeze the test split; record its hash.
5. **Re-score the Plan 24 and 25 decisions on the frozen test split** (df cap, whole-identifier terms,
   token dedupe, PPR order, Laya weight). Do not edit the old reports piecemeal before then.

## 4. Open questions for the user

1. Tier 1 only, or also a refreshed Tier 2 agent run on the new tasks?
2. OK to `git fetch` deeper history for keycloak and lexical (network, a few GB)?
3. Add the external SWE-bench localization track?

## 5. Tier 2 run on the release build (2026-09-30, done)

`benchmarks/dataset/results/hard/claude-code/claude-sonnet-5-release-2026-09-30/` (pre-registration,
transcripts, `grades.json`, `RESULTS.md`). Agent and blind judge: `claude-sonnet-5`; 30-turn cap; Grep/Read/Glob
floor plus one MCP server. Questions: the first of each capability group (Q01, Q06, Q11, Q16, Q21, Q26): six, so
every group appears once. codegraph = `cargo build --release` of commit `1f9038d`, run as its own daemon on an
isolated HOME (live indexes untouched), erpnext freshly indexed at pin `1887825ce5`. graphify and
code-review-graph were rebuilt at that pin the same day (the code-review-graph DB predated the re-pin).
The tool floor held (only Glob/Grep/Read plus each arm's MCP tools appear in the transcripts).

| Arm | Mean coverage (n=6) | Avg tool calls | Avg tokens/query | Cost |
|---|---|---|---|---|
| baseline | 65.3% | 15.7 | 272k | $1.61 |
| **codegraph** | **70.8%** | **11.2** | **244k** | **$1.57** |
| graphify | 62.5% | 15.8 | 300k | $1.67 |
| code-review-graph | 62.5% | 15.7 | 444k | $2.11 |

**What this can and cannot say.** n=6 per arm, one run each, LLM agent and LLM judge: differences of 3-8 points
are inside the noise (the report's McNemar test had 1 discordant pair). It shows the release build works end to
end through the real harness and uses its MCP tools (`search_code`, `inspect`, `trace_impact`), with fewer
tool calls than the other arms. It does **not** show codegraph is better. The older full 30-question Sonnet 5 run
(older codegraph build and older erpnext pin) scored the same questions quite differently per arm (e.g. baseline
Q01 0% there, 66.7% here), a direct illustration of run-to-run noise.

## 6. Scope check (2026-09-30)

**Deepening keycloak and lexical (needed to mine their history).** Both local clones are depth-1 (keycloak `.git`
52 MB, lexical 7 MB). GitHub reports 606 MB (keycloak) and 143 MB (lexical) for the full packed history; the window
since 2025-09-01 has about 3,400 (keycloak) and 870 (lexical) commits, since 2026-01-01 about 2,550 and 730. A bounded
`git fetch --shallow-since=2025-09-01` is a fraction of the full size (estimate, not measured). Disk free at the time:
13 GB. Needs network; changes only the two clones' `.git`, not their working trees.

**SWE-bench localization track.** Verified from the Hugging Face dataset API: SWE-bench Lite has 300 test (+23 dev)
instances and Verified 500, all Python, with `repo`, `instance_id`, `base_commit`, `patch`, `problem_statement`.
Query = problem statement, gold = functions changed by `patch`. Each instance needs the repo checked out at its
`base_commit` and indexed: about 12 repositories; done naively that is one full index per instance (roughly 300 x
30-60 s, several hours plus clones); grouping instances per repo and re-indexing incrementally between commits
should cut that a lot (not tried). The problem statements often paste identifiers and tracebacks, so the leak rate
would need measuring first. Python only, so it adds no language diversity.
