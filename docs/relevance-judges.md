# Relevance judges (optional)

Both judges are optional, off by default, and only ever add a bonus on top of the always-on lexical+graph ranking. A failed or slow judge falls back to the base ranking and never fails a query. If both are enabled, Jev is used. `watch_status()` reports which one is active (`judge_provider`).

> The measurements below were taken on benchmark suites that have since been retired (see [benchmarks](benchmarks.md)); treat them as indicative, not as evidence the judges help your queries.

## Status (2026-09-30)

| | **Jev** | **Laya** |
|---|---|---|
| Status | ✅ implemented, verified live via OpenRouter (2026-09-25) | ✅ implemented, verified against a real `laya-serve` (2026-09-29) |
| Runs | TypeSafe cloud / OpenRouter, needs an API key | your own machine (`pip install "laya[serve]"`), no key |
| Requests | 1 call for the whole pool (`top_n = 50`) | 1 request per candidate (`top_n = 20`) |
| Measured effect | keycloak 33 → 38 of 40 (lenient scorer, pre-Plan-24 code) | normalized MRR +0.010 / +0.045 / +0.007 (lexical / keycloak / erpnext) at the best setting; default weight 0.05 gives +0.020 / 0.000 / −0.007 |
| Added latency | ~0.85 s per query (network) | +1.4 to +1.5 s (multilingual) or +2.5 to +3.0 s (English) on an Apple GPU |
| Default weight | 0.15 | 0.05 |
| Not yet done | not re-measured on the current code or with the strict MRR scorer (no API key was available) | best setting is the best of 36 tried; needs validation on your own queries |

The two rows of numbers are **not comparable**: Jev's are from the older lenient scorer and code, Laya's from the strict harness on the shipped code.


`[retrieval.jev]` turns on an optional extra signal from [TypeSafe AI's Jev](https://typesafe.ai) — a per-candidate relevance judgment layered onto the always-on lexical+graph ranking, not a replacement for it. It's aimed specifically at the class of miss the base pipeline structurally can't close (see [How retrieval works without embeddings](#how-retrieval-works-without-embeddings)): a query whose vocabulary doesn't overlap the code's at all.

```toml
[retrieval.jev]
enabled  = true
api_key  = "..."          # or GRAPHRELAY_JEV_API_KEY
# base_url = "https://api.typesafe.ai/v1/systemone"   # default: direct to TypeSafe
# model    = "jev-latest"                              # default
# top_n    = 50            # how many top-ranked candidates to send Jev (validated default)
# weight   = 0.15           # how much its judgment can move a candidate's score
# timeout_ms = 3000
```

Also reachable through OpenRouter instead of a direct TypeSafe account — OpenRouter documents `/api/v1/systemone` as schema-compatible ("for redirecting the TypeSafe SDK"), so it's just a config swap, no code difference:

```toml
[retrieval.jev]
enabled  = true
api_key  = "sk-or-v1-..."                               # your OpenRouter key
base_url = "https://openrouter.ai/api/v1/systemone"
model    = "typesafe/jev-1.13"                          # or "~typesafe/jev-latest"
```

It's off by default because graphrelay's whole story is "fully offline, zero API keys, zero dependencies" — this is the one opt-in exception, and it costs nothing when disabled. A failed or timed-out call always falls back cleanly to the base lexical+graph ranking; it never fails a query. `watch_status()`'s `jev_active` field reports whether it's actually wired up for a project.

**Verified working end-to-end and benchmarked** (2026-09-25, via OpenRouter) — confirmed against a live account: `jev_active: true`, `search_code` latency jumping from graphrelay's normal ~2-5ms to ~850-880ms (a real network round-trip), no errors. On the Keycloak suite referenced above, the shipped default (`top_n = 50`, each candidate's snippet truncated before judging — see below) moves the score from 33/40 to **38/40 (95%)**, identically across repeated runs, with the lexical suite (already 40/40 without Jev) confirmed unaffected at 40/40 with Jev on. Getting there wasn't one clean shot: an intermediate attempt (`top_n = 50` with *un*truncated snippets) measurably regressed keycloak to 34/40 before the real cause — sending 50 full-length snippets in one batched judging call is a much bigger payload than 20, straining judgment quality across the whole batch, not just the new candidates — was found and fixed.



[Laya](https://github.com/NandhaKishorM/laya) (Apache-2.0) is a small non-autoregressive encoder that answers typed yes/no questions in one forward pass. Its `laya-serve` speaks the same `/v1/systemone` protocol as Jev, so graphrelay can use it as the relevance judge with **no API key and no cloud call** — you run the server yourself:

```bash
pip install "laya[serve]"
LAYA_MODELS=english laya-serve        # or multilingual; http://127.0.0.1:8000
```

```toml
[retrieval.laya]
enabled  = true
# base_url = "http://127.0.0.1:8000/v1/systemone"   # default
# model    = ""            # "" = Laya's router chooses; or "english" / "multilingual"
# top_n    = 20            # candidates judged per query — each is one forward pass
# weight   = 0.05          # default; see the measurements below before raising it
# concurrency = 4
# timeout_ms  = 15000      # whole-call deadline; on expiry the candidates scored so far are used
# api_key  = "..."         # only if the server was started with LAYA_API_KEY
```

It is independent of `[retrieval.jev]` (if both are enabled, Jev wins). Unlike Jev — which takes the whole candidate pool in one call — Laya reads a 512-1024-token window, so graphrelay sends **one request per candidate**; a slow or failed candidate is simply left unscored, and any error falls back cleanly to the base ranking. `watch_status()` reports `judge_provider: "laya"`.

**What it is worth** (retired benchmark suite; normalized MRR change vs no judge on lexical / keycloak / erpnext): at the default weight 0.05, +0.020 / 0.000 / −0.007. Laya's probability is compressed, so it behaves as a **near-tie-breaker among the top few results**, not a general ranker — larger weights help until they start overriding a good base ranking (English checkpoint, weight 0.60: +0.069 / +0.044 / −0.010). The best setting measured was `model = "multilingual"`, `weight = 0.30`: **+0.010 / +0.045 / +0.007**, no corpus worse — but that is the best of 36 configurations tried, so validate it on your own queries before relying on it.

**It costs latency.** On an Apple GPU, judging 20 candidates adds about **+2.5-3.0 s per query** (English checkpoint) or **+1.4-1.5 s** (multilingual); on CPU expect several times that. `laya-serve` runs one forward pass at a time, so `concurrency` barely changes this. Reasonable for offline/batch use or a fast GPU; a real cost for an interactive agent loop. Off by default for that reason.

