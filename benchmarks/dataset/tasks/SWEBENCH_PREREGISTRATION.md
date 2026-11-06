# SWE-bench Verified pilot: preregistration (DRAFT, not yet run)

Status: **draft, written 2026-10-29. Nothing has been run.** Freeze it (commit, then run) only after the owner approves the spend. Anything learned after the first model call is appended under "Deviations", never edited in.

## Why this exists

The structural slices (ERPNext, Gitea) measure whether an agent can *list callers*. The baseline already scores about 99% on Gitea, so that question type cannot show a lift. SWE-bench measures a different outcome: whether the agent produces a patch that passes the official hidden tests. Cost and tool use count against that outcome. The method (official grader, whole-task cost, immutable arms, selection frozen before looking at solutions) follows `dzhng/jevgrep`'s `evals/` policy. Their harness is not reused: it is tied to their `jg` package, the Vercel AI Gateway and a Codex agent, and their baselines are not in the repository.

## What this can and cannot show

10 tasks is a **pilot**. With 10 paired tasks and one repeat, only a large difference in solves would be visible, and cost is noisy. The pilot is allowed to produce: (a) paired solve outcomes, (b) cost per attempted and per solved task, (c) tool-call counts, and (d) a decision on whether a larger slice is worth paying for. It may not support a claim that GraphRelay improves or does not improve SWE-bench resolution.

## Tasks (frozen before running)

- Source: `princeton-nlp/SWE-bench_Verified`, test split, 500 instances. Sorted instance ids joined by newlines have sha256 `fad0fdea4fc2315e9b78cdf80882a32e32393297052e502e0e63c79ad648fb85`.
- Selection: `random.seed(27)`; `random.sample(sorted_ids, 10)`. No filtering by repository, difficulty or by looking at patches. Python 3.10 `random`.
- The 10 tasks: `pytest-dev__pytest-10356`, `django__django-16901`, `scikit-learn__scikit-learn-13142`, `django__django-14053`, `sympy__sympy-23413`, `django__django-14170`, `django__django-13128`, `django__django-11163`, `django__django-11133`, `django__django-13810`.
- Composition is 7 Django and one each of pytest, scikit-learn and sympy, by chance. This is disclosed, not corrected. Difficulty labels: 3 under 15 min, 5 in 15 min to 1 hour, 2 in 1 to 4 hours.
- Gold patches and hidden tests stay outside the agent's workspace.
- A task whose environment fails to build is reported as an infrastructure failure for **both** arms. It is not swapped for another task.

## Arms and agents

- `graphrelay`: Claude Code CLI with the GraphRelay MCP server (4 tools) indexed on the task's checkout at `base_commit`, in an isolated `$HOME`. Floor tools (Read, Grep, Glob, Bash edit/test) stay available.
- `baseline`: the same agent, same prompt, floor tools only.
- Agent: `claude-sonnet-5` (primary). GLM 5.3 Flash via the LiteLLM proxy is optional and reported separately if run.
- Same prompt, turn cap, timeout and environment in both arms. The prompt gives the issue text and asks for a patch. It does not require using the tool (the production skill stays selective), and it does not mention GraphRelay in the baseline arm.
- One attempt per task per arm (no pass@k, no best-of-N). Every attempt is kept, including timeouts and failures.

## Harness parameters (draft, fixed before the first paid call)

Implemented in `benchmarks/dataset/swebench/swebench.py` (`export`, `prepare`, `run`, `grade`, `account`). Both arms: tools Read, Grep, Glob, Edit, Write, Bash; 60 turns; 1,800 s timeout; clean arm-neutral system prompt; no user settings loaded. The checkout is the upstream repository at `base_commit` with its git history removed (one local commit), so the fix commit is not reachable. The patch is `git diff HEAD` of the checkout after the run. `graphrelay` indexes each checkout into an isolated `$HOME` on port 3050.

Known limitation, disclosed: the agent works on a host checkout **without the task's Python environment**, so neither arm can run the project's tests. Absolute solve rates will be lower than published SWE-bench numbers. This affects both arms equally but may understate what an agent with a working test loop can do. Running inside the official per-task images would remove it and is the main reason to prefer a Linux x86 host.

Verified without model calls: task export against the dataset hash, checkout without history, patch extraction, daemon start/stop in an isolated home, and the run -> predictions -> account flow with a stub `claude`. Review fixes verified the same way: both arms' patches contain only the agent's edits (the `.gitignore` line `gr init` adds and the `.graphrelay/` directory are excluded), and a retry resets the checkout to its single base commit instead of resuming from a half-edited tree.

Second known limitation, disclosed: both arms get Bash, and the harness only blocks obvious network commands (`curl`, `wget`, `git fetch/pull/clone/remote`, `pip`) and tells the agent not to look for the upstream fix. An agent could still reach the network through an interpreter, and the agent runs on the host with the operator's real `$HOME`. Any task whose transcript shows an attempt to fetch the fix is reported as contaminated for both arms' comparison. A sandboxed container host would remove both gaps. **Not verified:** `grade` (the `swebench` package is not installed here) and a real agent run.

## Outcome and accounting

- **Primary outcome:** official SWE-bench behavioral grade (`FAIL_TO_PASS` and `PASS_TO_PASS`) from the pinned official harness. Not the judge, not our coverage metric.
- **Cost:** the agent's whole task cost, including exploration, edits, verification and failed attempts, from the ledger. GraphRelay has no per-query charge, so tool cost is $0, and index build time is reported separately as wall time. Unknown cost is unknown, never zero.
- Reported per task and as cohort totals: solves, cost per attempted task, **total spend divided by solved tasks**, tool calls, turns. Timing is diagnostic only.
- Paired comparison per task (both solved, only graphrelay, only baseline, neither). Cost differences use a question-level bootstrap and are labelled exploratory.

## Decision rule

There is no pass/fail on efficacy. The pilot is read like this, in advance:

1. If an arm shows environment or harness errors on more than 2 tasks, the pilot is void for that arm and is fixed, not interpreted.
2. If both arms solve the same set of tasks or differ by 1, the solve outcome is a tie. Then the only reportable finding is cost and tool calls, exploratory.
3. If graphrelay solves 3 or more tasks than baseline, or baseline solves 3 or more than graphrelay, that is a signal worth a larger slice (at least 50 tasks, 2 repeats). It is still not a claim.
4. No outcome changes the README's current wording until a larger slice is run.

## Before running (owner decisions)

- **Spend.** Estimated at roughly $0.4 to $1.5 per task per arm for Sonnet, so about $10 to $30 for 10 tasks x 2 arms x 1 repeat. This estimate is not measured. A `--max-spend` cap is set on the run.
- **Where it runs.** The official images are `linux/amd64` and large. This machine is arm64 with about 28 GB free, so building or emulating 10 task images may not fit or may be slow. A Linux x86 machine or a cloud runner is the safer place.
- **Not started until** Plan 27's structural runs are finished and committed.

## Deviations

(none yet)
