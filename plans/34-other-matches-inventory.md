# Plan 34 — `other_matches` evidence inventory in search_code (2026-11-02) — REVERTED

## Context
Stop-early analysis (Fable 5.1, 2026-11-02): the baseline's Grep (`for_update|lock|Queued|Running|def `) returns an inventory of matching lines across the file, so the agent always continues to read; `search_code` returns merged line ranges of the matched symbols, and the one visible `for_update(skip_locked=True)` in `schedule_next_date` reads as the whole answer. `also_in_file` (names only) was ignored by the agent (Plan 31 note); the nested-result dedupe got the right function into the results but did not change behaviour (Plan 33). This tries evidence lines, which is what a Grep result shows.

## Change
- Each `search_code` result file gets `other_matches`: up to 6 `line (enclosing function): text` entries from lines outside the matched symbols' spans that contain query words (length > 3, minus a small stoplist). A word counts only if it is on at most 8 lines of the file, and a line scores the sum of 1/(lines containing the word). Entries are in source order, text trimmed to 110 chars.
- Computed from the already-cached file text and `symbols_in_file`; no index, DB or tool-count change. Tool description OUTPUT line explains the field.
- Tests: rare lines outside the shown ranges are listed with the enclosing function and covered lines are skipped; a word on many lines is ignored.

## Offline replay (79 distinct recorded intents, twin-merge index)
Payload 17.5k chars/call (was ~16.0k, +1.5k); 18 `other_matches` lines per query on average (6 per file); only 1 query had none. Q22: for the lock-related intents the output now shows `105 (initialize_parallel_threads): .for_update(skip_locked=True)`; noise includes auto-generated-comment and `def` lines.

## Decision rule (set before the run)
Graphrelay only, Q22/Q27/Q02/Q13/Q19, 5 repeats. Signal: Q22 facts 2/4/5 and tool calls per Q22 run (baseline ~5.1, graphrelay ~2.3), Q27 tool calls. Keep for a full 30 x 3 run if Q22 tool calls rise and Q22 coverage improves beyond the 10-repeat noise (graphrelay 63.3%, sd 14) without a drop on the other questions beyond their earlier ranges; otherwise revert. Token growth (~+9% payload) must show up as a cost in the final comparison.

## Result (2026-11-03): reverted
Slice run (Q22/Q27/Q02/Q13/Q19, 5 repeats, `results/sot/erpnext-sonnet55-othermatches-r5/`): Q22 71.7% with facts 2-5 covered in 4 of 5 runs, but 2.2 tool calls (the agent still stops after one search), Q27 62.5%, Q02 57.1%, Q19 65.0%, Q13 70.0%; slice mean 65.3% vs 71.6% before.
Full run (30 x 3, `results/sot/erpnext-sonnet55-othermatches-full30-r3/`, final grades): 67.6% (repeats 70.1 / 64.1 / 68.7) vs 71.2% for the previous build (-3.5, CI [-7.9, +0.5], better on 12 / worse on 14) and 70.6% baseline (-3.0, CI [-8.4, +2.4]); the summary also counts 3 of 90 turn-cap or empty answers (earlier runs: 0), not investigated; 4.0 tool calls, 71.9k tokens/q, $0.097/q (previous 70.0k, $0.099). Q22 fell to 47.2% (the slice's 71.7% was a favourable draw), Q27 58.3%, Q21 -33, Q15 -25. The pre-stated rule (keep only if at least a tie with 71.2%) is not met.
Rerun of the same full 30 x 3 (`results/sot/erpnext-sonnet55-othermatches-full30-r3-again/`): 69.7% (repeats 69.8 / 69.8 / 69.6), 0 of 90 turn-cap or empty answers, 4.1 tool calls, 73.8k tokens/q, $8.24 for the 90 runs; Q22 77.8%. The first run's drop was mostly one weak repeat plus a bad Q22 draw. Pooled over both runs (6 repeats) the build is about 68.7%, 2.5 points under the previous build's 71.2% and about 2 under the baseline: no gain, so it stays reverted.
