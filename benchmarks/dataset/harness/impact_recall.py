#!/usr/bin/env python3
"""LLM-free structural metric: score trace_impact against the ast-derived keys of tasks/structural_tasks.yaml.

Needs a daemon with ERPNext indexed at the pin (see run-sot.sh --setup-only, or `gr index && gr start`).
Costs nothing: no model is called. Compares (file, leaf function name) sets; precision/recall per task.
Usage: impact_recall.py [--url http://localhost:3030/<project>/mcp] [--tasks tasks/structural_tasks.yaml] [--out ...]
"""
import argparse, json, re, urllib.request
from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parent.parent

def call(url, symbol, depth):
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "trace_impact", "arguments": {"symbol": symbol, "depth": depth}}}
    req = urllib.request.Request(url, json.dumps(body).encode(), {"content-type": "application/json", "accept": "application/json, text/event-stream"})
    out = json.load(urllib.request.urlopen(req, timeout=60))
    return json.loads(out["result"]["content"][0]["text"])

def leaf(qual): return re.sub(r"#\d+$", "", re.split(r"[.:]+", qual)[-1])  # "#2" marks a same-name sibling in one file

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://localhost:3030/erpnext/mcp")
    ap.add_argument("--tasks", default=str(ROOT / "tasks" / "structural_tasks.yaml"))
    ap.add_argument("--out", default=None, help="default: results/structural/impact_recall[_<tasks stem>].json")
    a = ap.parse_args()
    tasks = yaml.safe_load(Path(a.tasks).read_text())["tasks"]
    stem = "" if Path(a.tasks).stem == "structural_tasks" else Path(a.tasks).stem.replace("structural_", "").replace("_tasks", "")
    a.out = a.out or str(ROOT / "results" / "structural" / (f"impact_recall_{stem}.json" if stem else "impact_recall.json"))
    rows, tp_all, fp_all, fn_all = [], 0, 0, 0
    for t in tasks:
        target = t["origin_note"].split()[-1]
        depth = 2 if "callers-of-callers" in t["origin_note"] else 1
        key = {(m.group(2), leaf(m.group(1))) for f in t["key_facts"] if (m := re.match(r"`([^`]+)` in (\S+) ", f))}
        res = call(a.url, target, depth)
        got = {(x["symbol"].split("::")[0], leaf(x["symbol"].split("::", 1)[1])) for x in res["affected"]}
        tp, fp, fn = len(key & got), len(got - key), len(key - got)
        tp_all += tp; fp_all += fp; fn_all += fn
        rows.append({"id": t["id"], "target": target, "depth": depth, "key": len(key), "returned": len(got), "tp": tp, "fp": fp, "fn": fn,
                     "recall": round(tp / len(key), 3), "precision": round(tp / len(got), 3) if got else None,
                     "truncated": res.get("truncated"), "missed": sorted(map(list, key - got)), "extra": sorted(map(list, got - key))})
    micro = {"recall": round(tp_all / (tp_all + fn_all), 3), "precision": round(tp_all / (tp_all + fp_all), 3) if tp_all + fp_all else None}
    Path(a.out).parent.mkdir(parents=True, exist_ok=True)
    Path(a.out).write_text(json.dumps({"micro": micro, "tasks": rows}, indent=1))
    print(f"{'id':4} {'target':45} d key got  tp  fp  fn  recall  prec")
    for r in rows:
        print(f"{r['id']:4} {r['target'][:45]:45} {r['depth']} {r['key']:3} {r['returned']:3} {r['tp']:3} {r['fp']:3} {r['fn']:3}  {r['recall']:.2f}   {r['precision'] if r['precision'] is None else format(r['precision'],'.2f')}")
    print("micro:", micro)

if __name__ == "__main__": main()
