#!/usr/bin/env python3
"""Generate <results-dir>/RESULTS.md from <results-dir>/*-results.jsonl +
grades.json.

Each (agent, model) directory (e.g. results/claude-code/claude-opus-4-8/)
gets its own report — this deliberately does NOT merge multiple directories
into one report: build_summary/build_significance/etc. key rows by arm name
alone, and two different models can share an arm name (e.g. `graphrelay`
under both Claude and DeepSeek), so merging would silently conflate them.
Compare two models by opening their RESULTS.md files side by side.

30-question capability-balanced set (Q01-Q30, grouped A-F by tested
capability). Q-numbers are the only ids — each question's origin note
(hand-authored, or which ERPNext PR it derives from) is recorded in
`tasks/erpnext_tasks.yaml`'s `origin_note` field, never in the task id.

Usage:
    python3 harness/report.py --results-dir results/claude-code/claude-opus-4-8
"""

from __future__ import annotations

import argparse
import os
import json
import math
from collections import defaultdict
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
TASKS_FILE = ROOT / "tasks" / "erpnext_tasks.yaml"
# run-sot.sh exports these for non-default repos; the defaults keep the ERPNext output unchanged.
REPO_LABEL = os.environ.get("BENCH_REPO_LABEL", "ERPNext")
REPO_SLUG = os.environ.get("BENCH_REPO_SLUG", "frappe/erpnext")
REPO_PIN = os.environ.get("BENCH_REPO_PIN", "1887825ce5c6fe2fbd5366ab52e4c73de24d23fa")

ARM_ORDER = ["baseline", "graphrelay", "graphify", "codegraph-colby", "graphrelay-jev", "graphrelay-laya",
             "codegraph", "codegraph-jev", "codegraph-laya"]  # legacy arm names in runs before the GraphRelay rename
PASS_THRESHOLD = 0.75


def load_rows(results_dir: Path) -> list[dict]:
    rows = []
    for f in sorted(results_dir.glob("*-results.jsonl")):
        for line in f.read_text().splitlines():
            if line.strip():
                rows.append(json.loads(line))
    return rows


def load_tasks(tasks_file: Path = TASKS_FILE) -> dict[str, dict]:
    tasks = yaml.safe_load(tasks_file.read_text())["tasks"]
    return {t["id"]: t for t in tasks}


def pct(v: float) -> str:
    return "—" if math.isnan(v) else f"{v:.1%}"


def mcnemar(pass_a: dict[str, bool], pass_b: dict[str, bool]) -> str:
    shared = sorted(set(pass_a) & set(pass_b))
    if not shared:
        return "no shared tasks"
    a_only = sum(1 for t in shared if pass_a[t] and not pass_b[t])
    b_only = sum(1 for t in shared if not pass_a[t] and pass_b[t])
    n = a_only + b_only
    if n == 0:
        return f"n={len(shared)}, no discordant pairs (identical pass/fail)"
    try:
        from scipy.stats import chi2
        stat = (abs(a_only - b_only) - 1) ** 2 / n
        p = 1 - chi2.cdf(stat, df=1)
        sig = "significant" if p < 0.05 else "not significant"
        return (f"discordant pairs: {a_only} vs {b_only} (n={n}), "
               f"chi2={stat:.3f}, p={p:.3f} ({sig} at alpha=0.05)")
    except ImportError:
        return (f"discordant pairs: {a_only} vs {b_only} (n={n}) — "
               f"scipy not installed, no p-value.")


def build_executive_summary(arms: list[str], by_arm: dict, cov: dict,
                            task_ids: list[str]) -> list[str]:
    stats = {}
    for arm in arms:
        rs = by_arm[arm]
        covs = [cov.get((tid, arm)) for tid in task_ids]
        covs = [c for c in covs if c is not None]
        n = len(covs)
        mean_cov = sum(covs) / n if n else float("nan")
        variance = sum((c - mean_cov) ** 2 for c in covs) / n if n else 0.0
        stdev = variance ** 0.5
        avg_tok = sum(r["total_tokens"] for r in rs) / len(rs) if rs else 0
        cost = sum(r["cost_usd"] for r in rs)
        below_50 = sum(1 for c in covs if c < 0.5)
        stats[arm] = {"mean": mean_cov, "stdev": stdev,
                      "min": min(covs) if covs else 0,
                      "max": max(covs) if covs else 0, "below_50": below_50,
                      "avg_tok": avg_tok, "cost": cost}

    wins = {a: 0 for a in arms}
    ties = 0
    for tid in task_ids:
        scores = {a: (cov.get((tid, a)) or 0) for a in arms}
        best = max(scores.values())
        winners = [a for a, s in scores.items() if s == best]
        if len(winners) == 1:
            wins[winners[0]] += 1
        else:
            ties += 1

    best_coverage = max(arms, key=lambda a: stats[a]["mean"])
    cheapest = min(arms, key=lambda a: stats[a]["cost"])
    most_consistent = min(arms, key=lambda a: stats[a]["stdev"])

    lines = [
        "## Executive summary",
        "",
        f"Head-to-head win count across {len(task_ids)} questions (highest "
        f"coverage on that question; ties when 2+ arms score equally): " +
        ", ".join(f"**{a}** {wins[a]}" for a in arms) + f", {ties} ties.",
        "",
        "| Arm | Mean coverage | Notes |",
        "|---|---|---|",
    ]
    for arm in arms:
        s = stats[arm]
        notes = []
        if arm == best_coverage:
            notes.append("highest mean coverage")
        if arm == cheapest:
            notes.append(f"cheapest (${s['cost']:.2f} total)")
        if arm == most_consistent:
            notes.append(f"most consistent (stdev {s['stdev']:.3f})")
        if s["below_50"] > 0:
            notes.append(f"{s['below_50']} question(s) below 50% coverage")
        lines.append(f"| {arm} | {pct(s['mean'])} | " +
                     "; ".join(notes) + " |")
    lines += [
        "",
        "**Caveat:** drawn from this specific 30-question run; see the "
        "significance section below for whether the coverage gaps are "
        "statistically distinguishable from noise.",
        "",
    ]
    return lines


def build_summary(arms: list[str], by_arm: dict, cov: dict) -> list[str]:
    lines = [
        "## Summary",
        "",
        "| Arm | Agent model | n | Mean coverage | Median coverage | Avg tokens/query | "
        "Avg tool calls | Avg turns | Total cost (USD) |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for arm in arms:
        rs = by_arm[arm]
        n = len(rs)
        models = sorted({r.get("model", "claude-opus-4-8") for r in rs})
        model_str = "/".join(models)
        covs = sorted(c for c in (cov.get((r["task_id"], arm)) for r in rs)
                     if c is not None)
        mean_cov = sum(covs) / len(covs) if covs else float("nan")
        median_cov = float("nan")
        if covs:
            mid = len(covs) // 2
            median_cov = (covs[mid] if len(covs) % 2
                         else (covs[mid - 1] + covs[mid]) / 2)
        avg_tok = sum(r["total_tokens"] for r in rs) / n
        avg_calls = sum(r["tool_calls"] for r in rs) / n
        avg_turns = sum(r["num_turns"] for r in rs) / n
        cost = sum(r["cost_usd"] for r in rs)
        lines.append(
            f"| {arm} | {model_str} | {n} | {pct(mean_cov)} | {pct(median_cov)} | "
            f"{avg_tok:,.0f} | {avg_calls:.1f} | {avg_turns:.1f} | "
            f"${cost:.2f} |")
    return lines


def build_group_breakdown(arms: list[str], tasks: dict, cov: dict,
                          task_ids: list[str]) -> list[str]:
    by_group: dict[str, list[str]] = defaultdict(list)
    group_title: dict[str, str] = {}
    for tid in task_ids:
        g = tasks[tid]["group"]
        by_group[g].append(tid)
        group_title[g] = tasks[tid]["group_title"]

    lines = [
        "## Per-group coverage",
        "",
        "| Group | Questions | " + " | ".join(arms) + " |",
        "|---|---|" + "---|" * len(arms),
    ]
    for g in sorted(by_group):
        tids = by_group[g]
        cells = []
        for arm in arms:
            covs = [cov.get((tid, arm)) for tid in tids]
            covs = [c for c in covs if c is not None]
            cells.append(pct(sum(covs) / len(covs)) if covs else "—")
        lines.append(f"| {g} — {group_title[g]} | " +
                     ", ".join(tids) + " | " + " | ".join(cells) + " |")
    return lines


def build_per_question_table(arms: list[str], tasks: dict, grades_by_key: dict,
                             task_ids: list[str]) -> list[str]:
    lines = [
        "## Per-question coverage (with covered/partial/missing fact counts)",
        "",
        "| Q# | Group | n facts | " + " | ".join(arms) + " |",
        "|---|---|---|" + "---|" * len(arms),
    ]
    for tid in task_ids:
        t = tasks[tid]
        n_facts = len(t["key_facts"])
        cells = []
        for arm in arms:
            g = grades_by_key.get((tid, arm))
            if g is None:
                cells.append("—")
                continue
            covered = sum(1 for v in g["verdicts"] if v["verdict"] == "covered")
            partial = sum(1 for v in g["verdicts"] if v["verdict"] == "partial")
            missing = sum(1 for v in g["verdicts"] if v["verdict"] == "missing")
            cells.append(f"{pct(g['coverage'])} ({covered}c/{partial}p/{missing}m)")
        lines.append(f"| {tid} | {t['group']} | {n_facts} | " +
                     " | ".join(cells) + " |")
    lines.append("")
    lines.append("c = covered, p = partial (weight 0.5), m = missing")
    return lines


def build_fact_detail(arms: list[str], tasks: dict, grades_by_key: dict,
                      task_ids: list[str]) -> list[str]:
    lines = ["## Per-fact verdicts", ""]
    for tid in task_ids:
        t = tasks[tid]
        lines.append(f"### {tid} ({t['group']} — {t['group_title']})")
        lines.append("")
        lines.append(f"> {t['question'].strip()}")
        lines.append("")
        for i, fact in enumerate(t["key_facts"]):
            lines.append(f"**[{i}]** {fact.strip()}")
            lines.append("")
            for arm in arms:
                g = grades_by_key.get((tid, arm))
                if g is None:
                    continue
                v = next((v for v in g["verdicts"] if v["fact_index"] == i),
                        None)
                if v is None:
                    continue
                mark = {"covered": "✅", "partial": "◐",
                       "missing": "❌"}[v["verdict"]]
                lines.append(f"- {mark} **{arm}** ({v['verdict']}): "
                            f"{v['reasoning']}")
            lines.append("")
    return lines


def build_significance(arms: list[str], cov: dict,
                       task_ids: list[str]) -> list[str]:
    lines = [
        "## Statistical significance (McNemar's test)",
        "",
        f"Binary pass = coverage >= {PASS_THRESHOLD:.0%}.",
        "",
    ]
    pass_by_arm = {
        arm: {tid: (cov.get((tid, arm)) or 0) >= PASS_THRESHOLD
             for tid in task_ids}
        for arm in arms
    }
    for i, a in enumerate(arms):
        for b in arms[i + 1:]:
            lines.append(f"**{a} vs {b}:** {mcnemar(pass_by_arm[a], pass_by_arm[b])}")
            lines.append("")
    n = len(task_ids)
    lines.append(
        f"With n={n} questions this test has real (if still modest) power. "
        "A 'not significant' result means the observed coverage gap "
        "could plausibly be noise, not that the arms are equivalent.")
    return lines


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", required=True,
                    help="directory of <arm>-results.jsonl to report on, "
                         "e.g. results/claude-code/claude-opus-4-8 — "
                         "required (no default), one report per (agent, "
                         "model) directory, never merged")
    ap.add_argument("--grades-file", default=None,
                    help="default: <results-dir>/grades.json")
    ap.add_argument("--output", default=None,
                    help="default: <results-dir>/RESULTS.md")
    ap.add_argument("--tasks-file", default=str(TASKS_FILE),
                    help="alternate question set matching --results-dir's "
                         "question set (default: tasks/erpnext_tasks.yaml)")
    args = ap.parse_args()

    results_dir = Path(args.results_dir)
    grades_file = Path(args.grades_file) if args.grades_file else results_dir / "grades.json"
    output_file = Path(args.output) if args.output else results_dir / "RESULTS.md"

    rows = load_rows(results_dir)
    tasks = load_tasks(Path(args.tasks_file))
    grades = json.loads(grades_file.read_text())
    cov = {(g["task_id"], g["arm"]): g["coverage"] for g in grades}
    grades_by_key = {(g["task_id"], g["arm"]): g for g in grades}

    by_arm: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        by_arm[r["arm"]].append(r)
    arms = [a for a in ARM_ORDER if a in by_arm] + sorted(
        set(by_arm) - set(ARM_ORDER))
    task_ids = sorted({r["task_id"] for r in rows})
    n_tasks = len(task_ids)

    models_by_arm = {a: sorted({r.get("model", "claude-opus-4-8")
                                for r in by_arm[a]}) for a in arms}
    all_models = sorted({m for ms in models_by_arm.values() for m in ms})
    if len(all_models) <= 1:
        model_clause = f"fixed agent {all_models[0] if all_models else 'claude-opus-4-8'}"
    else:
        model_clause = ("agent model varies by arm (see Summary table) — "
                        + ", ".join(f"{a}: {'/'.join(models_by_arm[a])}"
                                    for a in arms))

    # `max_turns_cap` is only present on rows written after it was added to
    # run.py's schema — every row from before that point ran under the
    # hardcoded MAX_TURNS=14 that was in effect at the time, so a missing
    # field means 14, not "unknown" (treating it as unknown would let it get
    # silently discarded below, and a handful of new 25-cap rows mixed into
    # mostly-legacy-14 rows would then misreport as "max 25 turns" for the
    # whole run). Flag a genuine mix explicitly rather than reporting a
    # single number that isn't true for every task (e.g. a subset of tasks
    # retried under a raised cap must not get reported as if the raised cap
    # applied uniformly to the whole run).
    turn_caps = {r.get("max_turns_cap", 14) for r in rows}
    if len(turn_caps) == 1:
        turns_clause = f"max {turn_caps.pop()} turns"
    else:
        turns_clause = (
            f"turn cap varies by task ({'/'.join(str(c) for c in sorted(turn_caps))} "
            "— a subset of tasks were retried under a raised cap; see "
            "per-task num_turns/max_turns_cap in the results jsonl)")

    lines = [
        f"# Benchmark Results: code-intelligence tools on {REPO_LABEL} "
        f"(Q01-Q{n_tasks:02d})",
        "",
        f"> Generated by `harness/report.py`. {n_tasks}-question set "
        f"from `tasks/{Path(args.tasks_file).name}` — see that file for "
        "each question's origin note. "
        f"Methodology: {model_clause}, {turns_clause}, "
        "grep/read/list floor + one code-intelligence MCP server per "
        "arm (baseline arm: no MCP server, tool-floor only); key-fact "
        "coverage = (covered + 0.5*partial)/total, judged blind by "
        "claude-sonnet-5 with verbatim-quote audit (markdown-insensitive). "
        f"Repo: {REPO_SLUG} @ {REPO_PIN}.",
        "",
    ]
    lines += build_executive_summary(arms, by_arm, cov, task_ids)
    lines += build_summary(arms, by_arm, cov)
    lines.append("")
    lines += build_group_breakdown(arms, tasks, cov, task_ids)
    lines.append("")
    lines += build_per_question_table(arms, tasks, grades_by_key, task_ids)
    lines.append("")
    lines += build_significance(arms, cov, task_ids)
    lines.append("")
    lines += build_fact_detail(arms, tasks, grades_by_key, task_ids)

    lines += [
        "## Reproducing",
        "",
        "```bash",
        f"python3 harness/report.py --results-dir {results_dir}",
        "```",
        "",
    ]

    output_file.write_text("\n".join(lines))
    print(f"Wrote {output_file} ({n_tasks} questions, {len(arms)} arms)")


if __name__ == "__main__":
    main()
