#!/usr/bin/env python3
"""SWE-bench Verified pilot harness (see tasks/SWEBENCH_PREREGISTRATION.md).

Subcommands: export | prepare | run | grade | account. `export`, `prepare`, `run --dry-run`
and `account` make no model calls. `run` (without --dry-run) is the only paid step.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path

DATASET_DIR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(DATASET_DIR / "harness"))
import run as bench_run  # noqa: E402  (reuses provider loading, stream parsing, MCP-connected check)

REPO_ROOT = DATASET_DIR.parent.parent
GR = REPO_ROOT / "target" / "release" / "gr"
TASKS_FILE = DATASET_DIR / "tasks" / "swebench_tasks.json"
DATASET = "princeton-nlp/SWE-bench_Verified"
IDS_SHA256 = "fad0fdea4fc2315e9b78cdf80882a32e32393297052e502e0e63c79ad648fb85"
SEED, N_TASKS = 27, 10
MAX_TURNS = 60
TIMEOUT_S = 1800
DAEMON_PORT = 3050

SYSTEM_PROMPT = """\
You are a software engineer fixing a bug in the repository at {repo}. You have these tools: \
Read, Grep, Glob, Edit, Write, Bash{mcp_note}

Read the issue, find the relevant code, and edit the files in place so the issue is resolved. \
Do not edit or add test files. Do not use the network or look for the upstream fix. Do not ask questions. Do not run git commands that change history. \
When you are finished, reply with a one-paragraph summary. Your change is whatever differs from the \
starting files. You have at most {max_turns} turns."""

MCP_NOTE = (", plus the code-intelligence tools under `mcp__graphrelay__*` (search by intent, inspect a "
            "symbol or file, trace impact, watch status). Use them or the other tools as you see fit.")

DISALLOWED = [
    "WebSearch", "WebFetch", "Task", "Agent", "Artifact", "AskUserQuestion", "TodoWrite", "ExitPlanMode",
    "EnterPlanMode", "Workflow", "ToolSearch", "Skill", "ScheduleWakeup", "CronCreate", "CronDelete",
    "CronList", "DesignSync", "EnterWorktree", "ExitWorktree", "LSP", "Monitor", "PushNotification",
    "RemoteTrigger", "ReportFindings", "SendMessage", "TaskOutput", "TaskStop", "NotebookEdit",
    "TaskCreate", "TaskGet", "TaskList", "TaskUpdate", "ListAgents",
    "ListMcpResourcesTool", "ReadMcpResourceDirTool", "ReadMcpResourceTool",
    # Keep the agent from fetching the upstream fix. Best effort only: Bash can still reach the network
    # through an interpreter (python urllib etc.); the preregistration discloses this.
    "Bash(curl:*)", "Bash(wget:*)", "Bash(git fetch:*)", "Bash(git pull:*)", "Bash(git clone:*)",
    "Bash(git remote:*)", "Bash(pip:*)", "Bash(pip3:*)",
]
ARMS = {"baseline": [], "graphrelay": ["mcp__graphrelay__*"]}


def sh(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


def curl_json(url: str):
    return json.loads(sh(["curl", "-sL", "--fail", url]).stdout)


def fetch_rows() -> list[dict]:
    rows = []
    for off in range(0, 500, 100):
        u = (f"https://datasets-server.huggingface.co/rows?dataset={DATASET.replace('/', '%2F')}"
             f"&config=default&split=test&offset={off}&length=100")
        rows += [r["row"] for r in curl_json(u)["rows"]]
    return rows


def cmd_export(a):
    rows = fetch_rows()
    ids = sorted(r["instance_id"] for r in rows)
    sha = hashlib.sha256("\n".join(ids).encode()).hexdigest()
    if len(rows) != 500 or sha != IDS_SHA256:
        sys.exit(f"dataset does not match the preregistered hash ({len(rows)} rows, {sha})")
    random.seed(SEED)
    pick = random.sample(ids, N_TASKS)
    by = {r["instance_id"]: r for r in rows}
    # No gold patch, no test patch, no FAIL_TO_PASS/PASS_TO_PASS: nothing the agent must not see.
    keep = ("instance_id", "repo", "base_commit", "problem_statement", "difficulty")
    TASKS_FILE.write_text(json.dumps([{k: by[i][k] for k in keep} for i in pick], indent=1) + "\n")
    print(f"wrote {TASKS_FILE} ({len(pick)} tasks): {', '.join(pick)}")


def load_tasks(sel: str | None) -> list[dict]:
    tasks = json.loads(TASKS_FILE.read_text())
    want = set(sel.split(",")) if sel else None
    out = [t for t in tasks if want is None or t["instance_id"] in want]
    if not out:
        sys.exit("no tasks selected (run `export` first, or check --task)")
    return out


def checkout_dir(work: Path, arm: str, iid: str) -> Path:
    return work / arm / iid


def reset_checkout(d: Path) -> None:
    """Return a prepared checkout to its single base commit (drops edits and index files from a prior attempt)."""
    sh(["git", "reset", "-q", "--hard", "HEAD"], cwd=d)
    sh(["git", "clean", "-fdxq"], cwd=d)
    if sh(["git", "rev-list", "--count", "HEAD"], cwd=d).stdout.strip() != "1":
        sys.exit(f"{d} is not a single-commit base checkout; delete it and re-run prepare")


def prepare_one(work: Path, arm: str, t: dict) -> Path:
    d = checkout_dir(work, arm, t["instance_id"])
    if (d / ".git").exists():
        reset_checkout(d)
        return d
    if d.exists():
        shutil.rmtree(d)  # a previous prepare died half way; never treat it as prepared
    d.mkdir(parents=True)
    try:
        url = f"https://github.com/{t['repo']}.git"
        sh(["git", "init", "-q"], cwd=d)
        sh(["git", "remote", "add", "origin", url], cwd=d)
        sh(["git", "fetch", "-q", "--depth", "1", "origin", t["base_commit"]], cwd=d)
        sh(["git", "checkout", "-q", "FETCH_HEAD"], cwd=d)
        # Drop the upstream history entirely: the fix commit must not be reachable from the agent's checkout.
        shutil.rmtree(d / ".git")
        sh(["git", "init", "-q"], cwd=d)
        sh(["git", "add", "-A"], cwd=d)
        sh(["git", "-c", "user.name=bench", "-c", "user.email=bench@localhost", "commit", "-q", "-m", "base"], cwd=d)
    except Exception:
        shutil.rmtree(d, ignore_errors=True)
        raise
    return d


def cmd_prepare(a):
    work = Path(a.workdir)
    for t in load_tasks(a.task):
        for arm in a.arms.split(","):
            d = prepare_one(work, arm, t)
            print(f"prepared {arm}/{t['instance_id']} at {d}")


def start_daemon(home: Path, d: Path, port: int) -> str:
    env = {**os.environ, "HOME": str(home)}
    for step in (["init"], ["index"]):
        subprocess.run([str(GR), *step], cwd=d, env=env, capture_output=True, text=True)
    subprocess.run([str(GR), "start", "--port", str(port)], cwd=d, env=env, capture_output=True, text=True)
    for _ in range(90):
        out = subprocess.run([str(GR), "status"], cwd=d, env=env, capture_output=True, text=True).stdout
        for line in out.splitlines():
            if line.startswith(d.name) and "fresh" in line:
                return line.split()[-1]
        time.sleep(2)
    sys.exit(f"graphrelay did not report {d.name} fresh")


def stop_daemon(home: Path, d: Path):
    subprocess.run([str(GR), "stop"], cwd=d, env={**os.environ, "HOME": str(home)}, capture_output=True)


def run_agent(arm: str, d: Path, issue: str, provider: dict, mcp_url: str | None) -> dict:
    mcp = {"mcpServers": {"graphrelay": {"type": "http", "url": mcp_url}}} if mcp_url else {"mcpServers": {}}
    cfg = d.parent / f".mcp-{arm}.json"
    cfg.write_text(json.dumps(mcp))
    system = SYSTEM_PROMPT.format(repo=d, max_turns=MAX_TURNS, mcp_note=MCP_NOTE if mcp_url else ".")
    cmd = ["claude", "-p", issue, "--model", provider["model"], "--max-turns", str(MAX_TURNS),
           "--output-format", "stream-json", "--verbose", "--mcp-config", str(cfg), "--strict-mcp-config",
           "--allowedTools", ",".join(["Read", "Grep", "Glob", "Edit", "Write", "Bash", *ARMS[arm]]),
           "--disallowedTools", ",".join(DISALLOWED), "--system-prompt", system,
           "--setting-sources", "", "--add-dir", str(d)]
    env = None
    if provider.get("base_url"):
        key_var = provider["api_key_env"]
        if key_var not in os.environ:
            sys.exit(f"provider requires env var {key_var}")
        env = dict(os.environ)
        env.pop("ANTHROPIC_API_KEY", None)
        env["ANTHROPIC_BASE_URL"] = provider["base_url"]
        env["ANTHROPIC_AUTH_TOKEN"] = os.environ[key_var]
    t0 = time.time()
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, cwd=str(d), timeout=TIMEOUT_S, env=env)
        out, rc, err = proc.stdout, proc.returncode, proc.stderr[-2000:]
    except subprocess.TimeoutExpired as e:
        out, rc, err = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or ""), 124, "timeout"
    events = []
    for line in out.splitlines():
        if line.strip().startswith("{"):
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    res = bench_run.parse_events(events, time.time() - t0, rc, err, provider["pricing"])
    res["mcp_connected"] = bench_run.mcp_servers_connected(events)
    return res


def extract_patch(d: Path, gitignore_after_setup: bytes | None = None) -> str:
    # `gr init` appends `.graphrelay/` to .gitignore; that harness side effect must not appear in the
    # graphrelay arm's patch. Restore .gitignore only if the agent left it exactly as setup left it.
    gi = d / ".gitignore"
    if gitignore_after_setup is not None and gi.exists() and gi.read_bytes() == gitignore_after_setup:
        tracked = subprocess.run(["git", "cat-file", "-e", "HEAD:.gitignore"], cwd=d, capture_output=True).returncode == 0
        if tracked:
            sh(["git", "checkout", "-q", "HEAD", "--", ".gitignore"], cwd=d)
        else:
            gi.unlink()
    sh(["git", "add", "-A", "--", ".", ":(exclude).graphrelay"], cwd=d)
    return sh(["git", "diff", "--cached", "--no-color", "HEAD"], cwd=d).stdout


def cmd_run(a):
    providers = bench_run.load_providers()
    provider = providers[a.provider]
    work, out = Path(a.workdir), Path(a.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    tasks = load_tasks(a.task)
    results_f, ledger_f, pred_f = (out / f"{a.arm}-results.jsonl", out / f"{a.arm}-ledger.json",
                                   out / f"{a.arm}-predictions.jsonl")
    done = {json.loads(l)["task_id"] for l in results_f.read_text().splitlines()} if results_f.exists() else set()
    home = Path(a.home) if a.home else Path(os.environ.get("TMPDIR", "/tmp")) / "swebench-gr-home"
    runs, spent = [], 0.0
    if ledger_f.exists():
        runs = json.loads(ledger_f.read_text())["runs"]
        spent = sum(r["cost_usd"] for r in runs)
    for t in tasks:
        iid = t["instance_id"]
        if iid in done:
            print(f"skip {iid} (already recorded; attempts are never replaced)")
            continue
        d = prepare_one(work, a.arm, t)
        if a.dry_run:
            print(f"[dry-run] {a.arm}/{iid}: checkout ok at {d}; no model call")
            continue
        if spent >= a.max_spend:
            sys.exit(f"max-spend ${a.max_spend} reached")
        url, gi_after_setup = None, None
        if a.arm == "graphrelay":
            home.mkdir(parents=True, exist_ok=True)
            url = start_daemon(home, d, DAEMON_PORT)
            gi_after_setup = (d / ".gitignore").read_bytes() if (d / ".gitignore").exists() else b""
        try:
            print(f"run {a.arm}/{iid} ...", flush=True)
            r = run_agent(a.arm, d, t["problem_statement"], provider, url)
        finally:
            if a.arm == "graphrelay":
                stop_daemon(home, d)
        patch = extract_patch(d, gi_after_setup)
        r.pop("events", None)
        row = {"task_id": iid, "arm": a.arm, "model": provider["model"], "patch_chars": len(patch),
               "final_text": r.pop("final_text", ""), **r}
        with results_f.open("a") as f:
            f.write(json.dumps(row) + "\n")
        with pred_f.open("a") as f:
            f.write(json.dumps({"instance_id": iid, "model_name_or_path": f"{provider['model']}-{a.arm}",
                                "model_patch": patch}) + "\n")
        runs.append({"task_id": iid, "cost_usd": row["cost_usd"], "tokens": row["usage"]})
        spent += row["cost_usd"]
        ledger_f.write_text(json.dumps({"arm": a.arm, "provider": a.provider, "model": provider["model"],
                                        "pricing": provider["pricing"], "max_turns": MAX_TURNS,
                                        "runs": runs}, indent=2))
        print(f"  turns={row['num_turns']} tools={row['tool_calls']} cost=${row['cost_usd']} "
              f"patch={len(patch)}ch rc={row['returncode']} mcp={row['mcp_connected']}")


def cmd_grade(a):
    try:
        import swebench  # noqa: F401
    except ImportError:
        sys.exit("official grader missing: pip install swebench (needs Docker; use a linux/amd64 host)")
    out = Path(a.out_dir).resolve()
    for arm in a.arms.split(","):
        pred = out / f"{arm}-predictions.jsonl"
        ids = [json.loads(l)["instance_id"] for l in pred.read_text().splitlines()]
        subprocess.run([sys.executable, "-m", "swebench.harness.run_evaluation", "--dataset_name", DATASET,
                        "--predictions_path", str(pred), "--instance_ids", *ids,
                        "--max_workers", "1", "--run_id", f"{out.name}-{arm}"], cwd=out, check=True)
    print("official reports are written next to the predictions (*.json named <model>-<arm>.<run_id>.json)")


def load_resolved(out: Path, arm: str) -> dict[str, bool]:
    res = {}
    for p in out.glob(f"*-{arm}.{out.name}-{arm}.json"):
        rep = json.loads(p.read_text())
        for i in rep.get("resolved_ids", []):
            res[i] = True
        for i in rep.get("unresolved_ids", []) + rep.get("error_ids", []):
            res.setdefault(i, False)
    return res


def cmd_account(a):
    out = Path(a.out_dir)
    tasks = [t["instance_id"] for t in load_tasks(None)]
    table = {}
    for arm in ARMS:
        f = out / f"{arm}-results.jsonl"
        rows = {json.loads(l)["task_id"]: json.loads(l) for l in f.read_text().splitlines()} if f.exists() else {}
        table[arm] = (rows, load_resolved(out, arm))
    print(f"{'task':36}" + "".join(f"{arm:>26}" for arm in ARMS))
    tot = {arm: {"cost": 0.0, "solved": 0, "attempted": 0, "tools": 0, "graded": 0} for arm in ARMS}
    for iid in tasks:
        cells = []
        for arm, (rows, res) in table.items():
            r = rows.get(iid)
            if not r:
                cells.append("-")
                continue
            t = tot[arm]
            t["cost"] += r["cost_usd"]; t["attempted"] += 1; t["tools"] += r["tool_calls"]
            g = res.get(iid)
            if g is not None:
                t["graded"] += 1; t["solved"] += int(g)
            cells.append(f"{'PASS' if g else 'FAIL' if g is not None else 'ungraded'} ${r['cost_usd']:.2f} {r['tool_calls']}tc")
        print(f"{iid:36}" + "".join(f"{c:>26}" for c in cells))
    for arm, t in tot.items():
        per_solved = f"${t['cost'] / t['solved']:.2f}" if t["solved"] else "undefined (none solved)"
        print(f"{arm}: attempted {t['attempted']}, graded {t['graded']}, solved {t['solved']}, "
              f"spend ${t['cost']:.2f}, per attempted "
              f"${t['cost'] / max(t['attempted'], 1):.2f}, per solved {per_solved}, tool calls {t['tools']}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("export").set_defaults(fn=cmd_export)
    p = sub.add_parser("prepare"); p.set_defaults(fn=cmd_prepare)
    p.add_argument("--workdir", required=True); p.add_argument("--task"); p.add_argument("--arms", default="baseline,graphrelay")
    p = sub.add_parser("run"); p.set_defaults(fn=cmd_run)
    p.add_argument("--arm", required=True, choices=list(ARMS)); p.add_argument("--provider", default="claude-sonnet-5")
    p.add_argument("--workdir", required=True); p.add_argument("--out-dir", required=True)
    p.add_argument("--task"); p.add_argument("--max-spend", type=float, default=15.0)
    p.add_argument("--home"); p.add_argument("--dry-run", action="store_true")
    p = sub.add_parser("grade"); p.set_defaults(fn=cmd_grade)
    p.add_argument("--out-dir", required=True); p.add_argument("--arms", default="baseline,graphrelay")
    p = sub.add_parser("account"); p.set_defaults(fn=cmd_account); p.add_argument("--out-dir", required=True)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
