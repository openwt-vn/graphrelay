#!/usr/bin/env python3
"""Source-of-truth benchmark runner for graphrelay on ERPNext.

An LLM agent (Claude Code CLI) answers hard questions about the ERPNext codebase with a
Grep/Read/Glob floor plus, for the `graphrelay` arm, our MCP server; the `baseline` arm has the
floor only and is the no-tool control. Answers are graded by a blind judge (grade.py).
Agent models are the providers in harness/providers.yaml: `claude-sonnet-5` (default, the
real Anthropic API) and `openrouter-glm-5-3-flash` (through the local LiteLLM proxy in
harness/proxy; needs OPENROUTER_API_KEY). The whole flow is driven by ../run-sot.sh.

Usage:
    python3 harness/run.py --arm graphrelay --task Q01,Q06 --out-dir results/sot/<run>/<model>
    python3 harness/run.py --arm baseline  --provider openrouter-glm-5-3-flash

Arms: graphrelay | baseline
Without --out-dir, writes results/hard/claude-code/<model>/<arm>-results.jsonl, per-task
transcripts and a spend ledger; pass --out-dir for anything you want kept separate.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
REPO_NAME = os.environ.get("BENCH_REPO", "erpnext")      # run-sot.sh --repo; default keeps ERPNext behaviour
REPO_LABEL = os.environ.get("BENCH_REPO_LABEL", "ERPNext")
ERPNEXT = ROOT / "repos" / REPO_NAME
# colby's CLI also writes a `.codegraph/` dir inside the project root; it is kept apart from ours;
# it gets its own byte-identical checkout (same pin) and an isolated install prefix.
COLBY_ERPNEXT = ROOT / "repos" / "erpnext-colby"
# graphify's graph (graphify-out/, ~100 MB, 3,000+ files) lives in its own checkout for the same reason:
# inside repos/erpnext it was indexed by graphrelay and readable by the baseline agent.
GRAPHIFY_ERPNEXT = ROOT / "repos" / "erpnext-graphify"
# The judge arms run our own graphrelay with a relevance judge enabled in the project's own
# .graphrelay/config.toml, so each needs a byte-identical checkout (own config) and its own daemon.
JEV_ERPNEXT = ROOT / "repos" / "erpnext-jev"
LAYA_ERPNEXT = ROOT / "repos" / "erpnext-laya"
TASKS_FILE = ROOT / "tasks" / "erpnext_tasks.yaml"
RESULTS_DIR = ROOT / "results"
MCP_DIR = ROOT / "harness" / "mcp"
PROVIDERS_FILE = ROOT / "harness" / "providers.yaml"

# Only agent CLI this runner drives today. Kept as an explicit constant
# (rather than a --agent flag) so the results/<question-set>/<agent>/<model>/
# layout is ready for a second runner (e.g. results/hard/opencode/, see
# harness/opencode_run.py) without implying one is wired up yet.
AGENT_NAME = "claude-code"

# Default --out-dir root when --tasks-file is left at its default
# (tasks/erpnext_tasks.yaml). Question sets are siblings under results/ —
# e.g. results/hard/claude-code/<model>/ vs results/easy/claude-code/<model>/
# (tasks/erpnext_tasks_easy.yaml) — an alternate --tasks-file always needs
# an explicit --out-dir (see harness/PIPELINE.md pitfall #11), this default
# only covers the un-overridden case.
DEFAULT_QUESTION_SET = "hard"

MAX_TURNS = 30

# Tool floor: grep / read / list — identical for every arm.
FLOOR_TOOLS = ["Grep", "Read", "Glob"]


def load_providers() -> dict:
    return yaml.safe_load(PROVIDERS_FILE.read_text())


ARMS = {
    "baseline": {"mcp": "empty.json", "mcp_tools": []},
    "graphrelay": {"mcp": "graphrelay.json", "mcp_tools": ["mcp__graphrelay__*"]},
    "graphify": {"mcp": "graphify.json", "mcp_tools": ["mcp__graphify__*"], "repo_dir": GRAPHIFY_ERPNEXT},
    "graphrelay-jev": {
        "mcp": "graphrelay-jev.json",
        "mcp_tools": ["mcp__graphrelay-jev__*"],
        "repo_dir": JEV_ERPNEXT,
    },
    "graphrelay-laya": {
        "mcp": "graphrelay-laya.json",
        "mcp_tools": ["mcp__graphrelay-laya__*"],
        "repo_dir": LAYA_ERPNEXT,
    },
    "codegraph-colby": {
        "mcp": "codegraph-colby.json",
        "mcp_tools": ["mcp__codegraph-colby__*"],
        "repo_dir": COLBY_ERPNEXT,
    },
}


def repo_dir_for(arm: str) -> Path:
    return ARMS[arm].get("repo_dir", ERPNEXT)

# Full replacement system prompt (not an append) — Claude Code's default
# system prompt pulls in the operator's CLAUDE.md, hooks, and persona
# instructions (e.g. terse "caveman mode"), which would silently bias
# answer style and tool choice differently per arm. A clean, arm-neutral
# prompt is required for a fair comparison.
SYSTEM_PROMPT = """\
You are a coding agent answering a question about the {label} codebase, \
located at {repo}. You have exactly these tools: Grep, Read, Glob{mcp_note}

Investigate using your tools, then give one complete, specific final answer \
naming the relevant functions, classes and files. Do not ask clarifying \
questions — investigate and answer directly. You have at most {max_turns} \
turns; use them efficiently."""

MCP_NOTE = (
    ", plus the code-intelligence tools under `mcp__{arm}__*` — these are "
    "your primary tools for understanding code structure, relationships and "
    "call graphs. Before your first Grep or Read call, call at least one "
    "`mcp__{arm}__*` tool relevant to the question. Only reach for Grep/Read "
    "afterward, and only for what the code-intelligence tools didn't surface "
    "(e.g. reading a specific line range one of them pointed you to). The one "
    "legitimate exception: a question that asks you to enumerate every "
    "literal string/pattern occurrence across many files (not a structural "
    "or relationship question) may start with Grep — but a question that "
    "looks keyword-heavy is not automatically that case; check what it's "
    "really asking before treating Grep as the shortcut."
)


def load_tasks(task_filter: str | None, tasks_file: Path = TASKS_FILE) -> list[dict]:
    """task_filter accepts a single id (E01) or a comma-separated list
    (E07,E08,E09) — comma form lets a partial re-run target exactly the
    tasks that need it without touching already-good rows for other tasks."""
    tasks = yaml.safe_load(tasks_file.read_text())["tasks"]
    if task_filter:
        wanted = set(task_filter.split(","))
        tasks = [t for t in tasks if t["id"] in wanted]
        missing = wanted - {t["id"] for t in tasks}
        if missing:
            sys.exit(f"No task(s) with id: {', '.join(sorted(missing))}")
    return tasks


def materialize_mcp_config(arm: str) -> Path:
    """Substitute placeholders and write a temp config."""
    src = MCP_DIR / ARMS[arm]["mcp"]
    text = (src.read_text()
            .replace("__ERPNEXT_COLBY__", str(COLBY_ERPNEXT))
            .replace("__ERPNEXT__", str(ERPNEXT))
            .replace("/erpnext/mcp", f"/{REPO_NAME}/mcp")
            .replace("__HOME__", str(Path.home())))
    dst = RESULTS_DIR / f".mcp-{arm}.json"
    dst.write_text(text)
    return dst


def run_agent(arm: str, question: str, mcp_config: Path, provider: dict) -> dict:
    model = provider["model"]
    repo = repo_dir_for(arm)
    allowed = FLOOR_TOOLS + ARMS[arm]["mcp_tools"]
    mcp_note = MCP_NOTE.format(arm=arm) if ARMS[arm]["mcp_tools"] else "."
    system_prompt = SYSTEM_PROMPT.format(
        repo=repo, label=REPO_LABEL, mcp_note=mcp_note, max_turns=MAX_TURNS)

    # Every built-in/plugin tool NOT in the floor, listed explicitly.
    # `claude -p` inherits the full local tool registry (Bash, Edit, Write,
    # Artifact, Task, Workflow, ToolSearch, Skill, ...) by default;
    # --allowedTools only ADDS to that set, it does not replace it — so the
    # floor must be enforced via an exhaustive --disallowedTools instead.
    disallowed = [
        "Bash", "Write", "Edit", "NotebookEdit", "WebSearch", "WebFetch",
        "Task", "Agent", "Artifact", "AskUserQuestion", "TodoWrite",
        "ExitPlanMode", "EnterPlanMode", "Workflow", "ToolSearch", "Skill",
        "ScheduleWakeup", "CronCreate", "CronDelete", "CronList",
        "DesignSync", "EnterWorktree", "ExitWorktree", "LSP", "Monitor",
        "PushNotification", "RemoteTrigger", "ReportFindings", "SendMessage",
        "TaskOutput", "TaskStop",
        # `Task`/`Agent` above were split into these newer tool names in a
        # later Claude Code CLI release (observed with claude_code_version
        # 2.1.259) — the old names alone silently let a local/weak model
        # discover and hammer TaskList/ListAgents instead of the intended
        # Grep/Read/Glob floor (root-caused during the 2026-09-07 Ollama
        # smoke test: 12-24 TaskList calls per run on a *baseline*, no-MCP
        # arm). Not Ollama-specific — this floor leak applies to every arm
        # and provider on this CLI version, so list every renamed variant
        # explicitly rather than relying on the old "Task"/"Agent" names.
        "TaskCreate", "TaskGet", "TaskList", "TaskUpdate", "ListAgents",
        # Generic MCP resource-browsing tools Claude Code adds automatically
        # once any MCP server is configured — not part of the tool floor.
        "ListMcpResourcesTool", "ReadMcpResourceDirTool", "ReadMcpResourceTool",
    ]

    cmd = [
        "claude",
        "-p", question,
        "--model", model,
        "--max-turns", str(MAX_TURNS),
        "--output-format", "stream-json",
        "--verbose",
        "--mcp-config", str(mcp_config),
        "--strict-mcp-config",
        "--allowedTools", ",".join(allowed),
        "--disallowedTools", ",".join(disallowed),
        "--system-prompt", system_prompt,
        # Load no user/project/local settings — hooks, permissions, and
        # persona rules from the operator's ~/.claude and repo CLAUDE.md
        # must not leak into a fairness-critical benchmark run.
        "--setting-sources", "",
        "--add-dir", str(repo),
    ]

    # Only override ANTHROPIC_BASE_URL/auth for a non-default provider, and
    # only in a copy passed to this one subprocess — os.environ of this
    # (parent) run.py process is never mutated, so a later grade.py call in
    # the same shell can never inherit a provider override meant for the
    # agent. Uses ANTHROPIC_AUTH_TOKEN, not ANTHROPIC_API_KEY: Claude Code
    # treats ANTHROPIC_API_KEY (and an interactive claude.ai login) as
    # higher-precedence than a custom base URL's credential, so setting
    # ANTHROPIC_API_KEY here gets silently ignored in favor of the
    # operator's own Anthropic auth (observed as a 401 authentication_failed
    # loop against a third-party base_url) — ANTHROPIC_AUTH_TOKEN is the
    # mechanism Claude Code actually uses for a custom ANTHROPIC_BASE_URL.
    # ANTHROPIC_API_KEY is explicitly popped from the copy in case it's set
    # in the ambient environment, so it can't win that precedence fight.
    env = None
    if provider.get("base_url"):
        key_var = provider["api_key_env"]
        if key_var not in os.environ:
            sys.exit(f"provider requires env var {key_var} to be set "
                      f"(see harness/providers.yaml)")
        env = dict(os.environ)
        env.pop("ANTHROPIC_API_KEY", None)
        env["ANTHROPIC_BASE_URL"] = provider["base_url"]
        env["ANTHROPIC_AUTH_TOKEN"] = os.environ[key_var]

    # MCP servers connect asynchronously; the `system/init` event can be
    # emitted with a server still `pending`, meaning that arm's tools may be
    # entirely absent for the run's early turns. Retry a few times when that
    # happens rather than silently grading a degraded (near-baseline) run.
    max_attempts = 3
    for attempt in range(1, max_attempts + 1):
        t0 = time.time()
        proc = subprocess.run(
            cmd, capture_output=True, text=True, cwd=str(repo), timeout=1800,
            env=env,
        )
        elapsed = time.time() - t0

        events = []
        for line in proc.stdout.splitlines():
            line = line.strip()
            if line.startswith("{"):
                try:
                    events.append(json.loads(line))
                except json.JSONDecodeError:
                    pass

        if not mcp_servers_connected(events) and attempt < max_attempts:
            print(f"    MCP server not connected at init (attempt "
                  f"{attempt}/{max_attempts}) — retrying...")
            time.sleep(3)
            continue
        break

    return parse_events(events, elapsed, proc.returncode, proc.stderr[-2000:],
                        provider["pricing"])


def mcp_servers_connected(events: list[dict]) -> bool:
    for ev in events:
        if ev.get("type") == "system" and ev.get("subtype") == "init":
            servers = ev.get("mcp_servers", [])
            if not servers:
                return True  # baseline arm — no MCP servers expected
            return all(s.get("status") == "connected" for s in servers)
    return False


def parse_events(events: list[dict], elapsed: float, rc: int, stderr: str, price: dict) -> dict:
    final_text = ""
    tool_calls = 0
    tool_names: dict[str, int] = {}
    usage = {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0}
    num_turns = 0

    for ev in events:
        etype = ev.get("type")
        if etype == "assistant":
            msg = ev.get("message", {})
            for block in msg.get("content", []):
                if block.get("type") == "tool_use":
                    tool_calls += 1
                    name = block.get("name", "?")
                    tool_names[name] = tool_names.get(name, 0) + 1
            u = msg.get("usage", {})
            usage["input"] += u.get("input_tokens", 0)
            usage["output"] += u.get("output_tokens", 0)
            usage["cache_read"] += u.get("cache_read_input_tokens", 0)
            usage["cache_write"] += u.get("cache_creation_input_tokens", 0)
        elif etype == "result":
            final_text = ev.get("result", "") or final_text
            num_turns = ev.get("num_turns", 0)
            u = ev.get("usage", {})
            if u:  # result carries authoritative cumulative usage when present
                usage["input"] = u.get("input_tokens", usage["input"])
                usage["output"] = u.get("output_tokens", usage["output"])
                usage["cache_read"] = u.get(
                    "cache_read_input_tokens", usage["cache_read"]
                )
                usage["cache_write"] = u.get(
                    "cache_creation_input_tokens", usage["cache_write"]
                )

    cost = (
        usage["input"] / 1e6 * price["in"]
        + usage["output"] / 1e6 * price["out"]
        + usage["cache_read"] / 1e6 * price["cache_read"]
        + usage["cache_write"] / 1e6 * price["cache_write"]
    )
    total_tokens = sum(usage.values())

    return {
        "final_text": final_text,
        "tool_calls": tool_calls,
        "tool_names": tool_names,
        "num_turns": num_turns,
        "usage": usage,
        "total_tokens": total_tokens,
        "cost_usd": round(cost, 4),
        "elapsed_s": round(elapsed, 1),
        "returncode": rc,
        "stderr_tail": stderr if rc != 0 else "",
        "events": events,
    }


def main() -> None:
    providers = load_providers()

    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", required=True, choices=list(ARMS))
    ap.add_argument("--task", help="task id or comma-separated ids, e.g. E01 or E07,E08,E09")
    ap.add_argument("--max-spend", type=float, default=30.0,
                    help="abort when cumulative USD spend exceeds this")
    ap.add_argument("--provider", default="claude-sonnet-5", choices=list(providers),
                    help="agent model provider from harness/providers.yaml "
                         "(default claude-sonnet-5 against the real Anthropic "
                         "API; openrouter-glm-5-3-flash goes through the "
                         "local LiteLLM proxy and needs OPENROUTER_API_KEY).")
    ap.add_argument("--model", default=None,
                    help="override the provider's default model string "
                         "(e.g. deepseek-reasoner under the deepseek "
                         "provider's proxy) without editing providers.yaml. "
                         "Pricing still follows the provider's table.")
    ap.add_argument("--out-dir", default=None,
                    help="output root for results/ledger/transcripts "
                         "(default: results/hard/claude-code/<model>/ — "
                         "every question-set/model combination gets its "
                         "own directory so runs never mix)")
    ap.add_argument("--tasks-file", default=str(TASKS_FILE),
                    help="alternate question set, same YAML schema as "
                         "tasks/erpnext_tasks.yaml (default). Pair with "
                         "--out-dir so a second question set's results "
                         "never mix with the default set's.")
    args = ap.parse_args()

    provider = dict(providers[args.provider])
    if args.model:
        provider["model"] = args.model

    model_dir_name = provider["model"].replace("/", "_")
    out_dir = Path(args.out_dir) if args.out_dir else (
        RESULTS_DIR / DEFAULT_QUESTION_SET / AGENT_NAME / model_dir_name)
    transcripts_dir = out_dir / "transcripts"

    RESULTS_DIR.mkdir(exist_ok=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    transcripts_dir.mkdir(exist_ok=True)
    tasks = load_tasks(args.task, Path(args.tasks_file))
    mcp_config = materialize_mcp_config(args.arm)
    results_file = out_dir / f"{args.arm}-results.jsonl"
    ledger_file = out_dir / f"{args.arm}-ledger.json"

    # Idempotent per task_id: drop any prior row(s) for a task before
    # (re-)running it, so retrying a single task never leaves duplicates.
    task_ids_to_run = {t["id"] for t in tasks}
    existing_rows = []
    if results_file.exists():
        for line in results_file.read_text().splitlines():
            if line.strip():
                row = json.loads(line)
                if row["task_id"] not in task_ids_to_run:
                    existing_rows.append(row)
    with results_file.open("w") as f:
        for row in existing_rows:
            f.write(json.dumps(row) + "\n")

    ledger = {"arm": args.arm, "provider": args.provider, "model": provider["model"],
              "base_url": provider.get("base_url"), "pricing": provider["pricing"],
              "max_turns": MAX_TURNS, "runs": [], "total_cost_usd": 0.0,
              "total_tokens": 0}
    spend = 0.0

    print(f"arm={args.arm} provider={args.provider} model={provider['model']} "
          f"max_turns={MAX_TURNS} tasks={len(tasks)} max_spend=${args.max_spend}")

    for task in tasks:
        tid = task["id"]
        if spend >= args.max_spend:
            print(f"max-spend ${args.max_spend} reached — stopping before {tid}")
            break
        print(f"\n--- {tid}: {task['question'][:90]}...")
        r = run_agent(args.arm, task["question"], mcp_config, provider)
        spend += r["cost_usd"]

        transcript_path = transcripts_dir / f"{args.arm}-{tid}.json"
        transcript_path.write_text(json.dumps(r["events"], indent=1))

        row = {
            "task_id": tid,
            "arm": args.arm,
            "model": provider["model"],
            "question": task["question"],
            "answer": r["final_text"],
            "tool_calls": r["tool_calls"],
            "tool_names": r["tool_names"],
            "num_turns": r["num_turns"],
            "max_turns_cap": MAX_TURNS,
            "usage": r["usage"],
            "total_tokens": r["total_tokens"],
            "cost_usd": r["cost_usd"],
            "elapsed_s": r["elapsed_s"],
            "returncode": r["returncode"],
            "stderr_tail": r["stderr_tail"],
        }
        with results_file.open("a") as f:
            f.write(json.dumps(row) + "\n")

        ledger["runs"].append({"task_id": tid, "cost_usd": r["cost_usd"],
                               "tokens": r["usage"]})
        ledger["total_cost_usd"] = round(spend, 4)
        ledger["total_tokens"] += r["total_tokens"]
        ledger_file.write_text(json.dumps(ledger, indent=2))

        print(f"    turns={r['num_turns']} tool_calls={r['tool_calls']} "
              f"tokens={r['total_tokens']:,} cost=${r['cost_usd']} "
              f"({r['elapsed_s']}s) rc={r['returncode']}")
        if not r["final_text"]:
            print(f"    WARNING: empty answer. stderr: {r['stderr_tail'][:300]}")

    print(f"\nDone. spend=${spend:.2f}  results={results_file}")


if __name__ == "__main__":
    main()
