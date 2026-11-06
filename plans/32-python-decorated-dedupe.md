# Plan 32 — one symbol per decorated Python function (2026-11-02)

## Context
Q22 diagnosis (Opus read-only analysis of `results/sot/*` transcripts plus a replay of 215 recorded `search_code` intents on a scratch ERPNext index): `python_lang.rs` matches a decorated function twice, as the bare `function_definition` and as the wrapping `decorated_definition`. `extractor.rs` only dropped exact-span duplicates, so both survived as `name#1` / `name#2`. In ERPNext that is 1,286 same-line twins (every `@frappe.whitelist()` function). Callers resolved to `#1` (whose only callee is the decorator), the body's calls hung off `#2`, so `on_submit -> start_pcv_processing -> initialize_parallel_threads` was cut for PageRank, `trace_impact` and the callers/callees shown to the agent. Twins also took ~0.28 of the 8 result slots per query.

## Change
- `Extractor` symbol dedupe: within one base qualified name, drop a symbol when another has the same `end_byte` and an earlier `start_byte` (the wrapper wins). Applies to any language that produces such nested same-name spans.
- Migration `017_python_decorated_dedupe`: Python files reset to the pending sentinel (ids change), SCIP markers cleared. Existing projects re-index Python files once.
- Tests: `decorated_python_function_is_one_symbol_that_owns_its_calls`, `python_decorated_migration_resets_only_python_files`.

## Decision rule (set before the run)
This is a correctness fix; keep it unless the 30-question x 3 repeat graphrelay run drops more than the noise band against `erpnext-sonnet55-full30-r3` (71.2%, baseline 70.6%) with a CI that excludes 0. Q22 is not the metric; expected effect there is at most one fact (the status re-check). `initialize_parallel_threads` is expected to stay outside the top 8 (rerank margin 0.007) and `PeriodClosingVoucher.on_submit` has no static edge (string doctype).

## Not done
- Discounting `name_match_bonus` for query terms already in the candidate's file path (would lift `initialize_parallel_threads`; risky for navigation queries).

## Result (2026-11-02)
30 questions x 3 repeats, graphrelay only (`results/sot/erpnext-sonnet55-twinmerge-r3/`): 69.6% vs 71.2% before (-1.5, CI [-4.6, +1.3], better on 12 / worse on 13), tokens/q 72.0k vs 70.0k, 0 turn-cap. 44,354 -> 43,067 symbols (matches the 1,286 merged twins). Q22 unchanged (63.9%). Q27 fell 83.3% -> 58.3%, but the search output was near-identical in all six runs and the score tracks whether the agent made 3 or 5 tool calls (stop-early noise, not the merge). Rule met: the CI includes 0, so the fix stays.
