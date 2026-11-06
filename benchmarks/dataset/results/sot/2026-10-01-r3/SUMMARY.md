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

## openrouter-glm-5-3-flash — 3 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 83.7%, 73.6%, 76.0% | **77.8%** | 0/18 | 17.4 | $0.087 |
| codegraph | 68.8%, 78.8%, 77.4% | **75.0%** | 0/18 | 13.1 | $0.076 |
| graphify | 67.0%, 72.2%, 81.2% | **73.5%** | 0/18 | 16.9 | $0.087 |
| codegraph-colby | 82.3%, 76.4%, 76.7% | **78.5%** | 0/18 | 8.8 | $0.076 |

| Question | baseline mean (min-max) | codegraph mean (min-max) | graphify mean (min-max) | codegraph-colby mean (min-max) |
|---|---|---|---|---|
| Q26 | 50.0% (50-50) | 41.7% (12-62) | 33.3% (12-50) | 54.2% (12-75) |
| Q21 | 77.8% (58-92) | 83.3% (75-92) | 66.7% (58-75) | 83.3% (75-92) |
| Q11 | 88.9% (75-100) | 91.7% (83-100) | 86.1% (67-100) | 91.7% (83-100) |
| Q16 | 83.3% (75-92) | 72.2% (67-75) | 83.3% (67-92) | 69.4% (58-83) |
| Q01 | 83.3% (83-83) | 86.1% (75-92) | 86.1% (83-92) | 88.9% (83-92) |
| Q06 | 83.3% (75-94) | 75.0% (69-81) | 85.4% (81-88) | 83.3% (75-94) |

- codegraph − baseline over 6 questions: **-2.8 pts**, 95% bootstrap CI [-7.9, +2.3] (resamples questions, so the interval is wide).
- graphify − baseline over 6 questions: **-4.3 pts**, 95% bootstrap CI [-10.3, +1.2] (resamples questions, so the interval is wide).
- codegraph-colby − baseline over 6 questions: **+0.7 pts**, 95% bootstrap CI [-5.3, +4.9] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 0/18, codegraph 0/18, graphify 0/18, codegraph-colby 0/18. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
