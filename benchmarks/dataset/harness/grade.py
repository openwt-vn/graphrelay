#!/usr/bin/env python3
"""Judge — key-fact coverage grading, replicating graphify's method.

For each answer, a judge model grades every gold key fact as
covered | partial | missing, citing a verbatim quote from the answer.
    coverage = (covered + 0.5 * partial) / total

The judge is blind to which arm produced an answer: rows from all arms are
shuffled deterministically and the prompt carries no arm/tool metadata.

Judge model: claude-sonnet-5, ALWAYS against the real Anthropic API — this
is fixed regardless of --results-dir, deliberately different from whichever
agent model produced the answers being graded, to reduce self-preference
bias. Grading a non-default agent model's results (e.g. DeepSeek) does not
change what grades them.

Usage:
    python3 harness/grade.py --results-dir results/claude-code/claude-opus-4-8
    python3 harness/grade.py --results-dir results/claude-code/claude-opus-4-8 --arm graphrelay
"""

from __future__ import annotations

import argparse
import json
import os
import random
import re
import subprocess
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
TASKS_FILE = ROOT / "tasks" / "erpnext_tasks.yaml"

JUDGE_MODEL = "claude-sonnet-5"
SHUFFLE_SEED = 20260708

JUDGE_OUTPUT_FORMAT = """\
Respond with ONLY a JSON object, no other text, matching this shape:
{"verdicts": [{"fact_index": <int>, "verdict": "covered"|"partial"|"missing", \
"quote": "<verbatim substring of the answer, or empty string for missing>", \
"reasoning": "<one sentence>"}]}"""

JUDGE_SYSTEM = """\
You are a strict, impartial grader for a code-intelligence benchmark on the \
ERPNext codebase. You receive a question, a list of gold atomic key facts a \
correct answer must contain, and a candidate answer. For EACH key fact, decide:
- covered: the answer states this fact (exact wording not required, but the \
specific function/class/file names and the described behavior must match).
- partial: the answer gets part of the fact right (e.g. right file but wrong \
function, right mechanism but wrong name, vague but pointing the right way).
- missing: the answer does not contain the fact or contradicts it.

Every covered/partial verdict MUST cite a verbatim quote from the answer: \
copy-paste a short exact substring (5-15 words) character-for-character from \
the <answer> block, including any markdown like ** or backticks exactly as \
they appear. Do NOT paraphrase, reformat, or reconstruct the quote from \
memory — an inexact quote will be programmatically rejected and the verdict \
discarded. If the supporting text is longer than needed, you may elide a \
middle portion with "..." — but every fragment on either side of "..." must \
still be an exact, character-for-character substring of the answer, in the \
same order it appears there; do not use "..." to paper over a paraphrase. \
If you cannot find a supporting exact substring, the verdict is missing and \
quote is "". Do not reward name-dropping without the described \
relationship. Grade each fact independently."""
JUDGE_SYSTEM = JUDGE_SYSTEM.replace("ERPNext codebase", os.environ.get("BENCH_REPO_LABEL", "ERPNext") + " codebase")


def _extract_json(text: str) -> dict:
    """Pull the JSON object out of judge output that may carry stray prose."""
    start = text.find("{")
    end = text.rfind("}")
    if start == -1 or end == -1:
        raise ValueError(f"no JSON object found in judge output: {text[:200]!r}")
    return json.loads(text[start:end + 1])


def _strip_md(s: str) -> str:
    """Markdown emphasis (backticks, **bold**) stripped before quote
    comparison — judges reliably transcribe the words correctly but
    sometimes drop surrounding ` or ** characters, which is not the
    paraphrasing/hallucination the verbatim-quote audit targets."""
    return s.replace("`", "").replace("**", "")


def _quote_is_verbatim(quote: str, answer_text: str) -> bool:
    """True if every fragment of `quote` (split on a "..."/"…" elision
    marker) is an exact substring of `answer_text`, occurring in order.

    A judge legitimately eliding a real, verbatim middle portion for length
    (explicitly permitted in JUDGE_SYSTEM) must not be penalized the same as
    a genuine paraphrase/hallucination — found live 2026-09-28 (erpnext Q05,
    `graphrelay` arm): the judge's reasoning matched the gold fact exactly
    and the answer really did contain the substance almost word-for-word,
    but the old whole-string check rejected it purely because "..." isn't
    literal text. Each surviving fragment still has to be a real, exact,
    in-order substring — this doesn't weaken the anti-hallucination
    guarantee, it only stops treating "..." itself as an automatic
    paraphrase. `answer_text` is expected to already be markdown-stripped
    via `_strip_md` by the caller.
    """
    fragments = [f.strip() for f in re.split(r"\.\.\.|…", quote)]
    fragments = [f for f in fragments if f]
    if not fragments:
        return False
    pos = 0
    for frag in fragments:
        idx = answer_text.find(frag, pos)
        if idx == -1:
            return False
        pos = idx + len(frag)
    return True


def judge_answer(question: str, key_facts: list[str], answer: str) -> list[dict]:
    facts_block = "\n".join(f"[{i}] {f}" for i, f in enumerate(key_facts))
    user = (
        f"QUESTION:\n{question}\n\n"
        f"GOLD KEY FACTS:\n{facts_block}\n\n"
        f"CANDIDATE ANSWER:\n<answer>\n{answer}\n</answer>\n\n"
        f"Grade all {len(key_facts)} facts.\n\n{JUDGE_OUTPUT_FORMAT}"
    )
    cmd = [
        "claude", "-p", user,
        "--model", JUDGE_MODEL,
        "--max-turns", "1",
        "--output-format", "json",
        "--system-prompt", JUDGE_SYSTEM,
        "--setting-sources", "",
        "--allowedTools", "",
        "--disallowedTools", "*",
    ]
    for attempt in range(3):
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
        try:
            payload = json.loads(proc.stdout)
            result_text = payload.get("result", "")
            verdicts = _extract_json(result_text)["verdicts"]
            break
        except (ValueError, KeyError) as e:
            if attempt == 2:
                raise RuntimeError(
                    f"judge failed to return parseable JSON after 3 attempts: "
                    f"{e}\nstdout: {proc.stdout[:500]}\nstderr: {proc.stderr[:500]}"
                ) from e

    # Audit rule: a covered/partial verdict whose quote is not verbatim in
    # the answer is downgraded to missing. See `_quote_is_verbatim`.
    stripped_answer = _strip_md(answer)
    for v in verdicts:
        if v["verdict"] != "missing" and not _quote_is_verbatim(_strip_md(v["quote"]), stripped_answer):
            v["verdict_original"] = v["verdict"]
            v["verdict"] = "missing"
            v["audit_note"] = "quote not found verbatim in answer"
    return verdicts


def coverage(verdicts: list[dict]) -> float:
    covered = sum(1 for v in verdicts if v["verdict"] == "covered")
    partial = sum(1 for v in verdicts if v["verdict"] == "partial")
    return (covered + 0.5 * partial) / len(verdicts) if verdicts else 0.0


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", required=True,
                    help="directory of <arm>-results.jsonl to grade, e.g. "
                         "results/claude-code/claude-opus-4-8 — required "
                         "(no default) since results now live one directory "
                         "per (agent, model), never merged")
    ap.add_argument("--grades-file", default=None,
                    help="default: <results-dir>/grades.json")
    ap.add_argument("--arm", help="grade a single arm only")
    ap.add_argument("--regrade", action="store_true",
                    help="re-grade pairs that already have a grade")
    ap.add_argument("--tasks-file", default=str(TASKS_FILE),
                    help="alternate question set matching --results-dir's "
                         "question set (default: tasks/erpnext_tasks.yaml)")
    args = ap.parse_args()

    results_dir = Path(args.results_dir)
    out = Path(args.grades_file) if args.grades_file else results_dir / "grades.json"

    tasks = {t["id"]: t for t in
             yaml.safe_load(Path(args.tasks_file).read_text())["tasks"]}

    grades = json.loads(out.read_text()) if out.exists() else []
    already = {(g["task_id"], g["arm"]) for g in grades}

    rows = []
    for f in sorted(results_dir.glob("*-results.jsonl")):
        arm = f.name.replace("-results.jsonl", "")
        if args.arm and arm != args.arm:
            continue
        for line in f.read_text().splitlines():
            if line.strip():
                rows.append(json.loads(line))

    if not args.regrade:
        rows = [r for r in rows if (r["task_id"], r["arm"]) not in already]
        grades = [g for g in grades if (g["task_id"], g["arm"]) not in
                 {(r["task_id"], r["arm"]) for r in rows}]

    # Blind grading: deterministic shuffle, no arm metadata in the prompt.
    random.Random(SHUFFLE_SEED).shuffle(rows)
    print(f"Grading {len(rows)} answers with {JUDGE_MODEL} "
         f"({len(already)} already graded, skipped unless --regrade)...")

    if args.regrade:
        grades = [g for g in grades if (g["task_id"], g["arm"]) not in
                 {(r["task_id"], r["arm"]) for r in rows}]

    for row in rows:
        task = tasks[row["task_id"]]
        verdicts = judge_answer(task["question"],
                                task["key_facts"], row["answer"] or "")
        cov = coverage(verdicts)
        grades.append({
            "task_id": row["task_id"],
            "arm": row["arm"],
            "coverage": round(cov, 4),
            "n_facts": len(task["key_facts"]),
            "verdicts": verdicts,
        })
        print(f"  {row['task_id']} [{row['arm']}]: coverage={cov:.1%}",
             flush=True)
        out.write_text(json.dumps(grades, indent=2, ensure_ascii=False))

    print(f"Wrote {out}")


if __name__ == "__main__":
    main()
