# Pre-registration — release-build smoke run, Claude Sonnet 5 (2026-09-30)

Written before any tool-arm result existed (the one baseline Q01 smoke result, used to check the
harness, predates it and is kept as the baseline arm's Q01 row).

- **Questions:** the first question of each capability group of `tasks/erpnext_tasks.yaml`:
  Q01 (A), Q06 (B), Q11 (C), Q16 (D), Q21 (E), Q26 (F). Six, not five, so every group is covered once.
  Chosen by this rule, not by looking at results.
- **Arms:** baseline, codegraph, graphify, code-review-graph.
- **Agent:** `--provider claude-sonnet-5` (claude-sonnet-5), max 30 turns, Grep/Read/Glob floor + one MCP server.
- **Judge:** claude-sonnet-5, blind, verbatim quotes required (unchanged `grade.py`).
- **codegraph under test:** the release build (`cargo build --release`) of commit 1f9038d on branch optimize/plan24-25-indexing-laya, run as its own daemon on :3030 from an isolated HOME (the user's live indexes were not touched); erpnext freshly indexed at pin 1887825ce5 (4,770 files, 44,062 symbols).
- **graphify / code-review-graph:** graphs rebuilt at the same pin the same day (the code-review-graph database was dated Sep 17, before the Sep 27 re-pin).
- **Not claimed:** n=6 per arm, one run each, LLM agent and LLM judge, so nothing here can establish a statistically significant difference between arms; McNemar needs many more questions.
