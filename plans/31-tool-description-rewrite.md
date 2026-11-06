# Plan 31 — Tool description rewrite and `lines` on trace_impact (2026-10-30)

## Context
An Opus 5.5 review of the four MCP tool descriptions (read-only, 132 benchmark transcripts: 30 ERPNext Sonnet 5.5, 30 ERPNext GLM, 72 Gitea Sonnet 5.5) found:
- `search_code` said that with `source_truncated: false` "you already have everything relevant in that file — do NOT follow up with inspect/Read". `source` is only the matched symbols' ranges (`find_relevant_code.rs:62-77`). Agents ignored the rule: 26 Sonnet and 72 GLM Reads hit already-shown files, nearly all at ranges that had not been shown.
- `inspect(path)` was called "a direct Read substitute"; its cap is 4,000-8,000 chars by repo size (`mod.rs:207-221`). 14 of 16 GLM responses were truncated; `focus_line` was used 0 of 24 times.
- `trace_impact`'s advised follow-up (`inspect`) was used 0 of 77 times; 53 of 72 Gitea transcripts re-Grepped the root name and none found a caller the trace missed; 43 of 72 `graphrelay` answers hedged vs 14 of 72 baseline.
- `watch_status` invited proactive calls (GLM: 9, all `fresh`) and spent ~400 chars on Jev/embeddings.
- Wording claimed "semantic" matching; retrieval is FTS5/BM25 plus 1-hop graph expansion (no embeddings).
- Confound: `harness/run.py` MCP_NOTE forces an MCP call before Grep/Read, so "MCP tool first" in transcripts says nothing about the descriptions.

## Change
- All four descriptions rewritten to one shape: what it does / USE WHEN / DON'T USE / FLOW / OUTPUT / LIMITS. Sizes 1,734 / 1,675 / 1,601 / 883 chars (total 5,893, was 6,971).
- Numeric caps are interpolated from the constants the handlers enforce (`response_budget`, `MAX_AFFECTED`, `MAX_SHOWN`, `MAX_TOP_N`, `MAX_DEPTH`, `CALLER_CALLEE_DISPLAY_CAP`) so descriptions cannot drift from the code.
- Parameter descriptions and schema bounds updated (`language` values, `top_n` counts symbols, `focus_line` 1-based).
- `trace_impact` items gain `lines` (the dependent's own 1-based range). Call-site lines are NOT provided (`Edge` stores no spans); the description says so.
- Test `every_tool_description_says_when_to_use_and_when_not_to`: each description has USE WHEN / DON'T USE, the three chaining tools have FLOW, per-tool length cap, no mention of embeddings.

## Success criteria
- 545 tests + clippy clean. Done.
- Behaviour effect measured: `run-sot.sh` on ERPNext, Sonnet 5.5, same arms as `2026-10-07-sonnet55-30`; compare coverage, tool calls, same-file Reads after inspect, Greps after trace_impact, hedge rate. **Open** (single repeat proves nothing; repeat swings of 5-14 pts were seen).

## Not done
- `source_scope`/`callers_count_is_lower_bound` machine-readable flags (review items C2, C3), `note` on an unmatched `language` (C4).
- Omitting `via: null` at hop 1 (C5).

## Measured effect and follow-up (2026-10-30)
Clean ERPNext run (Sonnet 5.5, 30 questions, 1 repeat, `results/sot/erpnext-sonnet55-clean/`; the earlier run was contaminated by `graphify-out/` and an old `.codegraph/` inside the shared checkout, now removed and the graphify arm moved to `repos/erpnext-graphify/`): baseline 72.2%, graphrelay 68.7% (-3.5, CI [-8.8, +2.3]), laya 71.4%, colby 69.5%, jev 68.1%, graphify 66.7% (-5.5, CI [-10.9, -0.2]). No arm beats the baseline; the description rewrite shows no coverage benefit on its own.

Investigation of the five questions graphrelay lost most (Q20, Q05, Q25, Q27, Q30), baseline vs graphrelay, 3 repeats each (`erpnext-sonnet55-losers-r3`): 69.8% vs 55.5% (-14.3, CI [-23.1, -5.5]); the first run's -16 was partly selection. graphrelay used fewer tool calls (4.2 vs 5.7) and stopped after one `search_code` hit on "is there X / what is the default" questions.
- Added to `search_code` LIMITS: a result shows what exists, never what is absent; Grep settings/hooks/sibling files before concluding. 3 repeats: 55.5% -> 60.5% (inside the noise); Q27 and Q20 unchanged.
- Added to `search_code` DON'T USE: non-code files (JSON fixtures, configs, templates, docs) are not indexed; Glob/Grep them. Q27, 3 repeats: 62.5% -> 70.8% (50/75/88); the Notification-records fact [2] went missing -> partial in 2 of 3. Not a reliable fix.
- Root causes, from transcripts: Q27 fact [2] lives in JSON fixtures (not indexed); Q20's five gold sites are tiny `order_by`/`.orderby(...)` arguments written in three styles, so neither BM25 nor one regex finds them (the baseline found one in six runs); Q25 is a ranking problem on high-frequency words (`IfNull`, `coalesce`, "null"). Not fixed; indexing JSON was rejected (new "language", dilutes BM25 and the df cutoff, one fact).

## Final numbers (2026-11-02, Sonnet 5.5, ERPNext, 30 questions x 3 repeats, `results/sot/erpnext-sonnet55-full30-r3/`)
| Arm | Coverage | Tool calls/q | Tokens/q | $/q |
|---|---|---|---|---|
| baseline | 70.6% | 5.6 | 47.0k | 0.076 |
| graphrelay (`8cb26e8` build) | 71.2% (+0.6, CI [-4.0, +5.0]) | 4.0 | 70.0k | 0.099 |
| graphify | 66.9% (-3.7, CI [-8.6, +0.7]) | 6.2 | 65.8k | 0.091 |
| codegraph-colby | 67.1% (-3.5, CI [-8.5, +1.3]) | 4.6 | 88.3k | 0.122 |

No turn-cap or empty answers in any arm (0/90 each). Graphrelay ties the no-tool baseline on coverage (better on 12 questions, worse on 14) with ~30% fewer tool calls and ~1.3x the cost per question. Later experiments, each a 3-repeat graphrelay-only run compared with the row above:
- Payload trim (`22e2dfb`, 1 repeat 67.1%): no token or cost saving; reverted (`24d62ad`).
- Flow hint in LIMITS (Q22/25/27/30 only, 66.3%): no gain; not committed.
- `also_in_file` (`ebf8994`, 70.2%, tokens +5%, Q22 63.9%): no gain; reverted (`98f6ba6`).
- Decorated-Python twin merge (Plan 32, `81ecf13`, 69.6%, CI vs previous [-4.6, +1.3]): kept as a correctness fix; 44,354 -> 43,067 symbols.

Q22 (period closing voucher) is the one confirmed loss: 10 repeats, graphrelay 63.3% vs baseline 75.8% (-12.5, CI [-22.5, -2.5]), 2.3 vs 5.1 tool calls. Cause: the single `search_code` returns the whole `schedule_next_date` body, which looks like a complete answer, so the agent skips `initialize_parallel_threads` (ranked #12 by a 0.007 rerank margin) and `process_individual_date`; seeing the name in `also_in_file` did not change that. Q27 swings with the same stop-early pattern: runs with 3 tool calls scored 0.375-0.5, runs that also Glob/Read scored 0.875-1.0 (the missed facts are JSON fixtures and `hooks.py`). Q25 is inside the noise (10 repeats, -5.8, CI [-20.0, +8.3]). Q20 is a tie (neither tool finds the five `order_by` sites).

## Still open
- The stop-early pattern (Q22, Q27); no wording, output or ranking change tried so far moves it.
- Indexing time: this plan's last runs took more than 5 minutes to index ERPNext on a machine that `checklist.md` records at 31 s idle; not investigated (see the session notes in the PR).

## Parser queries (2026-10-30, found while profiling the index)
The JavaScript extractor logged `Invalid node type generator_function_expression` once per file: the grammar's node is `generator_function`. Fixed. The Scala `case_class_definition` query had the same problem (case classes are `class_definition` in this grammar) and was removed. New test `every_symbol_query_compiles_against_its_grammar` compiles every language's symbol queries.
