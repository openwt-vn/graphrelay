# SOT benchmark summary

## claude-sonnet-5 — 3 repeat(s) × 12 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 98.6%, 99.0%, 99.7% | **99.1%** | 0/36 | 8.7 | $3.953 |
| graphrelay | 99.3%, 99.3%, 100.0% | **99.5%** | 0/36 | 7.2 | $3.506 |

| Question | baseline mean (min-max) | graphrelay mean (min-max) |
|---|---|---|
| S06 | 100.0% (100-100) | 100.0% (100-100) |
| S11 | 100.0% (100-100) | 100.0% (100-100) |
| S03 | 100.0% (100-100) | 100.0% (100-100) |
| S05 | 100.0% (100-100) | 100.0% (100-100) |
| S08 | 100.0% (100-100) | 100.0% (100-100) |
| S01 | 100.0% (100-100) | 100.0% (100-100) |
| S09 | 94.4% (83-100) | 94.4% (92-100) |
| S04 | 100.0% (100-100) | 100.0% (100-100) |
| S10 | 100.0% (100-100) | 100.0% (100-100) |
| S07 | 94.9% (88-100) | 100.0% (100-100) |
| S12 | 100.0% (100-100) | 100.0% (100-100) |
| S02 | 100.0% (100-100) | 100.0% (100-100) |

- graphrelay − baseline over 12 questions: **+0.4 pts**, 95% bootstrap CI [+0.0, +1.3] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 0/36, graphrelay 0/36. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
