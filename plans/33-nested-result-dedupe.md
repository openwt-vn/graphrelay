# Plan 33 — drop nested results from search_code and backfill (2026-11-02) — REVERTED (`1ccb07a`)

## Context
Stop-early analysis (Fable 5.1, read-only, 2026-11-02): 7.3% of `search_code` result slots (219 calls) are a symbol whose lines sit inside a higher-ranked same-file result (methods of an already-returned class). They add no source (ranges merge) but take slots, so on Q22 `initialize_parallel_threads` (#12, rerank margin 0.007) never reaches the top 8; the agent then sees only `schedule_next_date` and answers. Wording, `also_in_file`, payload trim and decorator twin merge (Plans 31/32) did not help.

## Change
- `find_relevant_code::handle` asks the pipeline for `top_n + 8`, then `drop_nested_results` removes a result whose lines are inside a higher-ranked same-file result whose span is at most `per_item_chars` bytes (so its source is rendered whole), and keeps the first `top_n`.
- No index, DB or tool-count change. Unit tests: nested dropped and backfilled; an oversized container does not hide its members.

## Offline replay (181 distinct recorded intents, twin-merge index, REST top-8 vs the new `search_code`)
- Top-8 set changed in 22 queries; mean 0.21 symbols swapped per query; payload 16.0k chars/call (recorded earlier runs: ~15.8k).
- Anchor files (`anchor_files` in `erpnext_tasks.yaml`): no query lost one; Q22 gained one. Symbol-level anchor losses (Q09 3, Q22 1, Q30 4) are nested members of a returned container.
- Q22: `initialize_parallel_threads` in the top 8 for 8 of 16 intents (was 3 of 16).

## Decision rule (set before the run)
Graphrelay-only, Q22/Q02/Q13/Q19, 5 repeats: signal is Q22 facts 2/4/5. Fact 0 (`on_submit`, string dispatch) is not expected to move. Keep if Q22 improves without a drop on the other three beyond their earlier 3-repeat range; otherwise revert. A full 30 x 3 run would follow before any claim beyond that.

## Result (2026-11-02): reverted
Graphrelay only, Q22/Q02/Q13/Q19, 5 repeats (`results/sot/erpnext-sonnet55-nested-r5/`), Sonnet 5.5. Q22 65.0% (67/50/58/83/67; the 83 is a regrade, see below) vs 69.4% before (3 repeats) and 63.3% over 10 repeats, baseline 80.6%; 2.4 tool calls, so the agent still stops after one search. Q02 61.4% (was 69.0%), Q13 70.0% (61.1%), Q19 68.3% (75.0%). No Q22 gain beyond noise, so the pre-stated rule says revert.
One Q22 grade came back with 2 of 6 verdicts (0.0%) on a real 2.8k-char answer; that item was regraded to 83.3% (backup `grades.before-regrade.json`). The same regrade moved Q19 75% -> 50% and Q02 64% -> 50% on identical answers, so the judge alone varies by up to 25 points per answer.
Offline replay had shown `initialize_parallel_threads` in the top 8 for 8 of 16 intents (was 3 of 16): getting the function into the results did not change what the agent did.
