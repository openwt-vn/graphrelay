#!/usr/bin/env python3
"""Generate <results-dir>/tool-flow-report.html from <results-dir>/*-results.jsonl
+ grades.json + transcripts/*.json.

A visual companion to report.py's RESULTS.md: for every question, a
collapsible card shows each arm's actual ordered tool-call sequence (tool
name, input, a truncated preview of what came back), token/cost breakdown,
and final answer — the level of detail RESULTS.md's tables don't carry.

Same one-report-per-(agent,model)-directory rule as report.py: never point
this at more than one results-dir per run, and never merge two directories'
output into one report (two different models can share an arm name, e.g.
`graphrelay` under both Claude and DeepSeek, and merging would conflate them).

This is a from-scratch reimplementation, not a refactor of a prior version —
an earlier ad-hoc report generator existed only as a scratch script (never
committed) and was lost; this was rebuilt by reverse-engineering its output
HTML directly (results/hard/claude-code/claude-sonnet-5-fixed/tool-flow-report.html)
so the visual structure/CSS matches byte-for-byte, generalized here to work
against any results-dir's arms rather than one hardcoded arm list.

Usage:
    python3 harness/report_html.py --results-dir results/hard/claude-code/claude-opus-4-8
"""

from __future__ import annotations

import argparse
import html
import json
from collections import defaultdict
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
TASKS_FILE = ROOT / "tasks" / "erpnext_tasks.yaml"
INDEXING_SPEED_FILE = ROOT / "results" / "indexing-speed" / "results.json"

# Preferred display order; any other arm found in a results dir (e.g. an old competitor arm
# in a historical run) still renders, sorted alphabetically after these.
ARM_ORDER = ["baseline", "graphrelay", "graphify", "codegraph-colby", "graphrelay-jev", "graphrelay-laya"]
ARM_LABELS = {
    "baseline": "Baseline (no tools)",
    "graphrelay": "GraphRelay",
}

QTRUNC_TOC = 110
QTRUNC_SUMMARY = 130
RESULT_TRUNC = 300


def cov_class(pct: float) -> str:
    if pct >= 75:
        return "cov-hi"
    if pct >= 50:
        return "cov-mid"
    return "cov-lo"


def esc(s) -> str:
    return html.escape(str(s), quote=True)


def trunc(s: str, n: int) -> str:
    s = s or ""
    return s if len(s) <= n else s[:n] + "…"


def fmt_num(n) -> str:
    return f"{n:,}"


def load_tasks(tasks_file: Path) -> list[dict]:
    return yaml.safe_load(tasks_file.read_text())["tasks"]


def load_jsonl(path: Path) -> dict[str, dict]:
    rows = {}
    if not path.exists():
        return rows
    for line in path.read_text().splitlines():
        if line.strip():
            d = json.loads(line)
            rows[d["task_id"]] = d
    return rows


def load_grades(grades_file: Path) -> dict[tuple[str, str], float]:
    grades = {}
    for g in json.loads(grades_file.read_text()):
        grades[(g["arm"], g["task_id"])] = g["coverage"] * 100.0
    return grades


def load_indexing_speed() -> dict[str, dict]:
    """Per-arm cold-start indexing time from harness/bench_indexing.py's
    results.json, keyed by arm name — a genuinely different benchmark
    (build speed, not answer quality) shown alongside these results since
    both matter for choosing a tool. Missing file / missing arm is not an
    error: not every results-dir's arm set has a matching indexing-speed
    run (e.g. a one-off provider smoke test), so this degrades to omitting
    the indexing-time line for that arm rather than failing the report."""
    if not INDEXING_SPEED_FILE.exists():
        return {}
    return {r["arm"]: r for r in json.loads(INDEXING_SPEED_FILE.read_text())}


def load_transcript_steps(transcripts_dir: Path, arm: str, tid: str):
    """Return list of (tool_name, input_dict, result_text, is_error)."""
    tpath = transcripts_dir / f"{arm}-{tid}.json"
    if not tpath.exists():
        return []
    events = json.loads(tpath.read_text())
    tool_calls = {}  # id -> (name, input)
    order = []
    results = {}  # id -> (text, is_error)
    for e in events:
        if e.get("type") == "assistant":
            for c in e.get("message", {}).get("content", []):
                if c.get("type") == "tool_use":
                    tool_calls[c["id"]] = (c.get("name", "?"), c.get("input", {}))
                    order.append(c["id"])
        elif e.get("type") == "user":
            content = e.get("message", {}).get("content", [])
            if isinstance(content, list):
                for c in content:
                    if c.get("type") == "tool_result":
                        tid_ = c.get("tool_use_id")
                        rc = c.get("content")
                        if isinstance(rc, list):
                            text = "".join(
                                part.get("text", "") for part in rc if isinstance(part, dict)
                            )
                        else:
                            text = str(rc) if rc is not None else ""
                        results[tid_] = (text, bool(c.get("is_error")))

    steps = []
    for call_id in order:
        name, inp = tool_calls[call_id]
        text, is_error = results.get(call_id, ("", False))
        steps.append((name, inp, text, is_error))
    return steps


def display_tool_name(name: str, arm: str) -> str:
    prefix = f"mcp__{arm}__"
    if name.startswith(prefix):
        return f"{arm}:{name[len(prefix):]}"
    if name.startswith("mcp__"):
        rest = name[len("mcp__"):]
        if "__" in rest:
            server, tool = rest.split("__", 1)
            return f"{server}:{tool}"
    return name


def tool_summary_line(steps, arm: str) -> str:
    counts: dict[str, int] = {}
    for name, _inp, _res, _err in steps:
        disp = display_tool_name(name, arm)
        counts[disp] = counts.get(disp, 0) + 1
    ordered = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))
    return ", ".join(f"{name}×{n}" for name, n in ordered)


def fmt_indexing_time(seconds: float) -> str:
    if seconds < 60:
        return f"{seconds:.1f}s"
    m, s = divmod(seconds, 60)
    return f"{int(m)}m{s:04.1f}s"


def build_stats_row(tasks, arms, grades, results_by_arm, indexing_speed) -> str:
    cards = []
    for arm in arms:
        label = ARM_LABELS.get(arm, arm.replace("-", " ").title())
        results = results_by_arm[arm]
        covs = [grades.get((arm, t["id"])) for t in tasks]
        covs = [c for c in covs if c is not None]
        n = len(covs)
        avg = sum(covs) / n if n else 0.0
        below50 = sum(1 for c in covs if c < 50)
        total_cost = sum(results.get(t["id"], {}).get("cost_usd", 0.0) for t in tasks)
        agg = {"input": 0, "output": 0, "cache_write": 0, "cache_read": 0}
        for t in tasks:
            r = results.get(t["id"])
            if not r:
                continue
            u = r.get("usage", {})
            for k in agg:
                agg[k] += u.get(k, 0)
        total = sum(agg.values())
        idx = indexing_speed.get(arm)
        idx_html = (
            f"<div class='sub' style='margin-top:4px'>"
            f"cold-start indexing: <b>{fmt_indexing_time(idx['elapsed_s'])}</b>"
            f"{' ⚠' if not idx.get('ok', True) else ''}</div>"
            if idx else ""
        )
        card = (
            f"<div class='stat-card'><h4>{esc(label)}</h4>"
            f"<div class='big {cov_class(avg)}'>{avg:.1f}%</div>"
            f"<div class='sub'>avg coverage across {n}/{len(tasks)} questions "
            f"· ${total_cost:.2f} total · {below50} below 50%</div>"
            f"{idx_html}"
            f"<table class='token-table' style='margin-top:8px'>"
            f"<tr><th>input</th><th>output</th><th>cache create</th><th>cache read</th><th>total</th></tr>"
            f"<tr><td>{fmt_num(agg['input'])}</td><td>{fmt_num(agg['output'])}</td>"
            f"<td>{fmt_num(agg['cache_write'])}</td><td>{fmt_num(agg['cache_read'])}</td>"
            f"<td>{fmt_num(total)}</td></tr></table></div>"
        )
        cards.append(card)
    return "<div class='stats-row'>" + "".join(cards) + "</div>"


def build_toc(tasks, arms, grades, results_by_arm) -> str:
    header_cells = "".join(
        f"<th>{esc(ARM_LABELS.get(arm, arm.replace('-', ' ').title()))}"
        f"<br><span style='font-weight:400;color:var(--muted)'>cov / cost / turns</span></th>"
        for arm in arms
    )
    rows = []
    cur_group = None
    for t in tasks:
        if t["group"] != cur_group:
            cur_group = t["group"]
            rows.append(
                f"<tr class='group-hdr'><td colspan='{2 + len(arms)}'>"
                f"Group {esc(t['group'])} — {esc(t.get('group_title', ''))}</td></tr>"
            )
        cells = []
        for arm in arms:
            r = results_by_arm[arm].get(t["id"])
            cov = grades.get((arm, t["id"]))
            if r is None or cov is None:
                cells.append("<td><span class='pill cov-na'>n/a</span></td>")
                continue
            cost = r.get("cost_usd", 0.0)
            turns = r.get("num_turns", 0)
            cells.append(
                f"<td><span class='pill {cov_class(cov)}'>{cov:.1f}%</span> "
                f"<span style='color:var(--muted)'>${cost:.2f} / {turns}t</span></td>"
            )
        rows.append(
            f"<tr><td><a href='#{t['id']}'>{t['id']}</a></td>"
            f"<td>{esc(trunc(t['question'].strip(), QTRUNC_TOC))}</td>"
            + "".join(cells)
            + "</tr>"
        )
    return (
        "<table class='toc'><thead><tr><th>Q#</th><th>Question</th>"
        + header_cells
        + "</tr></thead><tbody>"
        + "".join(rows)
        + "</tbody></table>"
        + "<div class='legend'>Coverage = judged key-fact coverage "
        "(covered + 0.5×partial)/total, blind claude-sonnet-5 judge. "
        "Click a question below to expand its full tool-call flow per arm.</div>"
    )


def build_arm_col(arm, task, grades, result, transcripts_dir) -> str:
    label = ARM_LABELS.get(arm, arm.replace("-", " ").title())
    tid = task["id"]
    cov = grades.get((arm, tid))
    cov_html = (
        f"<span class='pill {cov_class(cov)}'>{cov:.1f}%</span>"
        if cov is not None else "<span class='pill cov-na'>n/a</span>"
    )
    if result is None:
        return (
            f"<div class='arm-col'><h3>{esc(label)} {cov_html}</h3>"
            f"<div style='color:var(--muted);font-size:12px'>no run recorded for this question</div></div>"
        )

    turns = result.get("num_turns", 0)
    tool_calls_n = result.get("tool_calls", 0)
    cost = result.get("cost_usd", 0.0)
    elapsed = result.get("elapsed_s", 0.0)
    u = result.get("usage", {})
    total_tok = result.get("total_tokens", sum(u.values()))
    steps = load_transcript_steps(transcripts_dir, arm, tid)
    summary_line = tool_summary_line(steps, arm)

    step_html = []
    for i, (name, inp, res_text, is_error) in enumerate(steps, start=1):
        disp = display_tool_name(name, arm)
        li_class = "err" if is_error or "error" in res_text.lower()[:40] else ""
        step_html.append(
            f"<li class='{li_class}'><span class='step-n'>#{i}</span>"
            f"<span class='step-tool'>{esc(disp)}</span>"
            f"<div class='step-input'>{esc(json.dumps(inp))}</div>"
            f"<div class='step-result'>→ {esc(trunc(res_text.strip(), RESULT_TRUNC))}</div></li>"
        )

    answer = (result.get("answer") or "").strip()

    return (
        f"<div class='arm-col'><h3>{esc(label)} {cov_html}</h3>"
        f"<div class='stat-grid'>"
        f"<div>turns: <b>{turns}</b></div><div>tool calls: <b>{tool_calls_n}</b></div>"
        f"<div>cost: <b>${cost:.4f}</b></div><div>elapsed: <b>{elapsed:.1f}s</b></div>"
        f"</div>"
        f"<table class='token-table'><tr><th>input</th><th>output</th><th>cache create</th>"
        f"<th>cache read</th><th>total</th></tr>"
        f"<tr><td>{fmt_num(u.get('input',0))}</td><td>{fmt_num(u.get('output',0))}</td>"
        f"<td>{fmt_num(u.get('cache_write',0))}</td><td>{fmt_num(u.get('cache_read',0))}</td>"
        f"<td>{fmt_num(total_tok)}</td></tr></table>"
        f"<div style='font-size:11.5px;color:var(--muted);margin-bottom:8px'>{esc(summary_line) if summary_line else '(no tool calls)'}</div>"
        f"<ol class='flow'>{''.join(step_html)}</ol>"
        f"<details class='answer-toggle'><summary>final answer</summary>"
        f"<div class='answer-text'>{esc(answer) if answer else '(empty — did not converge to a final answer)'}</div></details>"
        f"</div>"
    )


def build_questions(tasks, arms, grades, results_by_arm, transcripts_dir) -> str:
    blocks = []
    for t in tasks:
        tid = t["id"]
        arm_cols = "".join(
            build_arm_col(arm, t, grades, results_by_arm[arm].get(tid), transcripts_dir)
            for arm in arms
        )
        grid_cols = f"repeat({min(len(arms), 5)}, 1fr)"
        blocks.append(
            f"<details class='q' id='{tid}'><summary><span class='qid'>{tid}</span>"
            f"<span class='grp'>Group {esc(t['group'])} — {esc(t.get('group_title', ''))}</span>"
            f"<span class='question-text'>{esc(trunc(t['question'].strip(), QTRUNC_SUMMARY))}</span></summary>"
            f"<div class='q-body'><div class='question-full'>{esc(t['question'].strip())}</div>"
            f"<div class='arms-grid' style='grid-template-columns:{grid_cols}'>{arm_cols}</div></div></details>"
        )
    return "".join(blocks)


STYLE = """
:root {
  --bg: #f7f7f5; --panel: #ffffff; --border: #e2e0da; --text: #24231f;
  --muted: #6b6a63; --accent: #b5651d; --mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  --hi: #1f7a3f; --hi-bg: #e6f4ea; --mid: #a06a00; --mid-bg: #fbf0d9; --lo: #b3261e; --lo-bg: #fbe9e7;
}
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--text); font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; font-size:14px; line-height:1.5; }
header.page { padding: 28px 32px 16px; border-bottom: 1px solid var(--border); background: var(--panel); }
header.page h1 { margin: 0 0 6px; font-size: 22px; }
header.page p { margin: 4px 0; color: var(--muted); font-size: 13px; max-width: 900px; }
.container { padding: 20px 32px 60px; max-width: 1800px; margin: 0 auto; }
table.toc { border-collapse: collapse; width: 100%; margin: 18px 0 30px; background: var(--panel); border:1px solid var(--border); border-radius:8px; overflow:hidden;}
table.toc th, table.toc td { padding: 6px 10px; border-bottom: 1px solid var(--border); text-align:left; font-size:13px; }
table.toc th { background:#efece4; position:sticky; top:0; }
table.toc tr:last-child td { border-bottom:none; }
table.toc a { color: var(--text); text-decoration:none; font-weight:600; }
table.toc a:hover { text-decoration:underline; }
.pill { display:inline-block; padding:1px 8px; border-radius:10px; font-weight:600; font-size:12px; }
.cov-hi { color: var(--hi); background: var(--hi-bg); }
.cov-mid { color: var(--mid); background: var(--mid-bg); }
.cov-lo { color: var(--lo); background: var(--lo-bg); }
.cov-na { color: var(--muted); background: #eee; }
.group-hdr td { background:#efece4 !important; font-weight:700; padding-top:10px; }
details.q { background: var(--panel); border:1px solid var(--border); border-radius: 10px; margin-bottom: 14px; scroll-margin-top: 12px; }
details.q > summary { cursor:pointer; padding: 12px 16px; font-weight:600; list-style:none; display:flex; align-items:center; gap:10px; flex-wrap:wrap;}
details.q > summary::-webkit-details-marker { display:none; }
details.q > summary .qid { font-family: var(--mono); color: var(--accent); }
details.q > summary .grp { color: var(--muted); font-weight:500; font-size:12px; }
details.q > summary .question-text { font-weight:400; color:var(--text); flex:1 1 300px; font-size:13px; }
.q-body { padding: 0 16px 16px; border-top:1px solid var(--border); }
.question-full { color: var(--muted); font-size: 13px; margin: 12px 0 16px; white-space: pre-wrap; }
.arms-grid { display:grid; gap: 14px; align-items:start; }
@media (max-width: 1700px) { .arms-grid { grid-template-columns: repeat(3, 1fr) !important; } }
@media (max-width: 1000px) { .arms-grid { grid-template-columns: repeat(2, 1fr) !important; } }
@media (max-width: 600px) { .arms-grid { grid-template-columns: 1fr !important; } }
.arm-col { border:1px solid var(--border); border-radius:8px; padding:10px 12px; background:#fcfcfa; }
.arm-col h3 { margin:0 0 6px; font-size:14px; display:flex; justify-content:space-between; align-items:center; }
.stat-grid { display:grid; grid-template-columns: repeat(2, 1fr); gap:4px 10px; font-size:12px; color:var(--muted); margin-bottom:10px; }
.stat-grid b { color: var(--text); }
.token-table { width:100%; border-collapse:collapse; font-size:11.5px; margin-bottom:10px; }
.token-table td, .token-table th { padding:3px 6px; border:1px solid var(--border); text-align:right; }
.token-table th { background:#efece4; text-align:right; }
.token-table td:first-child, .token-table th:first-child { text-align:left; }
ol.flow { list-style:none; margin:0; padding:0; display:flex; flex-direction:column; gap:6px; max-height: 480px; overflow-y:auto; }
ol.flow li { border:1px solid var(--border); border-radius:6px; padding:6px 8px; background:#fff; }
ol.flow li.err { border-color: var(--lo); background: var(--lo-bg); }
.step-n { font-family: var(--mono); color: var(--muted); font-size:11px; margin-right:6px; }
.step-tool { font-family: var(--mono); font-weight:700; font-size:12px; color:#1a4a7a; }
.step-input { font-family: var(--mono); font-size:11px; color:var(--text); word-break:break-word; margin-top:3px; }
.step-result { font-size:11px; color:var(--muted); margin-top:3px; white-space:pre-wrap; word-break:break-word; }
.answer-toggle { margin-top:10px; }
.answer-toggle summary { cursor:pointer; font-size:12px; color:var(--accent); font-weight:600; }
.answer-text { white-space:pre-wrap; font-size:12px; margin-top:6px; max-height:260px; overflow-y:auto; border:1px solid var(--border); padding:8px; border-radius:6px; background:#fff; }
.stats-row { display:grid; grid-template-columns: repeat(5, 1fr); gap: 14px; margin: 18px 0 8px; }
@media (max-width: 1400px) { .stats-row { grid-template-columns: repeat(3, 1fr); } }
@media (max-width: 700px) { .stats-row { grid-template-columns: repeat(2, 1fr); } }
.stat-card { background:var(--panel); border:1px solid var(--border); border-radius:10px; padding:14px 16px; }
.stat-card h4 { margin:0 0 8px; font-size:13px; color:var(--muted); font-weight:600; }
.stat-card .big { font-size:28px; font-weight:700; line-height:1; }
.stat-card .big.hi { color: var(--hi); }
.stat-card .big.mid { color: var(--mid); }
.stat-card .big.lo { color: var(--lo); }
.stat-card .sub { font-size:11.5px; color:var(--muted); margin-top:6px; }
.legend { font-size:12px; color:var(--muted); margin: 6px 0 18px; }
a.top { position:fixed; right:20px; bottom:20px; background:var(--accent); color:#fff; padding:8px 12px; border-radius:20px; text-decoration:none; font-size:12px; font-weight:600; box-shadow:0 2px 6px rgba(0,0,0,.2); }
"""


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", required=True,
                    help="directory of <arm>-results.jsonl + grades.json + "
                         "transcripts/ to report on, e.g. "
                         "results/hard/claude-code/claude-opus-4-8 — one "
                         "report per (agent, model) directory, never merged")
    ap.add_argument("--grades-file", default=None,
                    help="default: <results-dir>/grades.json (must already "
                         "exist — run grade.py first)")
    ap.add_argument("--output", default=None,
                    help="default: <results-dir>/tool-flow-report.html")
    ap.add_argument("--tasks-file", default=str(TASKS_FILE),
                    help="alternate question set matching --results-dir's "
                         "question set (default: tasks/erpnext_tasks.yaml)")
    args = ap.parse_args()

    results_dir = Path(args.results_dir)
    grades_file = Path(args.grades_file) if args.grades_file else results_dir / "grades.json"
    output_file = Path(args.output) if args.output else results_dir / "tool-flow-report.html"
    transcripts_dir = results_dir / "transcripts"

    tasks = load_tasks(Path(args.tasks_file))
    grades = load_grades(grades_file)
    indexing_speed = load_indexing_speed()

    by_arm_files = sorted(results_dir.glob("*-results.jsonl"))
    results_by_arm = {
        f.stem[: -len("-results")]: load_jsonl(f) for f in by_arm_files
    }
    # baseline only ever covers Q01 in most alternate-provider smoke tests
    # (n=1) rather than the full question set — including it would render
    # 29 all-"n/a" columns, all noise. Drop any arm whose run count is under
    # half the task set; every real comparison arm covers the whole set.
    results_by_arm = {
        a: rows for a, rows in results_by_arm.items() if len(rows) >= len(tasks) / 2
    }
    arms = [a for a in ARM_ORDER if a in results_by_arm] + sorted(
        set(results_by_arm) - set(ARM_ORDER))

    all_models = sorted({
        r.get("model", "?")
        for rows in results_by_arm.values()
        for r in rows.values()
    })
    model_clause = all_models[0] if len(all_models) == 1 else "/".join(all_models)
    arm_clause = " vs ".join(ARM_LABELS.get(a, a) for a in arms)

    header = (
        "<header class='page'><h1>ERPNext code-intelligence benchmark — tool-call flows &amp; token usage</h1>"
        f"<p>{esc(arm_clause)}, {len(tasks)} questions, {esc(model_clause)} agent. "
        "For each question: the ordered sequence of tool calls each arm "
        "actually made, its input arguments, a truncated preview of what "
        "came back, and the token/cost breakdown (input / output / "
        f"cache-creation / cache-read). Source: <code>{esc(str(results_dir))}/</code>.</p></header>"
    )

    body = (
        "<div class='container'>"
        + build_stats_row(tasks, arms, grades, results_by_arm, indexing_speed)
        + build_toc(tasks, arms, grades, results_by_arm)
        + build_questions(tasks, arms, grades, results_by_arm, transcripts_dir)
        + "<a class='top' href='#top'>↑ top</a></div>"
    )

    html_doc = (
        "<!doctype html><html><head><meta charset='utf-8'>"
        f"<title>ERPNext benchmark — tool-flow report ({esc(model_clause)})</title>"
        f"<style>{STYLE}</style></head><body>{header}{body}</body></html>"
    )

    output_file.write_text(html_doc)
    print(f"Wrote {output_file} ({output_file.stat().st_size:,} bytes, "
          f"{len(tasks)} questions x {len(arms)} arms)")


if __name__ == "__main__":
    main()
