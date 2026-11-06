# Plan 25 — Laya as a second `RelevanceJudge` adapter

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

Status: **IMPLEMENTED 2026-09-29** (started 2026-09-29 at the user's request: "deeply analyze
github.com/NandhaKishorM/laya and add Laya as the new adapter with Jev for the decision
engine").

## 1. What Laya is (from the repository itself, fetched directly)

Sources read: `README.md`, `docs/http-api.md`, `laya/serve.py`, `pyproject.toml` at
`main` (v0.3.21), 2026-09-29. Treat the numbers below as the project's own claims.

- A **non-autoregressive decision engine**: an encoder (ModernBERT-large 421M for English,
  mmBERT-base 322M for 100+ languages, plus a "typed-decisions" fine-tune) that answers
  typed questions about a *state* in one forward pass — `choice` (labelled options),
  `score` (ordinal), `noul` (yes/no, **calibrated P(true)**). No text generation.
- **Self-hosted, Apache-2.0**, `pip install "laya[serve]"` → `laya-serve` (FastAPI).
  Claimed ~33-40 ms/question on a T4 GPU (much slower on CPU), 7.8× faster than Jev's
  236-276 ms p50; typed-decisions accuracy 0.766 vs Jev 0.727 (their benchmark, not ours).
- **Wire protocol = Jev's**: `POST /v1/systemone` with `{state, questions, model?}` →
  `{answers: {<id>: {type, noul|choice|score, probabilities, confidence,
  answer_confidence, ...}}, usage, routing}`. It additionally accepts `max_len` /
  `head_max_len` (capped by `LAYA_MAX_TOKEN_BUDGET`, default 8192) and an optional
  bearer check (`LAYA_API_KEY`). Limits: state ≤ 50,000 chars, ≤ 64 questions, 2 MiB body,
  503 when `LAYA_MAX_CONCURRENT` (16) requests are in flight (refused, never queued);
  one forward pass at a time (single worker). **No batch endpoint.**
- **Context window is small**: 512 tokens (English), 1024 (others, up to 8192 with
  `max_len`, accuracy degrades past ~4,000 tokens). The question's instructions +
  criteria share `head_max_len` (192/256 tokens); the state gets the rest.

## 2. How it differs from Jev (and why it needs its own adapter)

| | Jev (`JevJudge`) | Laya (`LayaJudge`) |
|---|---|---|
| Hosting | TypeSafe cloud / OpenRouter, API key | self-hosted, no key (optional bearer) |
| Input | one structured state holding **all** candidates, one `noul` question per candidate id (up to 32K tokens) | state ≤ ~320-768 tokens → **one request per candidate**, state = query + that candidate |
| Calls per query | 1 | N (`top_n`), bounded concurrency |
| `noul` criteria | fine without | **mandatory** — without them the English checkpoint answers "no" to everything; option labels can dominate the answer (issue #156) |
| Confidence | `(n·p−1)/(n−1)` | entropy-based; only `answer_confidence` is comparable — irrelevant here (we consume P(true) directly) |

Design consequences (all implemented in `crates/codegraph-decision-engine/src/laya.rs`):
per-candidate requests with a semaphore; explicit criteria describing the *state*; a whole-call
deadline that returns what finished (missing ⇒ "no judgment", never 0, per the trait);
503 retry; 401 surfaced as an actionable error; `laya.top_n` defaults to 20 (each candidate
costs a forward pass) instead of Jev's 50.

## 3. Integration (no change to Jev behavior)

- `[retrieval.laya]` (`LayaConfig`) beside `[retrieval.jev]`, off by default. Jev wins if
  both are enabled (warned at startup).
- `RetrievalConfig::judge_provider/judge_top_n/judge_weight` make the pipeline provider-
  agnostic; `apply_jev` and the recall-widening gate read those. `watch_status` /
  `/api/status` gain `judge_provider` (`"jev"|"laya"|null`); `jev_active` keeps its name
  and now means "any judge active".
- `build_jev_judge` in `src/server.rs` selects the adapter. Failure modes are unchanged:
  the judge never fails a query, and any error falls back to lexical+graph ranking.

## 4. Validation protocol (fixed before measuring)

Harness: `offline_eval` with `CODEGRAPH_JUDGE=laya` / `LAYA_PROBE`, against a real
`laya-serve` (venv install, CPU). Corpora: lexical, keycloak (erpnext if time allows).

1. **Mocked tests** (in-process stand-in for `laya-serve`): wire shape, id↔answer mapping,
   bounded concurrency, partial failure, deadline, bearer, 503 retry, unreachable server.
2. **Judge quality, independent of blending** (one Laya pass over the judge-off top-30):
   how does Laya's P(true) rank the expected symbol within the pool (mean percentile /
   top-1 rate), vs the base ranking?
3. **End-to-end**: judge-off vs Laya-on through the real pipeline (widened recall,
   truncated snippets), weights swept over a small grid chosen *now*: {0.05, 0.15, 0.30,
   0.60}, `top_n` ∈ {20}. Metric: normalized and raw MRR, pass@5.
4. **Decision rule**: the shipped default `weight` is the smallest grid value inside the
   best plateau (same precedent as earlier plans). Laya stays **off by default** regardless
   of outcome (it needs a running server); if no weight is ≥ +0.01 normalized MRR on every
   measured corpus with no corpus below −0.01, the docs say so plainly and the default weight
   is the conservative 0.05.
5. **Latency**: per-candidate and per-query wall time on this machine (CPU), and the
   `concurrency` / `top_n` implications. State plainly that CPU latency is far from the
   project's claimed GPU numbers.

## 5. Results

See `bench-reports/plan25-laya-adapter-2026-09-29.md`. Summary: Laya is a near-tie-breaker (AUC 0.54-0.76 on hard negatives), not a standalone ranker. No weight met the decision rule, so the default is `weight = 0.05` and Laya is off by default; the best observed cell (multilingual, 0.30, validated end to end: +0.010 / +0.045 / +0.007 normalized MRR, no corpus worse) is a documented recommendation. Cost: +2.5-3.0 s per query (English) / +1.4-1.5 s (multilingual) for 20 candidates on an Apple GPU.
