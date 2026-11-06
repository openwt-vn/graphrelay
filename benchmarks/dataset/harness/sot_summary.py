#!/usr/bin/env python3
"""Aggregate a run-sot.sh label directory: per-repeat and pooled coverage per arm, per-question means
(min-max), turn-cap / empty-answer counts, tool calls, cost, and a question-level paired bootstrap of
graphrelay - baseline. Writes SUMMARY.md next to the model directories' parent and prints it.

    python3 harness/sot_summary.py results/sot/<label>          # every model directory under it
    python3 harness/sot_summary.py results/sot/<label>/<model>  # one model
"""
import json, random, re, statistics, sys
from pathlib import Path

ARM_ORDER = ["baseline", "graphrelay", "graphify", "codegraph-colby", "graphrelay-jev", "graphrelay-laya",
             "codegraph", "codegraph-jev", "codegraph-laya"]  # legacy arm names in runs before the GraphRelay rename


def load_model(mdir: Path):
    reps = sorted(p for p in mdir.glob("rep*") if (p / "grades.json").exists())
    cov, cost, calls, bad, n_runs, qs = {}, {}, {}, {}, {}, []
    for ri, rep in enumerate(reps, 1):
        for g in json.loads((rep / "grades.json").read_text()):
            arm, q = g["arm"], g["task_id"]
            cov.setdefault(arm, {})[(ri, q)] = 100.0 * g["coverage"]
            if q not in qs:
                qs.append(q)
    arms = [a for a in ARM_ORDER if a in cov] + sorted(set(cov) - set(ARM_ORDER))
    for arm in arms:
        cost[arm], calls[arm], bad[arm], n_runs[arm] = 0.0, [], 0, 0
        for rep in reps:
            f = rep / f"{arm}-results.jsonl"
            if not f.exists():
                continue
            for line in f.read_text().splitlines():
                r = json.loads(line)
                cost[arm] += r.get("cost_usd", 0)
                calls[arm].append(r.get("tool_calls", 0))
                n_runs[arm] += 1
                if r.get("returncode", 0) != 0 or not (r.get("answer") or "").strip():
                    bad[arm] += 1
    return reps, qs, arms, cov, cost, calls, bad, n_runs


def boot_ci(diffs):
    rnd = random.Random(3)
    bs = sorted(statistics.mean(rnd.choice(diffs) for _ in diffs) for _ in range(10000))
    return bs[250], bs[9750]


def summarize(mdir: Path) -> str:
    reps, qs, arms, cov, cost, calls, bad, n_runs = load_model(mdir)
    if not reps:
        return f"## {mdir.name}\n\nno graded repeats found.\n"
    nrep = len(reps)
    rr = range(1, nrep + 1)
    out = [f"## {mdir.name} — {nrep} repeat(s) × {len(qs)} questions", ""]
    out.append("| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |")
    out.append("|---|---|---|---|---|---|")
    for arm in arms:
        rm = [statistics.mean(cov[arm][(r, q)] for q in qs if (r, q) in cov[arm]) for r in rr]
        pooled = statistics.mean(cov[arm].values())
        out.append(f"| {arm} | {', '.join(f'{x:.1f}%' for x in rm)} | **{pooled:.1f}%** | {bad[arm]}/{n_runs[arm]} "
                   f"| {statistics.mean(calls[arm]) if calls[arm] else 0:.1f} | ${cost[arm]:.3f} |")
    out += ["", "| Question | " + " | ".join(f"{a} mean (min-max)" for a in arms) + " |",
            "|---|" + "---|" * len(arms)]
    for q in qs:
        cells = []
        for a in arms:
            v = [cov[a][(r, q)] for r in rr if (r, q) in cov[a]]
            cells.append(f"{statistics.mean(v):.1f}% ({min(v):.0f}-{max(v):.0f})" if v else "-")
        out.append(f"| {q} | " + " | ".join(cells) + " |")
    out.append("")
    if "baseline" in cov:
        for a in arms:
            if a == "baseline":
                continue
            diffs = []
            for q in qs:
                b = [cov["baseline"][(r, q)] for r in rr if (r, q) in cov["baseline"]]
                c = [cov[a][(r, q)] for r in rr if (r, q) in cov[a]]
                if b and c:
                    diffs.append(statistics.mean(c) - statistics.mean(b))
            if diffs:
                lo, hi = boot_ci(diffs)
                out.append(f"- {a} − baseline over {len(diffs)} questions: **{statistics.mean(diffs):+.1f} pts**, "
                           f"95% bootstrap CI [{lo:+.1f}, {hi:+.1f}] (resamples questions, so the interval is wide).")
        out.append("")
    zero = ", ".join(f"{a} {sum(1 for v in cov[a].values() if v == 0)}/{len(cov[a])}" for a in arms)
    out += [f"Zero-coverage answers: {zero}. An answer that hits the turn cap is scored 0% by construction; when "
            "one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn "
            "budget', not 'answers better'.", ""]
    return "\n".join(out)


def main():
    target = Path(sys.argv[1])
    mdirs = [target] if list(target.glob("rep*")) else sorted(p for p in target.iterdir() if p.is_dir())
    text = "# SOT benchmark summary\n\n" + "\n".join(summarize(m) for m in mdirs)
    (target / "SUMMARY.md").write_text(text)
    print(text)


if __name__ == "__main__":
    main()
