# Plan 36 — Python `self.method()` resolved up the inheritance chain

## Problem
The base resolver keys call edges on the bare method name. A `self.shared()` call whose `shared` lives on a parent class in another file stays unresolved when the name is not unique in the repo (e.g. `StockEntry.on_cancel` -> `StockController.repost_future_sle_and_gle`).

## Measurement before building (ERPNext index, 19,800 `self.x()` calls inside methods)
- 6,623 target a method of the same class; 6,522 already resolve to it.
- 702 target a method defined on a repo ancestor; 328 already resolve, 374 do not.
- 12,475 target nothing in the repo (Frappe `Document` methods such as `get`, `save`, `db_set`): not recoverable, the framework is not indexed.
- Ceiling: about 475 new edges, 0.5% of the 91k call edges. No call is left unresolved when one same-language function has that name, so there is no cheap by-name gain.

## Change
`crates/graphrelay-parser/src/synthesis/py_self_calls.rs` (synthesized edges, rebuilt on each full index; no schema change). For a Python method, each `self.name(` resolves to the same class's `name`, else the first definition up the class's resolved `Inherits` chain (DFS, 64 ancestors max, cycle safe). Pairs already produced by the base resolver are skipped. `GraphDb::resolved_edge_pairs(kind, lang)` feeds the Inherits map and the existing-pairs set; wired into the chunked driver in `graphrelay-watcher/src/worker.rs`.

## Result
ERPNext: 368 synthesized edges in total (26 from Plan 35, 342 from this pass); resolved call edges 27,101 -> 27,469 (+1.4%). First build failed silently (`synthesis failed` warning only): symbol ids are u64 stored as i64 and the new query read them unsigned. Fixed; the log line is the check.

## Benchmark (2026-11-03): kept, a tie
Full 30 x 3 (`results/sot/erpnext-sonnet55-selfcalls-full30-r3/`): 70.5% (repeats 70.9 / 71.6 / 69.0), 0 of 90 turn-cap or empty answers, 4.4 tool calls, $8.51. Paired by question: +1.5 vs Plan 35 (CI [-0.4, +3.4], better 13 / worse 7), +0.9 vs Plan 32 (CI [-1.4, +3.3], 12 / 12), -0.7 vs the 71.2% Plan 31 build (CI [-3.6, +2.4]). Every interval includes 0, and the baseline is 70.6%: no measurable effect, as the 342-edge size predicted. Kept as a correctness change.

## Review follow-up (2026-11-03)
PR review found: one edge per call site and phantom edges from docstrings/comments; nested classes and `name#N` siblings skipped because the class was derived from the qualified name. Now the class is found by span containment, one edge per target, and strings/comments are stripped first. ERPNext: 365 edges (was 368). Plan 35's pass now attributes a string to the narrowest enclosing function only (nested closures no longer double-attribute).
