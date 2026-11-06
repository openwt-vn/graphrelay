# SOT benchmark summary

## claude-sonnet-5 — 2 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| codegraph-jev | 67.0%, 72.6% | **69.8%** | 0/12 | 9.9 | $2.961 |
| codegraph-laya | 69.1%, 66.3% | **67.7%** | 0/12 | 11.2 | $3.077 |

| Question | codegraph-jev mean (min-max) | codegraph-laya mean (min-max) |
|---|---|---|
| Q06 | 87.5% (81-94) | 81.2% (81-81) |
| Q16 | 66.7% (67-67) | 62.5% (58-67) |
| Q26 | 43.8% (38-50) | 25.0% (25-25) |
| Q21 | 66.7% (58-75) | 75.0% (75-75) |
| Q11 | 79.2% (67-92) | 87.5% (75-100) |
| Q01 | 75.0% (75-75) | 75.0% (67-83) |

Zero-coverage answers: codegraph-jev 0/12, codegraph-laya 0/12. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.

## openrouter-glm-5-3-flash — 2 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| codegraph-jev | 80.2%, 78.8% | **79.5%** | 0/12 | 12.8 | $0.055 |
| codegraph-laya | 75.3%, 78.8% | **77.1%** | 0/12 | 12.4 | $0.058 |

| Question | codegraph-jev mean (min-max) | codegraph-laya mean (min-max) |
|---|---|---|
| Q06 | 81.2% (81-81) | 93.8% (94-94) |
| Q16 | 79.2% (75-83) | 75.0% (75-75) |
| Q26 | 37.5% (25-50) | 31.2% (25-38) |
| Q21 | 83.3% (75-92) | 79.2% (75-83) |
| Q11 | 100.0% (100-100) | 100.0% (100-100) |
| Q01 | 95.8% (92-100) | 83.3% (83-83) |

Zero-coverage answers: codegraph-jev 0/12, codegraph-laya 0/12. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
