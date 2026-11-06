# SOT benchmark summary

## claude-sonnet-5 — 1 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 55.9% | **55.9%** | 0/6 | 10.2 | $1.230 |
| codegraph | 73.3% | **73.3%** | 0/6 | 9.5 | $1.516 |
| graphify | 67.0% | **67.0%** | 0/6 | 15.5 | $1.585 |
| codegraph-colby | 58.3% | **58.3%** | 0/6 | 7.8 | $1.433 |

| Question | baseline mean (min-max) | codegraph mean (min-max) | graphify mean (min-max) | codegraph-colby mean (min-max) |
|---|---|---|---|---|
| Q26 | 37.5% (38-38) | 50.0% (50-50) | 37.5% (38-38) | 12.5% (12-12) |
| Q21 | 0.0% (0-0) | 75.0% (75-75) | 41.7% (42-42) | 50.0% (50-50) |
| Q11 | 83.3% (83-83) | 83.3% (83-83) | 91.7% (92-92) | 75.0% (75-75) |
| Q06 | 81.2% (81-81) | 81.2% (81-81) | 81.2% (81-81) | 62.5% (62-62) |
| Q01 | 75.0% (75-75) | 83.3% (83-83) | 83.3% (83-83) | 83.3% (83-83) |
| Q16 | 58.3% (58-58) | 66.7% (67-67) | 66.7% (67-67) | 66.7% (67-67) |

- codegraph − baseline over 6 questions: **+17.4 pts**, 95% bootstrap CI [+2.8, +41.0] (resamples questions, so the interval is wide).
- graphify − baseline over 6 questions: **+11.1 pts**, 95% bootstrap CI [+2.8, +23.6] (resamples questions, so the interval is wide).
- codegraph-colby − baseline over 6 questions: **+2.4 pts**, 95% bootstrap CI [-15.3, +23.6] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 1/6, codegraph 0/6, graphify 0/6, codegraph-colby 0/6. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.

## openrouter-glm-5-3-flash — 1 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 76.7% | **76.7%** | 0/6 | 16.3 | $0.038 |
| codegraph | 77.8% | **77.8%** | 0/6 | 12.0 | $0.042 |
| graphify | 76.4% | **76.4%** | 0/6 | 15.5 | $0.041 |
| codegraph-colby | 74.0% | **74.0%** | 0/6 | 8.3 | $0.041 |

| Question | baseline mean (min-max) | codegraph mean (min-max) | graphify mean (min-max) | codegraph-colby mean (min-max) |
|---|---|---|---|---|
| Q26 | 25.0% (25-25) | 50.0% (50-50) | 12.5% (12-12) | 37.5% (38-38) |
| Q21 | 75.0% (75-75) | 75.0% (75-75) | 83.3% (83-83) | 75.0% (75-75) |
| Q11 | 91.7% (92-92) | 91.7% (92-92) | 100.0% (100-100) | 83.3% (83-83) |
| Q06 | 93.8% (94-94) | 75.0% (75-75) | 87.5% (88-88) | 81.2% (81-81) |
| Q01 | 83.3% (83-83) | 83.3% (83-83) | 91.7% (92-92) | 100.0% (100-100) |
| Q16 | 91.7% (92-92) | 91.7% (92-92) | 83.3% (83-83) | 66.7% (67-67) |

- codegraph − baseline over 6 questions: **+1.0 pts**, 95% bootstrap CI [-9.4, +12.5] (resamples questions, so the interval is wide).
- graphify − baseline over 6 questions: **-0.3 pts**, 95% bootstrap CI [-7.3, +5.9] (resamples questions, so the interval is wide).
- codegraph-colby − baseline over 6 questions: **-2.8 pts**, 95% bootstrap CI [-14.6, +8.3] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 0/6, codegraph 0/6, graphify 0/6, codegraph-colby 0/6. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
