# Plan 35 — Python dotted-path string dispatch edges

## Problem
`frappe.enqueue(method="pkg.mod.func")`, `frappe.get_attr("pkg.mod.func")` and similar call a function by a dotted string. No call expression names `func`, so the graph has no edge from the enqueuing function to it. This is the structural gap behind Q22 (`PeriodClosingVoucher` -> `process_period_closing_voucher` jobs): the agent sees a function that looks complete and never learns it hands work to others.

## Change
New synthesis pass `crates/graphrelay-parser/src/synthesis/py_string_dispatch.rs` (same `EdgeSource::Synthesized` posture as `c_fnptr` / `js_callback`). A quoted `a.b.c.func` (3+ parts) inside a Python function/method body becomes a `Calls` edge to a top-level Python function when its module part matches a project file path (suffix match on `/`-to-`.` converted relative path, so a root above the package still works) and the name is unambiguous. Skipped: module-level code (hooks.py), test files (`tests/`, `test_*.py`, where the strings are `patch()` targets), ambiguous targets, self-loops. Wired into `synthesize_all` and the chunked driver in `graphrelay-watcher/src/worker.rs` (Python symbols are now fetched there). No schema change, no migration: synthesized edges are rebuilt on every full index.

## Measured on ERPNext
169 resolvable dotted strings in the checkout, 49 outside hooks.py. The pass recovers 26 edges (index 43,067 symbols, synthesis 0.9 s), e.g. `initialize_parallel_threads` and `schedule_next_date` in `process_period_closing_voucher.py` now point at their enqueued jobs. Not covered: `get_doc({"doctype": ...})` (doctype name, not a dotted path), method targets (`Class.method`), hooks.py tables.

## Keep rule
Agent benchmark, ERPNext, 30 questions x 3 repeats: keep if at least a tie with the previous build (69.6% after Plan 32, 71.2% before). It is a correctness change with a tiny footprint (26 edges), so a tie is the expected best case.

## Result (2026-11-03): kept
Full 30 x 3 (`results/sot/erpnext-sonnet55-pystring-full30-r3/`): 69.0% (repeats 67.2 / 69.7 / 70.1), 0 of 90 turn-cap or empty answers, 4.2 tool calls, $8.57. Previous build (Plan 32) 69.6%, so -0.6 with repeat swings of 3 points: a tie, not a gain. Q22 75.0% (75-75), Q27 62.5%, Q20 38.1%, Q25 66.7%. Kept as a correctness fix (26 real edges, no cost), not as a benchmark win; the 26 edges are too few to move a 30-question mean.
