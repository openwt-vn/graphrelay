# SOT benchmark summary

## claude-sonnet-5 — 3 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 64.9%, 64.6%, 73.3% | **67.6%** | 0/18 | 13.6 | $4.646 |
| codegraph | 63.5%, 66.3%, 74.7% | **68.2%** | 0/18 | 11.1 | $4.760 |
| graphify | 71.2%, 70.1%, 60.4% | **67.2%** | 0/18 | 14.3 | $4.376 |
| codegraph-colby | 69.4%, 68.4%, 67.7% | **68.5%** | 0/18 | 7.4 | $4.578 |

| Question | baseline mean (min-max) | codegraph mean (min-max) | graphify mean (min-max) | codegraph-colby mean (min-max) |
|---|---|---|---|---|
| Q26 | 41.7% (25-75) | 20.8% (12-38) | 45.8% (12-62) | 29.2% (12-50) |
| Q21 | 61.1% (50-75) | 69.4% (58-83) | 52.8% (42-58) | 61.1% (42-83) |
| Q11 | 88.9% (83-100) | 80.6% (75-83) | 86.1% (83-92) | 86.1% (75-100) |
| Q06 | 75.0% (62-81) | 93.8% (94-94) | 77.1% (75-81) | 87.5% (81-94) |
| Q01 | 75.0% (75-75) | 80.6% (75-83) | 80.6% (75-83) | 86.1% (83-92) |
| Q16 | 63.9% (58-67) | 63.9% (58-67) | 61.1% (58-67) | 61.1% (50-67) |

- codegraph − baseline over 6 questions: **+0.6 pts**, 95% bootstrap CI [-9.7, +10.0] (resamples questions, so the interval is wide).
- graphify − baseline over 6 questions: **-0.3 pts**, 95% bootstrap CI [-4.3, +3.4] (resamples questions, so the interval is wide).
- codegraph-colby − baseline over 6 questions: **+0.9 pts**, 95% bootstrap CI [-5.6, +7.6] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 0/18, codegraph 0/18, graphify 0/18, codegraph-colby 0/18. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
