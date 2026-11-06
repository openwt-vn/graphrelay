> **2026-09-30:** the competitor arms (graphify, code-review-graph, colby, no-semantic), the DeepSeek /
> Ollama / Gemini / Opus / OpenCode providers, the "easy" question set and the old results were removed;
> the harness now runs the `graphrelay` and `baseline` arms with `claude-sonnet-5` and
> `openrouter-glm-5-3-flash` (see `../README.md`). Sections below about the removed pieces are kept as
> methodology history; the pitfalls about the tool floor, grading and results directories still apply.

# Pipeline: agent-under-test + judge

This document explains the two "agents" in the benchmark (not Claude Code
subagents — two roles played by the same `claude -p` CLI, run with different
models and configs) and how to set up / re-run the whole pipeline from
scratch.

## Pipeline overview

```
tasks/erpnext_tasks.yaml (30 questions, Q01-Q30, gold key facts)
        │
        ▼
harness/run.py  ──► [Agent 1: AGENT UNDER TEST] ──► results/<question-set>/<agent>/<model>/<arm>-results.jsonl
        │        (Claude Opus 4.8 default;             results/<question-set>/<agent>/<model>/transcripts/*.json
        │         --provider selects another,
        │         e.g. DeepSeek's native Anthropic endpoint)
        ▼
harness/grade.py ──► [Agent 2: JUDGE] ──► results/<question-set>/<agent>/<model>/grades.json
        │                (Claude Sonnet 5, ALWAYS,
        │                 real Anthropic API — never proxied)
        ▼
harness/report.py ──► results/<question-set>/<agent>/<model>/RESULTS.md
        │
        ▼ (optional — visual companion to RESULTS.md, same inputs)
harness/report_html.py ──► results/<question-set>/<agent>/<model>/tool-flow-report.html
```

Two fully separate agent roles, deliberately run with **different models**
to reduce self-preference bias (a model tends to rate answers written "in
its own style" more favorably):

| | Agent 1 — under test (run.py) | Agent 2 — judge (grade.py) |
|---|---|---|
| Model | `claude-sonnet-5` (default; overridable via `--provider`/`--model`/`--base-url` per `harness/providers.yaml`, e.g. `deepseek-v4-pro` via DeepSeek's native Anthropic-compatible endpoint — see "Running an alternate agent provider" below) | `claude-sonnet-5` (fixed, always the real Anthropic API, regardless of which agent/model directory it's grading) |
| Role | Answers a code-intelligence question using tools (Grep/Read/Glob + one MCP server) | Reads the question + key facts + answer, grades each fact covered/partial/missing |
| Calls | 1 per (task × arm) = 30 × 3 = 90 | 1 per (task × arm) = 90 |
| Input | Question + tool access to the ERPNext repo | Question + key facts + answer (does NOT know which arm produced the answer — "blind") |
| Output | Final answer + full transcript | JSON verdict per fact, with a verbatim supporting quote |

## Question set structure

30 questions (`tasks/erpnext_tasks.yaml`), grouped by tested capability
into 6 groups of 5 (A-F), each question carrying an `origin_note` (hand-
authored, or which ERPNext PR it derives from — never encoded in the id):

| Group | Capability |
|---|---|
| A | Multi-file / end-to-end synthesis |
| B | Caller/callee trace + impact analysis |
| C | Semantic gap (question has no keyword overlap with the code) |
| D | Numeric/logic edge cases |
| E | Concurrency & database semantics |
| F | Trap / hallucination resistance (premise is false) |

The set was curated from an earlier 40-question pilot: questions that
every arm answered near-ceiling (>=95% coverage, no discrimination
between arms) were dropped, discriminating/hard questions were kept, and
new harder questions filled the remaining slots — see each question's
`origin_note` for provenance. Selection ran before any question in the
final set was re-graded, so no question was kept or dropped based on a
score computed after the final set was decided.

---

## Agent 1 — agent under test (`harness/run.py`)

### Fixed config across every arm (fairness)

- **Model:** `claude-sonnet-5` by default (`--provider claude`). Every arm
  in one benchmark run must use the same model for the comparison to be
  fair — `--provider`/`--model` exist for running the *whole* set under a
  different model (e.g. DeepSeek), not for varying it arm-by-arm within one
  comparison.
- **Turn limit:** at most 30 turns (`--max-turns 30`, `MAX_TURNS` in
  `run.py`) — raised from an earlier 14/25 over this project's history;
  narrative sections elsewhere in this file/README describing a specific
  past run keep whatever cap was actually in effect at the time, not this
  current value
- **Tool floor:** always `Grep`, `Read`, `Glob` — identical for all 3 arms
- **System prompt:** full replacement (`--system-prompt`, not
  `--append-system-prompt`) — does not inherit the running machine's
  CLAUDE.md/hooks, so behavior/tool-choice doesn't drift between runs
- **`--setting-sources ""`:** loads no user/project/local settings —
  prevents personal config from leaking into the benchmark
- **`--disallowedTools`:** explicitly lists every tool NOT in the floor
  (Bash, Edit, Write, Task, Workflow, ...) — because `--allowedTools` only
  **adds** to `claude -p`'s default tool registry, it does not replace it,
  so the floor must be enforced via an exhaustive disallow list instead

### The 3 arms differ in exactly one place: the MCP server

| Arm | MCP server | Port | Config file |
|---|---|---|---|
| `graphrelay` | graphrelay daemon | `localhost:3030` | `harness/mcp/graphrelay.json` |
| `graphify` | graphify serve | `localhost:8090` | `harness/mcp/graphify.json` |
| `code-review-graph` | code-review-graph serve | `localhost:5555` | `harness/mcp/code-review-graph.json` |
| `baseline` (optional) | none | — | `harness/mcp/empty.json` |

All 3 MCP servers run over **HTTP** (not stdio) — an important technical
decision: when the Claude Code CLI spawns an MCP server over stdio itself,
there's a race condition where the server can report `pending` forever in
the `system/init` event even though it connects successfully afterward (no
mechanism to update that status mid-session). The fix: run each MCP server
as a **standing daemon** before benchmarking, health-check it, then point
`run.py` at it over HTTP.

### Running it

```bash
# 1. One-time setup (see "Setup from scratch" below)

# 2. Run one arm, all 30 questions
python3 harness/run.py --arm graphrelay
python3 harness/run.py --arm graphify
python3 harness/run.py --arm code-review-graph

# 3. Run one or a few specific tasks (useful for retrying transient failures)
python3 harness/run.py --arm graphrelay --task Q01
python3 harness/run.py --arm graphrelay --task Q26,Q27,Q28

# Spend cap (default $30/arm)
python3 harness/run.py --arm graphrelay --max-spend 50
```

**Idempotent per `task_id`:** re-running `--task Q01` drops the old Q01
row from `results/sot/<label>/claude-sonnet-5-5/rep1/graphrelay-results.jsonl`
before writing the new one — no duplicates, and no effect on other tasks'
already-good rows. That makes retrying a single failed task always safe,
without re-running all 30.

### Output

- `results/<question-set>/<agent>/<model>/<arm>-results.jsonl` — one row per question:
  question, answer, tool-call count, token breakdown, cost, turns,
  returncode, model. `<agent>` is `claude-code` today (the only runner
  implemented — see `harness/opencode_run.py` for the reserved-but-not-built
  second one); `<model>` defaults to `claude-sonnet-5`, override with
  `--provider`/`--model`/`--out-dir`.
- `results/<question-set>/<agent>/<model>/transcripts/<arm>-<task_id>.json` — full raw
  stream-json events (gitignored, too large — regenerate by re-running)
- `results/<question-set>/<agent>/<model>/<arm>-ledger.json` — per-run spend ledger
  (gitignored), records `provider`, `base_url`, `pricing` used for that run
- `results/.mcp-<arm>.json` — **not** under `<question-set>/<agent>/<model>/`,
  deliberately. `materialize_mcp_config()` writes this scratch file (the
  arm's MCP config with `__ERPNEXT__`/`__HOME__` substituted) straight to
  `results/` regardless of `--out-dir`/`--tasks-file`, because its content
  depends only on `arm` — identical across every question set, model, and
  provider. Gitignored, overwritten fresh on every `run.py` invocation; not
  a result artifact at all.

---

## Agent 2 — judge (`harness/grade.py`)

### How grading works

For each answer, the judge receives: the question + a numbered list of
key facts + the answer — it does **not** know which arm produced the
answer (row order is shuffled with a fixed deterministic seed, and the
prompt carries no arm metadata). The judge returns a JSON verdict per
fact:

- `covered` — the answer states this fact (exact wording not required,
  but the specific function/class/file names and described behavior must
  match)
- `partial` — gets part of it right (e.g. right file, wrong function;
  right mechanism, wrong name)
- `missing` — absent, or contradicts the fact

Per-question coverage = `(covered + 0.5 × partial) / total facts`.

### Audit rule against judge-fabricated quotes

Every `covered`/`partial` verdict **must** include a verbatim quote (5-15
words) from the answer. After the judge returns its JSON, the code
double-checks: if the quote isn't found exactly in the original answer,
the verdict is automatically downgraded to `missing` (even if the judge
said covered). This is an anti-hallucination guard on the judge side — it
forces every score to be backed by a citable quote rather than the
judge's unverified say-so.

**Important implementation note:** the audit rule compares quotes **after
stripping markdown** (backticks and `**`) — the judge sometimes quotes the
right content but drops the surrounding formatting characters around a
function name, and an exact-character comparison would wrongly penalize
a correct verdict. This is a real bug that was hit in practice (see
"Lessons / pitfalls" below) — if you touch the audit rule, keep the
markdown-stripping step.

### Running it

```bash
# --results-dir is required — no default — since results now live one
# directory per (agent, model), never merged.
RD=results/sot/<label>/claude-sonnet-5-5/rep1

# Grade only questions NOT yet graded (default — safe, doesn't re-spend)
python3 harness/grade.py --results-dir $RD

# Re-grade EVERYTHING, including already-graded pairs (use after changing
# the judge prompt / audit rule)
python3 harness/grade.py --results-dir $RD --regrade

# Grade a single arm only
python3 harness/grade.py --results-dir $RD --arm graphrelay
```

`grade.py` writes `<results-dir>/grades.json` **after every single grade**
(not just at the end) — important because with dozens of rows at several
tens of seconds each, writing only at the end would look like a
multi-minute hang. `JUDGE_MODEL` and its `subprocess` call are hardcoded to
the real Anthropic API with no `env=` override anywhere in this file — no
`--results-dir` value changes that, by construction, not convention.

### Output

`<results-dir>/grades.json` — array of
`{task_id, arm, coverage, n_facts, verdicts: [{fact_index, verdict, quote, reasoning}]}`.

---

## `harness/report.py` — report generation

```bash
python3 harness/report.py --results-dir results/sot/<label>/claude-sonnet-5-5/rep1
# -> results/sot/<label>/claude-sonnet-5-5/rep1/RESULTS.md
```

Reads `<results-dir>/*-results.jsonl` + `<results-dir>/grades.json` +
`tasks/erpnext_tasks.yaml`, computes: executive summary, summary table,
per-group coverage (A-F), per-question coverage, McNemar significance
test (paired; needs `scipy` for a real p-value), per-fact verdict detail.
One report per directory, deliberately never merged across `<model>`
directories — see "Running an alternate agent provider" below for why.

---

## `harness/report_html.py` — visual tool-flow report (optional, run after grade.py)

```bash
python3 harness/report_html.py --results-dir results/sot/<label>/claude-sonnet-5-5/rep1
# -> results/sot/<label>/claude-sonnet-5-5/rep1/tool-flow-report.html
```

Same inputs as `report.py` (`<results-dir>/*-results.jsonl` +
`<results-dir>/grades.json` + `tasks/erpnext_tasks.yaml`) plus
`<results-dir>/transcripts/*.json`, but a different output shape: one
collapsible card per question, each holding a per-arm column with that
arm's actual ordered tool-call sequence (tool name, input, a truncated
preview of the result), token/cost breakdown, and the final answer —
the step-by-step detail `RESULTS.md`'s tables don't carry. A self-
contained static HTML file, no JS, no external assets — open it directly
in a browser.

Arms are discovered from whichever `*-results.jsonl` files exist in
`--results-dir` (same pattern as `report.py`), so it works unmodified
against any arm set. An arm covering fewer than half the question set
(e.g. `baseline`, which several alternate-provider smoke tests only ran
for Q01) is dropped from the grid rather than rendered as 29 all-"n/a"
columns — every arm meant for real cross-arm comparison covers the whole
question set, so this only ever filters out a genuinely partial smoke
test, never a real result.

Requires `grade.py` to have already run (reads `grades.json` directly,
no `--regrade` option here — regrade via `grade.py`, then re-run this).
One report per directory, same never-merged rule as `report.py`.

---

## Setup from scratch (new machine)

```bash
# 1. Clone + pin ERPNext at the frozen commit
bash setup/setup-erpnext.sh

# 2. Install + start each MCP server (graphrelay needs GITHUB_TOKEN — private repo)
bash setup/setup-graphrelay.sh           # HTTP daemon on :3030
bash setup/setup-graphify.sh            # AST-only build, no LLM key needed
bash setup/setup-code-review-graph.sh   # HTTP daemon on :5555

# 3. Verify all 3 daemons are alive before running the real benchmark
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:3030/erpnext/mcp
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8090/mcp
curl -s -o /dev/null -w "%{http_code}\n" http://localhost:5555/mcp
# (405/406 is normal — it means the server is alive and just rejected a
# bare GET without a JSON-RPC body)

# 4. Need ANTHROPIC_API_KEY or a logged-in `claude` CLI (used by both the
#    agent under test and the judge)
# 5. Need scipy for a real McNemar p-value (runs fine without it, just no p-value)
pip install scipy

# 6. Run the full pipeline
python3 harness/run.py --arm graphrelay
python3 harness/run.py --arm graphify
python3 harness/run.py --arm code-review-graph
python3 harness/grade.py --results-dir results/sot/<label>/claude-sonnet-5-5/rep1
python3 harness/report.py --results-dir results/sot/<label>/claude-sonnet-5-5/rep1
```

## Running an alternate agent provider

`--provider <name>` (from `harness/providers.yaml`) picks a different agent
model for `run.py`; `claude` is the fixed-fairness default documented
above. Claude Code only speaks the Anthropic Messages API, so any other
provider needs an Anthropic-compatible endpoint. DeepSeek exposes one
**natively** as of their V4 release (April 2026,
`https://api.deepseek.com/anthropic`) — no local translation proxy needed;
verify against https://api-docs.deepseek.com/guides/anthropic_api/ if
DeepSeek's API changes again. Ollama has no native Anthropic-compatible
endpoint at all, so the local LiteLLM proxy this section used to describe
as obsolete (pre-V4-DeepSeek era) is back and load-bearing again, just for
a different provider — see `harness/proxy/config.yaml` and README.md's
"Local model via Ollama" section for the `ollama-qwen` setup:

```bash
# harness/.env.deepseek (gitignored) holds DEEPSEEK_API_KEY=...
set -a; source harness/.env.deepseek; set +a

# 1. Verify the endpoint alone before spending anything real:
curl -s https://api.deepseek.com/anthropic/v1/messages \
  -H "x-api-key: $DEEPSEEK_API_KEY" -H "anthropic-version: 2023-06-01" \
  -H "content-type: application/json" \
  -d '{"model":"deepseek-v4-pro","max_tokens":64,"messages":[{"role":"user","content":"Reply OK"}]}'
# usage.input_tokens/usage.output_tokens must be non-zero — the entire
# spend ledger and --max-spend guard below depend on this. An exact $0.00
# ledger later is the specific symptom of this having silently failed.

# 2. Cheapest harness smoke test (baseline arm = no MCP server):
python3 harness/run.py --arm baseline --task Q01 --provider deepseek --max-spend 1
# -> results/sot/<label>/deepseek-v4-pro/rep1/baseline-results.jsonl

# 3. Only then scale up, and grade/report the same directory:
python3 harness/grade.py --results-dir results/sot/<label>/deepseek-v4-pro/rep1
python3 harness/report.py --results-dir results/sot/<label>/deepseek-v4-pro/rep1
```

**Auth gotcha (cost real debugging time in practice):** use
`ANTHROPIC_AUTH_TOKEN`, never `ANTHROPIC_API_KEY`, for the subprocess env
when pointing at a custom `ANTHROPIC_BASE_URL`. Claude Code treats
`ANTHROPIC_API_KEY` (and an active claude.ai login) as higher-precedence
than a custom base URL's credential — setting `ANTHROPIC_API_KEY` here is
silently ignored, surfacing only as a `401 authentication_failed` retry
loop with `"apiKeySource":"none"` in the init event, not an obvious error
about the wrong env var. `run.py`'s `run_agent()` handles this correctly
already (sets `ANTHROPIC_AUTH_TOKEN`, pops any ambient `ANTHROPIC_API_KEY`
from the subprocess-only env copy) — only matters when testing an endpoint
manually with the bare `claude` CLI, as above in step 1's sibling check.

`grade.py` never changes: it always calls the real Anthropic API with
`claude-sonnet-5`, whichever `--results-dir` it's pointed at. This is not
optional/configurable by design — see the Agent 2 row of the table above.

**Why results are never merged across `<model>` directories:** `report.py`'s
`cov`/`grades_by_key` dicts and `grade.py`'s "already graded" dedup set are
both keyed by `(task_id, arm)`, not `(task_id, arm, model)`. Two different
models can legitimately produce an arm with the same name (`graphrelay`
under both `claude-opus-4-8` and `deepseek-v4-pro`) — merging their
`*-results.jsonl` into one `grade.py`/`report.py` call would silently
collide those rows under one key (e.g. `grade.py` would think DeepSeek's
`(Q01, graphrelay)` was already graded because Claude's `(Q01, graphrelay)`
was). Keep providers/models in separate directories; compare by opening
two `RESULTS.md` files side by side. Widening the key to
`(task_id, arm, model)` throughout both files would be required before ever
adding a merged/unified report mode — not done here, flagged for whoever
attempts that next.

**Observed 2026-08-07 (Q01 smoke tests only, n=1 per arm — not enough to
generalize):** `baseline` completed normally. All 3 tool arms hit
`error_max_turns` — deepseek-v4-pro burned 19-30 tool calls without a final
answer inside the shared 14-turn cap. Cost was trivial regardless
(~$0.01-0.03/task, roughly 10-40x cheaper than the Opus arms per task).
Don't raise `MAX_TURNS` for DeepSeek alone to "fix" this — that breaks the
fixed-turn-limit fairness rule for every other arm/model. If this pattern
holds across more questions, it's a real result (DeepSeek-v4-pro converges
slower with tools on this benchmark), not a bug to paper over.

## Lessons / pitfalls hit in practice (read before touching the harness)

1. **Tool floor leak:** `claude -p` inherits the entire local tool
   registry by default (Bash, Edit, Write, Artifact, Workflow, ...).
   `--allowedTools` only ADDS to that set, it does not replace it — the
   floor must be enforced via an exhaustive `--disallowedTools` instead.
2. **`--append-system-prompt` pulls in the running machine's CLAUDE.md /
   hooks** — use `--system-prompt` (full replacement) +
   `--setting-sources ""` instead.
3. **MCP stdio race condition:** a server the CLI spawns over stdio can
   report `pending` forever even though it connects successfully
   afterward — moved every MCP server to a standing HTTP daemon,
   health-checked before running.
4. **`grade.py` didn't skip already-graded pairs (old bug):** the first
   version re-graded the entire dataset on every run and only wrote
   output at the very end — with 100+ rows this looked like a hang for
   tens of minutes even though it was working correctly. Fixed: skip
   `(task_id, arm)` pairs that already have a grade, write incrementally
   after each one (`--regrade` forces a full re-grade).
5. **Audit-rule quote matching was too strict (old bug):** exact-character
   comparison downgraded correct verdicts to `missing` whenever the quote
   matched in content but not in surrounding markdown (backticks/`**`) —
   in one case this scored a complete, correct answer at 0%. Fixed:
   strip markdown before comparing.
6. **Idempotent runs are per `task_id`, not per file:** `run.py --task
   Q01` only drops/rewrites the Q01 row, every other task's row is left
   untouched — avoids re-spending when only 1-2 tasks need a retry.
7. **Pre-registration:** always `git commit` the question set
   (`tasks/erpnext_tasks.yaml`) BEFORE running the agent under test —
   guards against editing questions after seeing results
   (cherry-picking).
8. **Piping `run.py` through `tee` block-buffers stdout — a live run's
   log can look empty/dead for many minutes even though it's working.**
   This caused an accidental duplicate paid run (two `run.py` invocations
   for the same arm/tasks running concurrently, both completing
   successfully, wasting the second one's cost). Before assuming a run
   died and relaunching: check `ps aux | grep harness/run.py` AND
   confirm the target `results/<arm>-results.jsonl` isn't actively
   growing new rows. Prefer `python3 -u harness/run.py ... > logfile
   2>&1` (unbuffered, direct redirect) over piping through `tee`. If a
   duplicate run does happen, `results/<arm>-results.jsonl` can end up
   with two rows for the same `task_id` — dedupe by keeping the last
   occurrence per id before grading.
9. **Don't rely on a per-invocation ledger for spend tracking** — a file
   that gets overwritten on every `run.py` call only reflects the most
   recent run for that arm, not a cumulative record, which is misleading
   by construction. Derive cost/token totals from the append-only
   `*-results.jsonl` files instead.
10. **Never `grade.py`/`report.py` two different `<model>` directories
    together** — their keys are `(task_id, arm)`, not
    `(task_id, arm, model)`, so merging silently conflates same-named arms
    run under different models (see "Running an alternate agent provider").
    Concretely: `results/sot/<label>/claude-sonnet-5-5/rep1/` (the 3 tool arms)
    and `results/hard/claude-code/claude-sonnet-5/` (the `baseline` arm) are
    already split this way — `results/` used to hold all 4 arms flat and
    a `grade.py`/`report.py` run with no directory scoping merged a
    `baseline`/Sonnet smoke-test run straight into the canonical
    Opus-4.8 `grades.json`/`RESULTS.md` before this was caught and fixed
    by moving everything to `results/<question-set>/<agent>/<model>/`.
11. **`--tasks-file` must be paired with `--out-dir`/`--results-dir`
    pointed at a distinct location** — otherwise a second question set's
    rows land in the default question set's directory and get graded
    against the wrong `key_facts` (or, if ids happen to collide, silently
    overwrite rows). The easy-set experiment below uses `E01`-`E30` ids
    specifically to make an accidental cross-set mixup obvious even if the
    directory scoping were ever done wrong.

## Running an alternate question set

`--tasks-file <path>` (same flag name on `run.py`, `grade.py`,
`report.py`) swaps in a different YAML file with the same schema as
`tasks/erpnext_tasks.yaml` (`id`, `group`, `group_title`, `origin_note`,
`question`, `key_facts`, `anchor_files`). Always pair it with
`--out-dir`/`--results-dir` — see pitfall #11.

Real example: `tasks/erpnext_tasks_easy.yaml` (ids `E01`-`E30`) — a
single-file/single-hop/keyword-overlapping question set (deliberate
opposite of the main set's group C), authored to test whether an easier
question set produces the token savings from MCP tools that a harder set
doesn't reliably show. Results at
`results/easy/claude-code/claude-sonnet-5/`; analysis in `results/RESULTS.md` §7.
Finding: no — the easier set showed *larger* relative token overhead from
tools (+27% to +196% vs. baseline, compared to the hard set's -9% to
+45%), because a tool call's largely-fixed payload cost is a bigger
fraction of an already-minimal baseline. Don't assume "easier questions
save more tokens" without checking this.
