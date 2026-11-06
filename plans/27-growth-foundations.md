# Plan 27 — Growth foundations (2026-10-26)

## Context
A strategy review found no documented growth strategy and a value proposition the evidence does not support: the 30-question agent benchmark is a tie with the baseline, graphrelay costs slightly more per question, the caller/impact group shows no lift, the repo is private, and the latest release predates the rename (the README installer cannot find a `graphrelay-*` archive).

## Scope (what this plan did / does)
1. **Structural slice.** 12 caller/impact tasks with `ast`-derived keys (`benchmarks/dataset/tasks/structural_tasks.yaml`, generator `harness/make_structural_tasks.py`), preregistration (`tasks/STRUCTURAL_PREREGISTRATION.md`), `run-sot.sh --structural`, and an LLM-free `harness/impact_recall.py`.
2. **Positioning.** Remove the unsupported "useful for structural questions" claim (README, intro video); state only what is supported (local/private, accurate static call graph, graph UI); report cost honestly.
3. **Beachhead + more repos.** Recorded as hypotheses in `ROADMAP.md` with real HEAD SHAs; no setup scripts yet.
4. **Distribution.** Drafts only: Homebrew formula template, launch-post draft. Blocked on a public repo and a GraphRelay-named release.
5. **Web UI as second product.** Documented in README; no new UI work.
6. **Roadmap + sustainability.** `ROADMAP.md`, with the sustainability model left as an owner decision.

## Success criteria
- Structural slice agent run (`--structural --repeats 3`, both agents) recorded under `results/sot/`; decision rule in the preregistration applied as written. **Done 2026-10-29: rule not met on ERPNext or Gitea (tie within the noise; fewer tool calls and lower cost with the tool).**
- No README/docs/video claim about subgroup performance that the data does not support.

## Invariants
- Answer keys never come from graphrelay output.
- Preregistration is not edited after a run starts; deviations are appended.
- Nothing is published or submitted to an external service without the owner's go-ahead.

## Not done
- Extra-repo question sets; the SWE-bench pilot run; any release, tap or directory submission; the visibility and sustainability decisions.
