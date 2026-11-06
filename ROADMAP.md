# Roadmap

Status of this file: a working draft written 2026-10-26. Items marked **decision** belong to the project owner; nothing here is committed to until they say so. Evidence behind every claim is in [docs/benchmarks.md](docs/benchmarks.md).

## Where we stand

- **Built:** one local Rust binary (`gr`), 31 languages, 4 MCP tools, REST API and a web UI; indexing a ~1M LOC repo takes under a minute.
- **Not shown:** that an agent answers better with it. Latest 30-question runs are ties with the no-tool baseline on both models, and it costs more per question (+36% Sonnet, +20% GLM).
- **Not true of the world yet:** the project is brand new (v0.1.0 is the first public release) with no stars and no outside users.

So growth work comes in this order: make it installable, find out whether it helps anyone, then tell people.

## Positioning (hypotheses to test, not claims)

What the evidence supports today: your code stays on your machine, no account or API key, one binary, an accurate static call graph (recall 1.00 on the 12-question `impact_recall` checks; precision 0.91 on ERPNext and 0.755 on Gitea, where every extra is a test caller the key excludes), and a human-facing graph UI. What it does not support: "better answers", "cheaper", "better at structural questions".

Candidate angles, each needing its own evidence:
1. **Private by construction.** For teams that cannot send code to a hosted indexer. True today; strength depends on whether those teams exist in numbers. Test: talk to 5-10 of them.
2. **A graph you can see.** The web UI (search, interactive call graph, inspector with callers/callees) helps humans onboard to and review unfamiliar code, independent of any agent benchmark. Test: do developers use it unprompted?
3. ~~**Deterministic impact answers.**~~ Tested on ERPNext and Gitea (2026-10-29): no coverage gain, intervals touch 0 on both agents, baseline already 96% to 99%. Dropped as a claim. A possible weaker observation (fewer tool calls, lower cost on these tasks) is not consistent with the earlier 30-question cost results (Sonnet 5.5, a different model from the Sonnet 5 used here).

## Steps

### Now: remove blockers
- [ ] **decision:** make the repository public (or decide it stays private and says so).
- [ ] Cut a release under the GraphRelay name (`graphrelay-<tag>-<target>.tar.gz`) and add the missing `x86_64-apple-darwin` target the installer expects. Until then the README installer cannot work.
- [ ] Enable GitHub private vulnerability reporting (SECURITY.md relies on it).

### Next: find out if it helps
- [x] Run the preregistered structural slice (done 2026-10-29, rule not met, see `docs/benchmarks.md`): `benchmarks/dataset/run-sot.sh --structural --repeats 3` ([preregistration](benchmarks/dataset/tasks/STRUCTURAL_PREREGISTRATION.md)). Costs real API money (Sonnet about $1.6 per arm-run per the harness notes; GLM pennies). Record under `results/sot/`.
- [ ] Replicate on more repositories before believing anything from ERPNext alone. Proposed targets, with HEAD commits as of 2026-10-26 (pin one per repo before any run): `keycloak/keycloak` 7e07aeecd7281e6cd90a0dcc46957d74332dd553 (Java), `django/django` 36920bb4766679f5b32445797dcc5d32b8ff5ad2 (Python), `laravel/framework` 5d7797528723107cd4f495326838a7d7d67fbb20 (PHP). Each needs its own question set and answer keys built the same independent way; there are no setup scripts yet.
- [ ] Report tokens, tool calls and cost per question as first-class results, not just coverage.
- [ ] Beachhead hypothesis, to be validated or dropped: large legacy or enterprise codebases (Java/PHP/Python monoliths) used with Claude Code or Cursor, where grep gets slow or noisy and privacy matters. Nothing measured supports this yet.

### Then: reach people (only after the blockers are gone)
- [ ] Homebrew tap: template at [packaging/homebrew/graphrelay.rb](packaging/homebrew/graphrelay.rb); it needs a real release and sha256 values.
- [ ] Submit to MCP server directories after reading each directory's current submission format (not drafted here, to avoid guessing).
- [ ] Publish the [launch post draft](docs/launch-post.md) once the structural result is in, with that result in it either way.
- [ ] Short demo video: source is in `internal/video/` (Remotion), renders with `npm run render:v2` (the current intro; `npm run render` is the older v1).

### Sustainability (decision)
No model is chosen. Options, none assumed:
- Stay a free MIT hobby project; cheapest, no promises to anyone.
- Free core plus paid team features (shared hosted index, SSO). Requires a deployment story the project does not have and contradicts "nothing leaves your machine" unless self-hosted.
- Sponsorship or grants. Needs a public repo and visible users first.
- Services around the tool (indexing for legacy stacks). Only if the beachhead hypothesis holds.

## What would change this plan
- Structural slice supported on both agents: lead the README with impact analysis and invest in `trace_impact` depth/recall on dynamic dispatch.
- Structural slice not supported (**this is what happened on ERPNext and Gitea**): the retrieval thesis is a tie on every measure we have; the project's remaining value is privacy and the UI, and effort should move there.
