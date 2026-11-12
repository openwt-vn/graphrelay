# Plan 37 — loss audit and edge resolution tag (2026-11-09)

## Context
What limits the agent benchmark? Inputs: the one retained per-fact grade file (`results/sot/2026-10-07-sonnet55-30/claude-sonnet-5-5/rep1/grades.json`, 1 repeat, pre-rename `codegraph` arm) and the ERPNext index.

## Audit findings
- Every fact the graph arm lost to baseline on Q21, Q24, Q04, Q08, Q28 and Q22 is a missing detail or enumeration item (a function name, a test file, a second savepoint site, the wrapping function). None is a missing graph edge. The graph is already accurate: `harness/impact_recall.py` recall 12/12, precision 0.91.
- These are single-repeat numbers (swings of 5-14 points per arm). Only Q22 is a confirmed loss (Plan 31, 10 repeats).
- Test-file penalty lead (Q21 missed a test-file fact): rejected. Q21's question does not ask about tests, so `TEST_FILE_PENALTY` (0.15) is applied as designed, and the missed file is an incidental key fact.
- Stop-early (#2): Plans 31, 33 and 34 already tried wording, `also_in_file`, nested dedupe and `other_matches`; all tied or lost. No new mechanism is justified by this evidence, so none was built.
- Cost (#3): the payload trim (Plan 31) saved no tokens. Not repeated.
- ERPNext Calls edges (an index built before Plans 35/36): 29% resolved; of all Calls edges 8.2% unique-name, 5.7% multi-candidate (import or in-degree tie-break), 11.6% ambiguous with no import signal, 59% no candidate (external). `self.method()` is covered by Plan 36.

## Change
- Migration 018 adds `edges.resolution` (`unique` / `import` / `tiebreak`; NULL = resolved before the column). `decide_resolutions` returns the method, `apply_resolutions` stores it, and `resolve_edges` logs the counts ("edge resolution"). Both the full pass and the live single-file path share `decide_resolutions`, so the tag is consistent.
- Not exposed through MCP and not used in ranking: nothing shows either helps (Plan 34 history; CLAUDE.md rule on rejected levers).

## Success criteria
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass.
- Resolution counts are visible in the index log, so the ambiguous share can be re-measured on a fresh index before any resolver change.
- A 1-repeat benchmark is recorded; it is not a result (CI spans 0), only a regression check.

## Not done
Receiver-aware resolution: measured and rejected, see the last section.

## Result (2026-11-09): regression check, a tie
`results/sot/erpnext-sonnet55-plan37-r1/`, Sonnet 5.5, ERPNext, 30 questions x 1 repeat, graphrelay + baseline: graphrelay 68.7%, baseline 71.6% (-2.9, CI [-9.3, +3.8]), better on 8 / worse on 13 / tied on 9; 4.3 vs 5.1 tool calls; $2.79 vs $1.97; 0 of 30 turn-cap or empty answers in either arm. The interval includes 0 and one repeat swings 5-14 points, so this is neither a gain nor a regression, as expected for a change the agent cannot see. The build under test was `0c26496` plus this change; `RUN.txt` records `+dirty` because the change was not yet committed.
The audit's loss list did not replicate: Q21 went from -25 to +17, Q24 and Q28 tied; only Q08 (-25) and Q04 (-17) lost again, and Q22 lost (-17, the known loss). Q26 changed sign across runs (+37 in the 2026-10-07 run, -38 here). This supports reading the earlier per-question losses as mostly noise.

## Fresh-index count and rerun (2026-11-09)
Forced full index of ERPNext (36 s, 4,825 files, 43,067 symbols, 365 synthesized edges). Calls edges: 58.9% external, 30.0% resolved, 11.1% (10,141) ambiguous with no import signal. Resolved with a tag: unique 8,744, import 2,884, tiebreak 1,423 (about 1.5% of Calls), synthesized 365; about 14,050 resolved at parse time carry no tag.
Rerun (`results/sot/erpnext-sonnet55-plan37-r2/`, same settings, fresh index): graphrelay 69.1%, baseline 70.1% (-1.0, CI [-9.0, +6.9]); better 11 / worse 11 / tied 8; 4.3 vs 5.6 tool calls; $2.62 vs $2.05; 0 of 30 turn-cap in either arm. Pooling both repeats: -1.9, CI [-8.3, +4.7]. A tie. Per-question results flipped between repeats (Q04 -17 then -50; Q26 -38 then -50, after +37 in the 2026-10-07 run), so per-question differences are noise.

## Receiver-aware resolution: ceiling measured, not built (2026-11-09)
Python-only ambiguous Calls edges on the older index (9,627 over 270 names; the 10,141 above covers all languages on the fresh index) are dominated by generic names: `get` 3,197, `submit` 1,469, `save` 1,462, `run` 993, `cancel` 418 (78% of them). A scan of ERPNext for call sites of those names (17,989) by receiver: local variable 83.8% (`doc.save()`, `d.get(...)`), `self` 9.5%, dotted attribute 4.8%, `frappe.*` 1.8%. `self` is already handled by Plan 36; a local-variable receiver needs type inference, which tree-sitter extraction does not do, and many of these targets are Frappe or dict methods outside the repo, so resolving them by name would create false edges. Decision: not built. The ambiguous bucket is mostly generic external-looking names, not a recoverable gap.
