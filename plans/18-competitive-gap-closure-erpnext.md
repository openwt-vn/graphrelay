# Plan 18 — ERPNext Benchmark Competitive Gap Closure (vs Graphify)

> **Note (2026-09-30):** `bench-reports/`, the retrieval-only suites (`benchmarks/llm_tasks_*.yaml`), the old benchmark scripts and all pre-2026-09-30 benchmark results were removed; the one benchmark that counts is `benchmarks/dataset/` (`run-sot.sh`, see `CLAUDE.md` "Benchmarking"). File names below that no longer exist are recoverable from git history at commit `5d413ea`. Numbers below were measured on the retired suites: history, not evidence.

*Continues the "Plan N" numbering used by checklist.md's Plan 14/15 ("Competitive
Hardening", "Competitive Hardening v2" — both against Tool A / graphify, neither
had a dedicated `plans/*.md` file). This one gets a file because the audit behind
it is large enough to be worth reading on its own before touching code.*

## Motivation

The `claude-sonnet-5-fixed` 3-arm ERPNext run (30 questions, all 3 arms on one
consistent all-25-turn re-run, post the `get_file_context` relative-path fix)
put codegraph at **64.2%** mean coverage vs graphify's **64.7%** and
code-review-graph's **65.2%** — a virtual tie, not the clear lead an earlier
(noisier, single-run) snapshot suggested. The user asked for a real audit of
*why*, per question, not just the aggregate number, and a plan to beat graphify
specifically. This document is that audit plus the resulting fix plan.

**Read `bench-reports/` and checklist.md's Plan 16/17 sections first if you
haven't** — this plan assumes familiarity with the established ~13pp
run-to-run noise floor on this harness (confirmed twice now: 74.8%→70.2%→64.2%
across three runs of the *same* codegraph binary/index) and the project's
evidentiary bar (a single run's score delta is not proof; mechanistic evidence
— error counts, tool-call histograms, source-level confirmation — is stronger
and should be sought before trusting an aggregate number).

## Methodology

Per-question, per-fact judge verdicts (`grades.json`'s `verdicts[].reasoning`,
written by the blind claude-sonnet-5 judge) were cross-referenced against each
question's actual tool-call transcript (`results/.../transcripts/codegraph-Q*.json`)
for every one of the 13 questions where codegraph scored below graphify.
Three questions per named finding below were verified against **primary
source** — read the actual erpnext file, or queried the live MCP daemon — not
just the transcript text, matching this project's established "verify before
trusting a self-reported cause" bar.

## Current standing (this run, `claude-sonnet-5-fixed`, n=30)

| Group | codegraph | graphify | Δ |
|---|---|---|---|
| A — cross-module synthesis | 60.7% | 68.6% | **−7.9** |
| B — caller/callee trace + impact | 69.8% | 79.2% | **−9.4** |
| C — semantic gap | 63.3% | 80.0% | **−16.7** |
| D — numeric/logic edge cases | 56.9% | 37.4% | **+19.5** |
| E — concurrency & DB semantics | 73.6% | 74.0% | −0.4 |
| F — trap / hallucination resistance | 61.0% | 49.0% | **+12.0** |

The paradox worth sitting with: codegraph **wins** decisively on D and F —
groups that reward grounded, conservative, don't-hallucinate answers, which is
consistent with the graph-fusion thesis working as intended — but **loses**
worst on B and C, the groups that should be codegraph's home turf (caller/callee
tracing is `trace_impact`'s literal job; semantic-gap questions are what real
embeddings + graph expansion are *for*). The findings below explain why B and C
are underperforming, and none of the fixes touch D/F's mechanism, so the win
there should hold.

## Root cause findings, ranked by evidence strength × fix leverage

### Finding 1 — `trace_impact` and `find_implementations` are described as pre-edit tools, so a Q&A agent never reaches for them

**Evidence.** Across all 30 codegraph transcripts: `trace_impact` called in
**3/30** questions, `find_implementations` in **1/30**, `summarize_module` in
**0/30** — against `Grep` (151 calls) and `Read` (141 calls) totally
dominating (86 MCP-tool calls vs. 292 generic-fallback calls, project-wide).
Group B ("caller/callee trace + impact analysis") is the *namesake* use case
for `trace_impact`, yet it's one of the two worst-losing groups.

Root cause, read directly from `crates/codegraph-mcp/src/server.rs`'s tool
schemas:
- `trace_impact`: *"Find all symbols that depend on a given symbol.
  **Understand blast radius before changing a function.**"*
- `find_implementations`: *"...Essential for understanding code patterns
  **before adding new implementations**."*

Both are framed entirely around a pre-edit/refactor workflow. Every benchmark
question is read-only ("trace how X works", "what calls Y") — an agent reading
either description would reasonably conclude neither tool applies, even when
the question is *exactly* "what calls this" or "what implements this
interface."

**A `trace_impact`-solves-Q24 claim was checked and retracted.** An earlier
draft of this section claimed `trace_impact` on the shared savepoint wrapper
would have exhaustively enumerated Q24's call sites. Verified against the
live index before shipping this plan: all of Q24's savepoint sites call
`frappe.db.savepoint(...)` — a method on the **Frappe framework itself**,
which is not checked out or indexed by this benchmark's setup (`repos/`
contains only `erpnext/`; `get_file_context` on a `frappe/` path returns
"file not indexed"). There is no indexed symbol for `trace_impact` to target,
so it could not have closed this specific gap. Q24 (one `find_relevant_code`
call, then 8 raw `Grep` calls, `trace_impact` never called) is real evidence
for **Finding 2** (single-shot abandonment on a many-site question) — multiple
differently-angled `find_relevant_code` queries, one per distinct flow
(subcontracting order, reposting job, subscription update, CRM linking — the
question's own multiple sub-parts), would have been the right lever, not
`trace_impact`. It is **not** independent evidence for Finding 1; Finding 1
rests on the usage-rate correlation (3/30, 1/30) and the description text
itself, not on this example.

**Fix.** Reword both descriptions to be read/explain-neutral, mirroring the
successful pattern from the Plan 17 `find_relevant_code` description fix
(which measurably changed real tool-call mix on Q23 — see checklist.md). E.g.:

> `trace_impact`: "Find every symbol that calls or otherwise depends on a
> given symbol — the graph-derived, exhaustive answer to 'what calls this' /
> 'what would this change affect' / 'trace every place X happens'. Use this
> instead of grepping for a name when you need completeness, not just a
> plausible sample."

> `find_implementations`: "Find every class/struct/module that implements or
> inherits from a given interface, trait, protocol, or base class — useful for
> both understanding existing patterns and finding every concrete case of an
> abstraction."

**Risk:** near-zero. Pure additive text change, same class of fix as the
already-shipped Plan 17 description changes, no retrieval/scoring code
touched. **Files:** `crates/codegraph-mcp/src/server.rs` only.

### Finding 2 — single-shot retrieval abandonment on compound/multi-faceted questions; Plan 17's `concentration_note` doesn't catch this shape

**Evidence.** Of the 13 questions codegraph lost, the 5 worst (Q10 −42.8, Q21
−41.6, Q24 −35.7, Q11 −33.4, Q12 −33.3) all show the same shape: 1-3
`find_relevant_code` calls, then the agent abandons MCP tools for 6-15 raw
Grep/Read calls. This is the exact failure mode Plan 17's `concentration_note`
was built to catch and redirect — but checking whether the note actually fired
(`"note"` field present in the tool result) shows it fired for only **1 of
these 5** (Q10), and even there the agent didn't act on it (still went on to
6 Grep + 8 Read calls). It never fired for Q21/Q24/Q11 at all, because
`concentration_note`'s heuristic (≥80% of results sharing an ancestor
directory) only detects one specific failure shape — a query that accidentally
anchors to one module — not "the top-8 results collectively don't cover
enough of a multi-part question," which is a different problem (a coverage
*breadth* gap, not a directory *concentration* gap) and can span many
directories while still being incomplete.

Plan 17 already flagged its own scope limit here: *"The deeper fix — actual
query decomposition ... is a real pipeline change, not attempted here."* This
finding is direct evidence that gap is now costing real benchmark points, not
just a theoretical concern.

**A co-cause, found by second-opinion review, that changes how much to expect
from a description-only fix.** `harness/run.py`'s system prompt (identical
across all 3 tool arms — a fairness-preserving, not codegraph-specific,
detail) says: *"Before your first Grep or Read call, call at least one
`mcp__{arm}__*` tool relevant to the question. Only reach for Grep/Read
afterward..."* — literally licensing exactly **one** MCP call before Grep
becomes fair game. This is a stronger, more direct explanation for the
1-3-calls-then-Grep pattern than tool-description wording alone, and it
applies identically to graphify/code-review-graph too (so it's not why
codegraph specifically loses — but it does mean an in-tool nudge is fighting
an explicit competing instruction, and should be expected to have a smaller
effect than the Plan 17 Q23 precedent it's modeled on, not an equal one).
Changing the harness prompt itself would be a benchmark-methodology change
needing the same all-arms-identically treatment as the `MAX_TURNS` raise
(checklist.md Plan 17) — out of scope for "improve codegraph's own tools",
noted here as a separate lever if Fix 2 underperforms.

**Fix (two tiers, cheap first).**

1. **Cheap, additive, do first:** strengthen `find_relevant_code`'s
   description to explicitly state the pattern seen in Q10/Q21/Q24/Q11 — that
   getting a `note`, or getting results that only address part of a multi-part
   question, is itself the signal to call `find_relevant_code` again with a
   *different* (not narrower-in-the-same-directory, but genuinely different)
   intent, once per distinct aspect, before reaching for Grep. This is the
   same lever that already worked once (Q23's turn-cap fix, checklist.md Plan
   17 follow-up: `Grep` 16→8, `Read` 9→2 from a description change alone) —
   though per the co-cause above, expect a smaller effect here, since that
   precedent didn't have to fight an explicit "only one MCP call is required"
   instruction the way this one does.
   **Files:** `crates/codegraph-mcp/src/server.rs`. **Risk:** near-zero.

2. **Real pipeline fix, bigger, do second/separately:** a lightweight
   heuristic coverage check in `find_relevant_code`'s handler — after getting
   results, check whether the query text plausibly contains multiple distinct
   asks (e.g. splitting on "and", "but", multiple "?" clauses, or just always
   emitting a softer version of the note whenever `final_n` is small relative
   to `top_k`'s spread of distinct file-groups) and surface a note in more
   cases than pure directory-concentration. **This needs its own scoped
   sub-plan and the full lexical+keycloak+erpnext regression cycle before
   shipping** — per this project's own established bar (`checklist.md`'s
   keycloak section: an ungated scoring-adjacent change previously caused a
   real regression). Do not ship without it. **Risk: moderate** — touches a
   tool response agents act on, needs the same discipline as the Plan 16
   `TEST_FILE_PENALTY` work (validate on ≥2 independent corpora before
   shipping).

### Finding 3 — JS/TS object-literal method properties are never extracted as symbols (real parser gap, not just an agent-behavior gap)

**Evidence, verified against source and against the live daemon (correcting
an earlier, broader-than-true version of this finding — see below).** Q12
(−33.3pp) requires naming client-side permission-check logic in `item.js`.
The check itself (`frappe.model.can_read("Item Price")`) was confirmed
present at `item.js:832` by direct `grep`, and reading the surrounding source
shows it lives inside `render_item_prices: function (frm) { ... }` — a
**colon-form** object-literal property whose value is a `function`
expression, at `item.js:826`.

`get_file_context` on `item.js` (queried live against the real daemon)
returns 16 symbols. Checking their line spans against 832: `render_item_prices`
is genuinely absent — no returned symbol's span covers it. **But** the
returned list already includes several object-literal methods —
`toggle_has_serial_batch_fields(frm)`, `valuation_method(frm)`,
`warehouse_group(frm, cdt, cdn)` — all defined with the ES6 shorthand form
(`key(args) {...}`) inside the *same* `frappe.ui.form.on("Item", {...})`
block. **Those already work**, because tree-sitter represents shorthand
object-literal methods as `method_definition` nodes regardless of whether
they sit in a `class` body or a plain object literal, and
`javascript_lang.rs`/`typescript_lang.rs` already query for
`(method_definition name: (property_identifier) @name) @def`.

So the real, narrower gap is specifically the **colon + function-expression**
form — `key: function(args) {...}` (and its arrow-function sibling,
`key: (args) => {...}`) — which tree-sitter represents as a `pair` node, not
a `method_definition`, and which currently has no query pattern at all.
This is still a real, common idiom (it's `item.js`'s own `refresh`,
`make_dashboard`, and `render_item_prices` handlers, among others in the same
file) but the earlier claim that this "under-indexes JS logic project-wide"
overstated it — the shorthand form, which is at least as common, is already
covered. Scope the fix and its expected impact accordingly.

**Fix.** Add one tree-sitter query pattern (plus its arrow-function variant)
in both `javascript_lang.rs` and `typescript_lang.rs`:
`(pair key: (property_identifier) @name value: (function_expression)) @def`
and `(pair key: (property_identifier) @name value: (arrow_function)) @def`
(kind `SymbolKind::Method`, mirroring `method_definition`'s existing
treatment). Confirm the exact node shape against the installed
`tree-sitter-javascript`/`tree-sitter-typescript` grammar version before
writing the query — don't guess from the pattern above alone.

**Risk: low-moderate.** Additive to the symbol set (more `Method` symbols
extracted, nothing removed), but touches the parser's core extraction path —
needs the project's standard parser-change validation: existing
`codegraph-parser` test suite must stay green, plus new fixture tests for
(a) the frappe.io `frappe.ui.form.on("X", {...})` shape specifically, (b) a
generic `{ key: function(){} }` object literal, (c) confirm it does *not*
false-positive on plain non-function `pair`s (`{ key: "value" }`) or on
values that happen to be identifiers referencing a function elsewhere
(`{ key: someFn }` — should NOT create a new symbol there; that's a
reference, not a definition). **Files:**
`crates/codegraph-parser/src/language/javascript_lang.rs`,
`typescript_lang.rs`, plus fixture tests in `crates/codegraph-parser/tests/`.

### Finding 4 — `get_file_context` underused relative to raw `Read` with guessed line offsets

**Evidence.** `get_file_context` called 21 times project-wide vs. 141 raw
`Read` calls. Several losing transcripts (Q10 is the clearest) show a
sequence of `Read(offset=995, limit=75)`, `Read(offset=1570, limit=40)` —
guessing at line ranges rather than calling `get_file_context` first to see
the file's symbol layout (which is exactly what that tool, built in the Plan
17 follow-up, exists to prevent). Lower-confidence finding than 1-3 (harder to
prove this changes outcomes vs. just token cost), but a cheap, low-risk
companion fix.

**Fix.** Minor description strengthening on `get_file_context` suggesting it
as the first move on an unfamiliar file, before a blind `Read`. **Risk:**
near-zero, additive text only. **Files:** `crates/codegraph-mcp/src/server.rs`.

### Finding 5 (background noise, not a fix target) — some losses are answer-precision, not retrieval

Q01 (−16.6), Q13 (−25.0), Q09 (−8.3), Q02 (−7.1) show reasonable MCP tool
usage (multiple targeted calls, not Grep-abandonment) but still lose on
attribution precision (e.g. citing the right formula but the wrong
function/file it lives in — see Q10's `apply_pricing_rule_on_item` (JS) vs.
`calculate_item_rate` (Python) mixup, which straddles Finding 2 and this one).
This looks like ordinary judge-strictness / LLM-writing-precision variance,
consistent with the already-established noise floor, not a systematic product
defect. **Do not chase this with scoring/rerank changes** — the keycloak
section already documents that ungated attempts to fix a similar-shaped
"near miss, not a total miss" problem regressed other things. Revisit only if
Findings 1-4 are shipped and validated and a gap of this shape is *still*
visible after averaging multiple repeat runs.

## Explicit non-goals for this plan

- **No rerank-weight changes** (α/β/γ, `TEST_FILE_PENALTY`, PPR damping). None
  of the evidence above points at scoring — it points at retrieval *coverage*
  breadth and tool *selection*, both upstream of rerank. Touching rerank
  weights without new evidence pointing there would repeat the exact mistake
  the keycloak section warns against.
- **No changes to `expand_hops`, `top_k`/`final_n` defaults** without a
  dedicated before/after measurement — raising candidate pool size is a
  plausible complementary lever for Finding 2/3 but has its own regression
  risk (dilutes precision elsewhere) and isn't evidenced strongly enough here
  to justify on its own.
- **Full automatic query decomposition** (Finding 2, tier 2) is scoped as a
  separate follow-on, not bundled into this plan's first pass — ship the
  cheap tier-1 fixes (1, 2-tier-1, 4) first, measure, then decide if tier 2 is
  still needed.

## Fix priority order (do in this order — each should be independently
   measurable before starting the next)

1. Finding 1 (tool description reframe) — cheapest, highest-confidence,
   directly explains the worst single loss (Q24).
2. Finding 2 tier 1 (find_relevant_code re-query nudge) — same class of fix,
   same file, ship alongside #1.
3. Finding 4 (get_file_context nudge) — trivial, ship alongside #1/#2.
4. Finding 3 (JS/TS object-literal method extraction) — real parser change,
   needs its own test cycle; do after 1/2/4 are measured so its effect isn't
   confounded with the description changes.
5. Finding 2 tier 2 (heuristic multi-aspect coverage note) — only if 1-4
   don't close enough of the gap; needs the full 2-corpus regression
   discipline before shipping.

## Validation protocol (do not skip — this project has been burned twice by trusting a single run)

- After 1-3 ship: rebuild, redeploy (`rm`-then-`cp`, full stop/start poll —
  the documented macOS code-signing gotcha), re-verify `cargo test --workspace`
  + clippy clean, then **at least 2 repeat 30-question codegraph runs** (not
  one) before comparing against graphify's existing 64.7% baseline. Report a
  mean and a range, not a single number, exactly like the honest reporting
  already done for the `get_file_context` fix above.
- Check `trace_impact`/`find_implementations` usage counts go up from 3/30
  and 1/30 — if the description change didn't change tool-call *behavior*,
  the coverage number won't be interpretable regardless of which way it moves
  (mirrors this project's own "mechanistic evidence beats a bare score"
  standard). **Treat Group B/C per-group coverage deltas as color, not a
  pass/fail criterion** — each group is only 5 questions, so a single
  question's ±30pp swing moves the group mean ±6pp, and the established
  ~13pp *whole-run* noise floor means a 5-question subgroup mean across only
  2 repeats has essentially no statistical resolving power. Anchor
  correctness claims on what's directly checkable instead (tool usage counts,
  Fix 4's symbols existing on a live query), not on whether a 5-question
  group average went up.
- After Finding 3 ships: verify item.js's `refresh` handler (and the function
  containing the `Item Price` permission check) now appear in
  `get_file_context`'s symbol list for `item.js`, via a direct live-daemon
  query — not just unit tests — before claiming it's fixed, matching the
  live-verification bar every other MCP-facing fix in this project's history
  has been held to.
- Do not declare "codegraph beats graphify" from one run. Given the
  established ~13pp noise floor and this run's near-tie (64.2% vs 64.7%, a
  0.5pp gap smaller than the noise floor by an order of magnitude), the
  honest framing even after these fixes land should be a *distribution*
  comparison (mean ± range across repeats), not a single-run headline.

## Success criteria

- `trace_impact` used in noticeably more than 3/30 questions after the
  description fix (directly measurable, cheap to check).
- Finding 3's new symbols verified present via live daemon query on `item.js`
  — specifically that `render_item_prices` (the exact previously-missing
  symbol, confirmed absent above) now appears — before claiming it's fixed.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D
  warnings` clean throughout, same bar as every prior plan in this file, plus
  new parser fixture tests asserting `render_item_prices`-shaped symbols are
  extracted and plain non-function `pair`s (`{ key: "value" }`) and
  identifier-reference values (`{ key: someFn }`) are correctly *not* turned
  into new symbols.
- Group B/C coverage and the overall codegraph-vs-graphify comparison are
  **not** pass/fail criteria for this plan (see the validation protocol above
  on why 5-question subgroups and a single/double repeat can't resolve that) —
  report them as color in the eventual writeup, not as the headline.
