# graphrelay source-of-truth benchmark

The one benchmark that counts for graphrelay. A real coding agent (Claude Code CLI) answers hard
questions about **ERPNext** (~1M LOC, frappe/erpnext pinned at
`1887825ce5c6fe2fbd5366ab52e4c73de24d23fa`) and a blind judge scores key-fact coverage:
`(covered + 0.5 × partial) / total`, verbatim quotes required (non-verbatim counts as missing).

- **Arms:** `graphrelay` (Grep/Read/Glob + our MCP server) and `baseline` (Grep/Read/Glob only, the no-tool
  control). Optional arms added 2026-10-01 for comparison: `graphify`, `codegraph-colby` (competitors, `setup/setup-arms.sh`), and
  `graphrelay-jev` / `graphrelay-laya` (our own graphrelay with a relevance judge on; each its own daemon and checkout,
  Laya needs `harness/laya-venv`). Select with `run-sot.sh --arms "graphrelay baseline graphify codegraph-colby graphrelay-jev graphrelay-laya"`.
- **Agents:** `claude-sonnet-5` (real Anthropic API) and `openrouter-glm-5-3-flash` (OpenRouter, through the
  local LiteLLM proxy in `harness/proxy`). Both defined in `harness/providers.yaml`.
- **Judge:** `claude-sonnet-5`, always, regardless of the agent (`harness/grade.py`).
- **Questions:** `tasks/erpnext_tasks.yaml` holds 30 (6 capability groups × 5). The SOT set is frozen in
  `tasks/sot_questions.txt`: the first of each group (Q01, Q06, Q11, Q16, Q21, Q26).
- **Fairness floor:** 30 turns, Grep/Read/Glob for every arm; `run.py` disallows every other built-in tool
  (an audit of the transcripts confirms nothing else was used).

## Run it

```bash
bash setup/setup-erpnext.sh                       # once: clone ERPNext at the pin into repos/erpnext
benchmarks/dataset/run-sot.sh --dry-run           # check prerequisites, spend nothing
benchmarks/dataset/run-sot.sh --repeats 3         # both agents, both arms, 3 repeats per cell
benchmarks/dataset/run-sot.sh --models claude-sonnet-5 --label my-change   # one agent only
```

`run-sot.sh` builds `cargo build --release`, indexes `repos/erpnext` into an **isolated `$HOME`** (your
real `~/.graphrelay` and daemon are untouched; port 3030 must be free), serves it for the MCP config in
`harness/mcp/graphrelay.json`, starts the LiteLLM proxy on :4000 when GLM is requested, runs every
model × arm × repeat, grades, writes `RESULTS.md`, and cleans up. Output:
`results/sot/<label>/<model>/rep<k>/` (per-arm results and ledgers, transcripts, `grades.json`,
`RESULTS.md`, `RUN.txt` recording the commit under test), plus `results/sot/<label>/SUMMARY.md`
(per-repeat and pooled coverage, turn-cap counts, per-question means, paired bootstrap).

**Prerequisites:** the `claude` CLI on PATH (agent runner) and logged in or `ANTHROPIC_API_KEY`; Python 3
with PyYAML (and `scipy` for a real McNemar p-value, optional); Rust toolchain. For the GLM agent also an
`OPENROUTER_API_KEY` (environment, or `harness/.env.openrouter`, which is gitignored); without it the
runner skips GLM and says so. The proxy venv lives in `harness/proxy/.venv` (gitignored; create it with a
Python 3.11+ interpreter and `pip install 'litellm[proxy]==1.100.0'` — never an unpinned litellm).
The `openrouter-glm-5-3-flash` prices in `providers.yaml` are from a promo that ended 2026-09-09 and
only affect the cost ledger; re-check them.

## Reading the results honestly

- **Sample size.** The frozen set is 6 questions; one run per cell can only reveal large differences.
  Use `--repeats 3` (Sonnet costs about $1.6 per arm-run, GLM pennies) and, for any claim about a small
  effect, more questions. `harness/report.py` prints a McNemar test on pass/fail (coverage ≥ 75%); at n = 6
  it has almost no power.
- **Agent noise.** The agent and the judge are non-deterministic; the same question can differ by tens of
  points between runs. Compare repeats, not single runs.
- **Turn-budget failures.** An agent that hits the turn cap without a final answer is scored 0% (a real
  result, not a harness bug). GLM 5.3 Flash converges less often than Claude with tools on.
- **Reference runs.**
  - `results/sot/2026-09-30/claude-sonnet-5/rep1/` (with `PREREGISTRATION.md`): Sonnet 5, 6 questions,
    1 repeat, release build of `1f9038d`: graphrelay 70.8% vs baseline 65.3% coverage, 11.2 vs 15.7 tool
    calls per question. n = 6, one run: not distinguishable. It predates the two-arm trim, so it also
    contains graphify and code-review-graph rows; ignore them.
  - `results/sot/2026-09-30-glm/` (`SUMMARY.md`): GLM 5.3 Flash, 6 questions × 3 repeats, release build of
    `d3af13d`: graphrelay **74.8%** vs baseline **53.7%** pooled coverage (+21 pts; question-level 95% CI
    [+2.8, +40.0]), and the baseline hit the turn cap with no answer in 5 of 18 runs versus 0 of 18 for
    graphrelay. Much of that gap is "reaches a final answer within 30 turns" (Q01 and Q21); on questions
    both arms answer the difference is small (Q06 −4, Q16 +3, Q26 0). Repeats differ by up to 24 points
    for the baseline (38.5–62.5%), 13 for graphrelay (68.1–80.9%).

## Layout

```
run-sot.sh              one-command runner (see above)
tasks/                  erpnext_tasks.yaml (30 questions + key facts), sot_questions.txt (frozen SOT ids)
harness/run.py          runs one arm × provider over a task list (claude CLI, MCP config, spend ledger)
harness/grade.py        blind judge; harness/report.py, report_html.py  RESULTS.md / tool-flow HTML
harness/sot_summary.py  aggregates repeats into SUMMARY.md
harness/providers.yaml  the two agents;  harness/proxy/  LiteLLM proxy for GLM
harness/PIPELINE.md     methodology and the pitfalls we hit (parts about removed arms/providers are historical)
setup/setup-erpnext.sh  clone ERPNext at the pin
results/sot/            reference and future runs
```

## History

Everything that used to live here (the competitor arms graphify / code-review-graph / colby / no-semantic,
the DeepSeek, Ollama, Gemini, Opus and OpenCode runs, the "easy" question set, the indexing-speed
comparison, and ~230 MB of results) was retired on 2026-09-30 to keep one benchmark. It is recoverable
from git history at commit `5d413ea`.
