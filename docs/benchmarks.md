# Benchmarks

The only benchmark we treat as authoritative is `benchmarks/dataset/`: a real coding agent (Claude Code CLI) answers hard questions about the ERPNext codebase (~1M LOC, pinned commit), with Grep/Read/Glob plus, in the `graphrelay` arm, our MCP server. A `baseline` arm (the same floor tools, no MCP) is the no-tool control, and a blind judge grades key-fact coverage. Agents: **Claude Sonnet 5.5** and **GLM 5.3 Flash** (via OpenRouter and a local LiteLLM proxy). Optional arms (`--arms`): `graphify` and `codegraph-colby` (competitors), and `graphrelay-jev` / `graphrelay-laya` (graphrelay with a relevance judge on).

```bash
benchmarks/dataset/run-sot.sh --dry-run        # check prerequisites, spend nothing
benchmarks/dataset/run-sot.sh --repeats 3      # build, index into an isolated HOME, run, grade, report
```

The 30 questions are frozen in `benchmarks/dataset/tasks/sot_questions.txt` (six capability groups). The GLM agent needs an `OPENROUTER_API_KEY` (environment or the gitignored `benchmarks/dataset/harness/.env.openrouter`); without one the runner skips it and says so.

**Latest evidence (2026-10-06/07, all six arms, 30 questions, 1 repeat): no arm is reliably better than the no-tool baseline.**

| Mean key-fact coverage | baseline | graphrelay | graphify | colby | jev | laya |
|---|---|---|---|---|---|---|
| GLM 5.3 Flash | 77.6% | 80.2% (+2.6, CI −1.4…+6.6) | 80.5% | 70.1% | 76.9% | 78.9% |
| Sonnet 5.5 | 67.2% | 70.6% (+3.4, CI −1.6…+8.7) | 70.1% | 72.5% | 71.4% | 73.0% |

graphrelay's lead over baseline is positive on both models but its confidence interval includes zero; the arm ordering flips between models, and the judge arms never beat plain graphrelay while adding 1-2 s per search. The only difference whose interval excludes zero is colby on GLM (−7.5). Single-repeat intervals resample questions only; repeat-to-repeat swings of 5-14 points per arm were seen. Earlier, larger gaps (e.g. +21 points on GLM) came from baseline turn-cap failures that did not recur. Data: `benchmarks/dataset/results/sot/2026-10-06-glm30/` and `2026-10-07-sonnet55-30/`.

**Per-question tool-flow reports.** The base reference is the four-arm, 3-repeat run (`erpnext-sonnet55-full30-r3`, graphrelay 71.2%, baseline 70.6%): [repeat 1](benchmark-report-sonnet55-4-arms-rep1.html), [repeat 2](benchmark-report-sonnet55-4-arms-rep2.html), [repeat 3](benchmark-report-sonnet55-4-arms-rep3.html). The six-arm report below is older: [benchmark-report-sonnet55-6-arms.html](benchmark-report-sonnet55-6-arms.html) (open in a browser) shows every tool call of every arm on each of the 30 questions, with the judge's verdict per key fact. It is the single-repeat Sonnet 5.5 run `erpnext-sonnet55-clean` (baseline, graphrelay, graphify, colby, jev, laya; graphrelay 68.7%, baseline 72.2%). It predates the Plan 32-36 builds, and a six-arm 3-repeat run on the current binary has not been done (the 3-repeat comparison below covers four arms). Local paths in the transcripts were replaced with `<repo>` and `~`.

**Later, more reliable runs (2026-11, Sonnet 5.5, ERPNext, 30 questions x 3 repeats unless noted):** the shipped design scored 71.2% against a no-tool baseline of 70.6% (+0.6, CI −4.0…+5.0), with about 30% fewer tool calls and about 1.3x the cost per question. Eight later changes were each tried against it: a payload trim, a flow hint and a file-level function list (dropped, no gain); nested-result dedupe and an evidence-line field (reverted, 67.6-69.7%); and a decorated-Python twin merge, Python string-dispatch edges and `self.method()` edges (kept as correctness fixes, 69.6%, 69.0% and 70.5%). The v0.1.0 code scored 69.5% over 3 repeats (local build), and the downloaded pre-release binary (internal version 0.1.3, the same code as v0.1.0) 71.5% over 1 repeat. Every paired interval includes zero: no build is distinguishable from another or from the baseline. One question (Q22) is a confirmed loss: the agent stops after one search that looks complete (63.3% vs 75.8% over 10 repeats).

**GLM 5.3 Flash, released v0.1.0 binary (2026-11, four arms, 30 questions x 2 repeats, `erpnext-glm53flash-v013-4arms-r3`):** graphrelay 79.9% (repeats 81.4 / 78.3), graphify 77.5%, codegraph-colby 77.1%, baseline 76.6% (repeats 79.0 / 74.3). graphrelay − baseline is +3.3 points, paired CI −1.0…+8.0, so the interval includes zero; graphify (+0.9) and colby (+0.5) are indistinguishable from the baseline. Average tool calls per question: graphrelay 18.7, baseline 22.9, graphify 21.0, colby 13.9; cost per question $0.533, $0.499, $0.478 and $0.499. The baseline hit the turn cap on 2 of 60 answers (scored 0% by construction), graphify on 1, graphrelay on none, so part of the gap is 'reaches an answer within the budget'. The run was stopped after 2 of the planned 3 repeats. Tool-flow reports: [repeat 1](benchmark-report-glm53flash-4-arms-rep1.html), [repeat 2](benchmark-report-glm53flash-4-arms-rep2.html). The ordering matches the earlier single-repeat GLM run (graphrelay ahead, interval spanning zero) and, unlike Sonnet 5.5, graphrelay leads on GLM in both runs.

Indexing time (`benchmarks/src/bin/offline_index.rs`, idle machine, full index from scratch, 2026-09-29): 49 s for `facebook/lexical` (1,373 files), 31 s for ERPNext (4,770 files) and 104 s for Keycloak (10,043 files), 3.4-3.8× faster than before an old bug that forced parsing onto one core on macOS was fixed.

Earlier retrieval-only benchmarks (a 40-task lexical suite, a 40-task Keycloak suite, a 90-task ERPNext suite, a synthetic oracle corpus and a three-tool head-to-head) were retired on 2026-09-30: many queries contained the target's name, some gold labels were ambiguous, and most effects were inside the noise. Numbers from them that still appear in older plans, the checklist or `CLAUDE.md` are history, not evidence.


## Where it might help (exploratory, not a claim)

Post-hoc breakdown of the two 30-question runs above by capability group (A end-to-end, B caller/callee + impact, C semantic gap, D numeric/logic edge cases, E concurrency/database, F trap/hallucination; five questions each, one repeat). Mean key-fact coverage difference, graphrelay minus baseline, with a bootstrap 95% interval over the five questions:

| Group | Sonnet 5.5 | GLM 5.3 Flash |
|---|---|---|
| A end-to-end | +5.3 [−6.6, +15.7] | +3.3 [−2.8, +10.9] |
| B caller/callee + impact | −0.4 [−8.8, +5.8] | +3.6 [−5.9, +15.2] |
| C semantic gap | +0.0 [+0.0, +0.0] | +6.7 [−1.7, +13.3] |
| D numeric/logic edge cases | +16.2 [+6.2, +26.6] | +6.2 [−1.7, +14.1] |
| E concurrency/database | −11.2 [−18.6, −4.5] | −3.1 [−11.7, +7.1] |
| F trap/hallucination | +10.5 [−4.0, +27.5] | −1.0 [−15.0, +12.0] |

**Read this as noise until replicated.** Twelve group-by-model comparisons were made after seeing the data; at 95% about one false "significant" result is expected, and two appeared with opposite signs (D positive and E negative, both on Sonnet only; D does not replicate on GLM). A bootstrap over five questions is nearly degenerate (group C's interval collapses to a point because every question tied). The only defensible reading is that the overall result is a tie and no subgroup signal has been confirmed. In particular, **the caller/impact group (B) shows no lift**, so "better at structural questions" is not a supported claim.

**Cost and calls.** graphrelay made fewer tool calls than baseline (Sonnet 4.1 vs 5.0; GLM 13.5 vs 15.2) but cost more per question (Sonnet $2.66 vs $1.95 across 30 questions; GLM $0.159 vs $0.132). Fewer calls does not mean cheaper: tool results add input tokens.

### Structural slice (preregistered; run 2026-10-27/29)

`tasks/structural_tasks.yaml` holds 12 caller/impact questions whose answer keys come from Python's `ast`, independent of graphrelay (generator: `harness/make_structural_tasks.py`; rules and decision criteria: `tasks/STRUCTURAL_PREREGISTRATION.md`). Run it with `run-sot.sh --structural --repeats 3`; Gitea (Go, compiler-keyed) is a second slice with `--repo gitea`.

Agent results (3 reps, 12 tasks each, `claude-sonnet-5` (Sonnet 5, **not** the 5.5 used in the 30-question runs above) and `openrouter-glm-5-3-flash`, arms graphrelay vs baseline, 95% question-level bootstrap on the coverage difference; the decision rule needs the interval to exclude 0 on both agents):

| Repo | Agent | graphrelay | baseline | Difference | 95% interval |
|---|---|---|---|---|---|
| ERPNext | Sonnet 5 | 99.5% | 99.1% | +0.4 | [0.0, +1.3] |
| ERPNext | GLM 5.3 Flash | 100.0% | 95.6% | +4.4 | [+0.0, +10.9] |
| Gitea | Sonnet 5 | 99.8% | 98.6% | +1.2 | [0.0, +3.2] |
| Gitea | GLM 5.3 Flash | 99.4% | 98.8% | +0.6 | [-0.4, +2.1] |

**The preregistered rule is not met on either repo: structural questions are not a supported advantage on coverage.** Four of the five arm means are 98.6% to 99.5%, so grep and read solve almost all of these questions; the exception is the ERPNext GLM baseline at 95.6%. The gaps come from a few tasks (ERPNext GLM S04, S07, S12; Gitea Sonnet S10, GLM S07), and the intervals touch 0. About 3 of the 4.4 ERPNext GLM points come from one run: the baseline on S04 in rep 1 hit the turn cap (31 turns, 48 tool calls) with an empty answer and scored 0%. The same pattern (a baseline turn-cap failure) inflated an earlier GLM result, so treat that number as weak. What did differ was effort: over 3 reps graphrelay used fewer tool calls and cost less (Sonnet per task 7.2 vs 8.7 tool calls on ERPNext and 3.9 vs 8.4 on Gitea; total cost ERPNext Sonnet $3.51 vs $3.95, Gitea Sonnet $1.60 vs $2.79; GLM $0.08 vs $0.11 and $0.07 vs $0.09). That is an observation from one set of 24 tasks per agent, not a preregistered claim; the earlier 30-question runs (Sonnet 5.5, not the Sonnet 5 used here) cost more with the tool ($2.66 vs $1.95, +36%; GLM $0.159 vs $0.132, +20%), so it is not yet a consistent result either. Raw results: `results/sot/structural-2026-10-26/` (Sonnet), `structural-2026-10-26-glm/`, `structural-gitea-2026-10-27/`.


One part needs no model and has been run (2026-10-26, the `target/release/gr` build from 2026-10-14, ERPNext at the pin): `harness/impact_recall.py` scores `trace_impact` against those keys. Result: **recall 12/12 tasks (1.00), precision 0.91**; every extra result was a real caller in a test or patch file that the key excludes by rule. This shows the call graph is accurate on unambiguous static calls. It does not show that an agent answers better with it, and it says nothing about dynamic dispatch (`frappe.get_attr`, hooks, string-dispatched methods), which the key deliberately excludes.
