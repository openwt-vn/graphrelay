# Draft: "We built a code graph for AI agents. It ties grep. Here is the data."

Status: draft, not published. The structural result is filled in (a tie); review the wording before publishing.

AI coding agents find code with grep and by reading files. GraphRelay is a local tool that indexes a repository into a symbol and call graph and lets an agent ask it for relevant code, a symbol's callers and callees, and the blast radius of a change. One binary, no cloud, no API key.

We tested whether that makes agents better. On 30 hard questions about a ~1M-line codebase (ERPNext), with two models, the agent with GraphRelay scored 80.2% vs 77.6% (GLM 5.3 Flash) and 70.6% vs 67.2% (Sonnet 5.5) against the same agent with grep alone. Both gaps are inside the noise. It also cost more per question (+36% for Sonnet, +20% for GLM). Competing tools tied or lost too. Three repeats on the v0.1.0 code gave 69.5% against a 70.6% baseline, and one run of the downloaded binary 71.5%: still a tie.

We then asked the narrower question people assume it wins: callers and impact. The existing questions show no lift there. So we built 12 new ones with answer keys from Python's `ast`, independent of our own tool, and wrote down the decision rule before running anything. The tool's call-graph query returned every key caller (recall 1.00; precision 0.91 on ERPNext and 0.755 on Gitea; the extras were real callers in test and patch files the key excludes). Agent result: a tie. On ERPNext and Gitea, 12 tasks x 3 reps x 2 agents, the baseline already scored 96% to 99% and our difference was +0.4 to +4.4 points (one of them mostly from a single baseline failure) with intervals that touch zero, so the rule was not met. With the tool the agents used fewer tool calls and spent somewhat less in those runs, the opposite of the 30-question runs.

What we think is true so far: the graph is accurate on static calls; whether that makes agents answer better is not supported by our tests. What you get regardless: your code never leaves your machine, and there is a web UI to explore the graph yourself.

Everything, including the questions, keys and scripts, is in the repo.
