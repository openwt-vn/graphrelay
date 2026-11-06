# SOT benchmark summary

## claude-sonnet-5 — 1 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| codegraph-jev | 72.6% | **72.6%** | 0/6 | 11.5 | $1.433 |
| codegraph-laya | 73.3% | **73.3%** | 0/6 | 9.2 | $1.438 |

| Question | codegraph-jev mean (min-max) | codegraph-laya mean (min-max) |
|---|---|---|
| Q06 | 81.2% (81-81) | 93.8% (94-94) |
| Q16 | 66.7% (67-67) | 66.7% (67-67) |
| Q26 | 37.5% (38-38) | 37.5% (38-38) |
| Q21 | 83.3% (83-83) | 75.0% (75-75) |
| Q11 | 91.7% (92-92) | 83.3% (83-83) |
| Q01 | 75.0% (75-75) | 83.3% (83-83) |

Zero-coverage answers: codegraph-jev 0/6, codegraph-laya 0/6. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
