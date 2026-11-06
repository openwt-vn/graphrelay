# Plan 30 — `via` on trace_impact results (2026-10-30)

## Context
In the Gitea Sonnet 5.5 re-run (`results/sot/structural-gitea-2026-10-30-sonnet55-receiver-fix/`), S10 repeat 3 scored 0.82 because the agent wrote "I don't know which hop-1 function each one calls" for five second-hop callers: `trace_impact` returned a flat list with `hop_depth` only.

## Change
- `ImpactNode.via: Option<String>`: qualified name of the symbol one hop closer to the root that the node depends on, on the BFS shortest path; `None` at hop 1. It is the first parent found (deterministic: `ORDER BY s.id, e.to_id, e.kind`); other parents may exist.
- Exposed as `via` in `trace_impact` output; tool description updated.
- `run-sot.sh --structural` accepts `--questions` as a subset filter (ids validated against the tasks file).

## Decision rule (set before the run)
Keep if (a) `graphrelay` coverage on S07/S10 does not drop vs the earlier runs, (b) answers stop saying they don't know which direct caller leads to a second-hop one, (c) payload growth is small.

## Result (exploratory, S07+S10 only, 5 repeats, both arms, `results/sot/structural-gitea-via-s07-s10-sonnet55/`)
- graphrelay 99.3% vs baseline 98.2% (+1.1, CI [-1.4, +3.6], 2 questions). S07: 100% in 5/5 (baseline 96.4%). S10: 98.6% (one miss: an answer excluded `InitIntegrationTest` as test-only, unrelated to `via`).
- `via` present in all five S10 tool results (12 values each). No answer said it did not know which hop-1 function a hop-2 caller goes through; the remaining hedge is "did not open the call sites", a different limitation (no call-site lines).
- Payload: S10 4.9K -> 5.5K chars (+13%), S07 8.3K -> 9.6K (+16%).
- Rule met on (a), (b), (c). Caveat: the parentage hedge appeared in only 1 of 6 earlier S10 repeats, so 0 of 5 now is weak evidence; coverage is at the ceiling, so no score gain is demonstrable.

## Not done
- `lines` on trace_impact items (would let agents map Grep call lines to functions; the other hedge source). Call-site lines need edge spans, which `Edge` does not store.
- `via` lists one parent, not all.
