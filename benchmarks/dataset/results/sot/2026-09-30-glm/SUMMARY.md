# SOT benchmark summary

## openrouter-glm-5-3-flash — 3 repeat(s) × 6 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 62.5%, 60.1%, 38.5% | **53.7%** | 5/18 | 24.5 | $0.167 |
| codegraph | 80.9%, 68.1%, 75.3% | **74.8%** | 0/18 | 15.0 | $0.125 |

| Question | baseline mean (min-max) | codegraph mean (min-max) | difference |
|---|---|---|---|
| Q01 | 30.6% (0-92) | 86.1% (75-92) | +55.6 |
| Q06 | 87.5% (81-94) | 83.3% (81-88) | -4.2 |
| Q11 | 77.8% (67-92) | 100.0% (100-100) | +22.2 |
| Q16 | 75.0% (67-83) | 77.8% (67-92) | +2.8 |
| Q21 | 22.2% (0-67) | 72.2% (67-83) | +50.0 |
| Q26 | 29.2% (0-62) | 29.2% (12-38) | +0.0 |

Mean paired difference (codegraph − baseline) over the 6 questions: **+21.1 pts**, 95% bootstrap CI [+2.8, +40.0] — resampling *questions*, so with 6 questions the interval is wide.
Zero-coverage answers: baseline 5/18, codegraph 0/18. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
