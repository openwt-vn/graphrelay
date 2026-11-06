# SOT benchmark summary

## openrouter-glm-5-3-flash — 3 repeat(s) × 12 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 90.1%, 99.1%, 97.7% | **95.6%** | 1/36 | 16.0 | $0.108 |
| graphrelay | 100.0%, 100.0%, 100.0% | **100.0%** | 0/36 | 9.8 | $0.080 |

| Question | baseline mean (min-max) | graphrelay mean (min-max) |
|---|---|---|
| S06 | 100.0% (100-100) | 100.0% (100-100) |
| S11 | 100.0% (100-100) | 100.0% (100-100) |
| S03 | 100.0% (100-100) | 100.0% (100-100) |
| S05 | 100.0% (100-100) | 100.0% (100-100) |
| S08 | 100.0% (100-100) | 100.0% (100-100) |
| S01 | 100.0% (100-100) | 100.0% (100-100) |
| S09 | 100.0% (100-100) | 100.0% (100-100) |
| S04 | 64.3% (0-100) | 100.0% (100-100) |
| S10 | 100.0% (100-100) | 100.0% (100-100) |
| S07 | 89.7% (81-96) | 100.0% (100-100) |
| S12 | 93.3% (80-100) | 100.0% (100-100) |
| S02 | 100.0% (100-100) | 100.0% (100-100) |

- graphrelay − baseline over 12 questions: **+4.4 pts**, 95% bootstrap CI [+0.0, +10.9] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 1/36, graphrelay 0/36. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
