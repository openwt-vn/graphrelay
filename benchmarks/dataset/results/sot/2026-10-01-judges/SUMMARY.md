# SOT benchmark summary

## claude-sonnet-5 — 3 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| codegraph-jev | 72.6%, 67.0%, 72.6% | **70.7%** | 0/18 | 10.4 | $4.394 |
| codegraph-laya | 73.3%, 69.1%, 66.3% | **69.6%** | 0/18 | 10.6 | $4.515 |

| Question | codegraph-jev mean (min-max) | codegraph-laya mean (min-max) |
|---|---|---|
| Q06 | 85.4% (81-94) | 85.4% (81-94) |
| Q16 | 66.7% (67-67) | 63.9% (58-67) |
| Q26 | 41.7% (38-50) | 29.2% (25-38) |
| Q21 | 72.2% (58-83) | 75.0% (75-75) |
| Q11 | 83.3% (67-92) | 86.1% (75-100) |
| Q01 | 75.0% (75-75) | 77.8% (67-83) |

Zero-coverage answers: codegraph-jev 0/18, codegraph-laya 0/18. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.

## openrouter-glm-5-3-flash — 3 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| codegraph-jev | 74.7%, 80.2%, 78.8% | **77.9%** | 0/18 | 12.9 | $0.083 |
| codegraph-laya | 75.3%, 75.3%, 78.8% | **76.5%** | 0/18 | 12.7 | $0.089 |

| Question | codegraph-jev mean (min-max) | codegraph-laya mean (min-max) |
|---|---|---|
| Q06 | 81.2% (81-81) | 89.6% (81-94) |
| Q16 | 83.3% (75-92) | 75.0% (75-75) |
| Q26 | 33.3% (25-50) | 33.3% (25-38) |
| Q21 | 77.8% (67-92) | 80.6% (75-83) |
| Q11 | 97.2% (92-100) | 97.2% (92-100) |
| Q01 | 94.4% (92-100) | 83.3% (83-83) |

Zero-coverage answers: codegraph-jev 0/18, codegraph-laya 0/18. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
