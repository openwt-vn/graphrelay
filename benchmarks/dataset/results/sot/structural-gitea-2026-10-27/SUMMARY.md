# SOT benchmark summary

## claude-sonnet-5 — 3 repeat(s) × 12 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 98.8%, 98.3%, 98.8% | **98.6%** | 0/36 | 8.4 | $2.789 |
| graphrelay | 100.0%, 99.4%, 100.0% | **99.8%** | 0/36 | 3.9 | $1.596 |

| Question | baseline mean (min-max) | graphrelay mean (min-max) |
|---|---|---|
| S06 | 100.0% (100-100) | 100.0% (100-100) |
| S11 | 97.9% (94-100) | 100.0% (100-100) |
| S03 | 100.0% (100-100) | 100.0% (100-100) |
| S05 | 100.0% (100-100) | 100.0% (100-100) |
| S08 | 100.0% (100-100) | 100.0% (100-100) |
| S01 | 100.0% (100-100) | 100.0% (100-100) |
| S09 | 100.0% (100-100) | 100.0% (100-100) |
| S04 | 100.0% (100-100) | 100.0% (100-100) |
| S10 | 85.7% (86-86) | 97.6% (93-100) |
| S07 | 100.0% (100-100) | 100.0% (100-100) |
| S12 | 100.0% (100-100) | 100.0% (100-100) |
| S02 | 100.0% (100-100) | 100.0% (100-100) |

- graphrelay − baseline over 12 questions: **+1.2 pts**, 95% bootstrap CI [+0.0, +3.2] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 0/36, graphrelay 0/36. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.

## openrouter-glm-5-3-flash — 3 repeat(s) × 12 questions

| Arm | repeat means | pooled mean | turn-cap / empty answers | avg tool calls | cost |
|---|---|---|---|---|---|
| baseline | 97.5%, 98.8%, 100.0% | **98.8%** | 0/36 | 15.0 | $0.090 |
| graphrelay | 100.0%, 98.8%, 99.4% | **99.4%** | 0/36 | 10.4 | $0.067 |

| Question | baseline mean (min-max) | graphrelay mean (min-max) |
|---|---|---|
| S06 | 100.0% (100-100) | 100.0% (100-100) |
| S11 | 100.0% (100-100) | 100.0% (100-100) |
| S03 | 100.0% (100-100) | 100.0% (100-100) |
| S05 | 100.0% (100-100) | 100.0% (100-100) |
| S08 | 100.0% (100-100) | 100.0% (100-100) |
| S01 | 100.0% (100-100) | 100.0% (100-100) |
| S09 | 97.6% (93-100) | 100.0% (100-100) |
| S04 | 100.0% (100-100) | 100.0% (100-100) |
| S10 | 95.2% (93-100) | 92.9% (86-100) |
| S07 | 92.4% (77-100) | 100.0% (100-100) |
| S12 | 100.0% (100-100) | 100.0% (100-100) |
| S02 | 100.0% (100-100) | 100.0% (100-100) |

- graphrelay − baseline over 12 questions: **+0.6 pts**, 95% bootstrap CI [-0.4, +2.1] (resamples questions, so the interval is wide).

Zero-coverage answers: baseline 0/36, graphrelay 0/36. An answer that hits the turn cap is scored 0% by construction; when one arm hits it far more often, much of the coverage gap is 'reaches a final answer within the turn budget', not 'answers better'.
