use anyhow::anyhow;
use graphrelay_core::types::{FileRecord, Symbol};
use graphrelay_store::GraphDb;
use std::path::Path;

pub mod find_relevant_code;
pub mod find_implementations;
pub mod trace_impact;
pub mod explain_symbol;
pub mod get_file_context;
pub mod summarize_module;
pub mod watch_status;
pub mod search_code;
pub mod inspect;

/// Plan 23: outcome of resolving a `path` argument against the index — used
/// by `inspect` to decide whether to route into `get_file_context`'s
/// single-file behavior or `summarize_module`'s directory-prefix behavior.
/// A file match and an ambiguous match are kept distinct on purpose: an
/// ambiguous suffix match (multiple indexed files share the given suffix)
/// is a real problem the caller must resolve with a longer path, never
/// something to silently fall back past into a module summary.
pub enum PathLookup {
    File(FileRecord),
    NotAFile,
    Ambiguous(anyhow::Error),
}

/// Resolve `path` to an indexed file: exact match first (the common case —
/// the caller passes back the absolute `file` a prior result gave it), then
/// a unique repo-relative suffix match (what Read/Grep/Glob accept
/// elsewhere in the same agent session). Zero matches means "probably not a
/// file at all" (`NotAFile`) — the caller's job to decide what to try next,
/// not this function's.
pub fn resolve_file_lookup(state: &crate::server::McpAppState, path: &str) -> anyhow::Result<PathLookup> {
    if let Some(f) = state.graph_db.file_by_path(Path::new(path))? {
        return Ok(PathLookup::File(f));
    }
    let mut matches = state.graph_db.files_by_suffix(Path::new(path))?;
    match matches.len() {
        0 => Ok(PathLookup::NotAFile),
        1 => Ok(PathLookup::File(matches.remove(0))),
        n => {
            let shown: Vec<String> = matches
                .iter()
                .take(AMBIGUOUS_PATHS_SHOWN)
                .map(|f| f.path.to_string_lossy().into_owned())
                .collect();
            let more = if n > shown.len() { format!(", and {} more", n - shown.len()) } else { String::new() };
            Ok(PathLookup::Ambiguous(anyhow!(
                "ambiguous path: {path} matches {n} indexed files — pass a longer, more specific \
                 suffix or the absolute path: {}{more}",
                shown.join(", ")
            )))
        }
    }
}

/// How many candidate paths an ambiguous-path error lists (`__init__.py` on a
/// large Python repo matches hundreds).
const AMBIGUOUS_PATHS_SHOWN: usize = 10;

/// A span's lines as `"start-end"`, 1-based like `Read` and the line-numbered
/// `source` text in the same responses (spans themselves are 0-based).
pub fn line_range(span: &graphrelay_core::types::Span) -> String {
    format!("{}-{}", span.start_line + 1, span.end_line + 1)
}

/// Cap on how many caller/callee names a tool response includes inline (Plan
/// 21 item 7). `callers_of`/`callees_of` already bound the underlying DB
/// query at 200 rows for cross-file safety, but every tool response used to
/// echo that whole list back verbatim with no smaller display cap and no
/// signal that it was a partial view — a single hub symbol's caller list
/// could cost several thousand tokens of an agent's context on its own, and
/// there was no `count`/`truncated` field for the agent to tell "here are
/// all of them" from "here are the first N of many".
pub const CALLER_CALLEE_DISPLAY_CAP: usize = 10;

/// Implemented by both `Symbol` (used by `explain_symbol`'s `callers_of`/
/// `callees_of` results) and `RetrievalResult`'s lighter-weight `SymbolRef`
/// (used by `find_relevant_code`) so `capped_names` works for either without
/// an allocating conversion between them.
pub trait HasQualifiedName {
    fn qualified_name(&self) -> &str;
}
impl HasQualifiedName for Symbol {
    fn qualified_name(&self) -> &str { &self.qualified_name }
}
impl HasQualifiedName for graphrelay_core::types::SymbolRef {
    fn qualified_name(&self) -> &str { &self.qualified_name }
}

/// Names of up to `CALLER_CALLEE_DISPLAY_CAP` symbols, plus the real total
/// count and whether the list was truncated to reach the cap.
pub fn capped_names<T: HasQualifiedName>(symbols: &[T]) -> (Vec<&str>, usize, bool) {
    let total = symbols.len();
    let names = symbols
        .iter()
        .take(CALLER_CALLEE_DISPLAY_CAP)
        .map(|s| s.qualified_name())
        .collect();
    (names, total, total > CALLER_CALLEE_DISPLAY_CAP)
}

/// Plan 22 item 2: whether a result's underlying file may be out of sync with
/// the index *right now*. The live-watcher's debounce (`debounce_ms`,
/// default 50ms) is far shorter than the real staleness window — the whole
/// batch's processing time (graph writes, the repo-wide `resolve_edges()`,
/// and the FTS flush), measured at ~10.8s in CI's own
/// `rapid_saves_p95_latency_and_no_duplicates` test. During that window
/// `SourceLoader::snippet` can silently slice freshly-changed disk content
/// with the index's *old* line spans (a cache miss re-reads the new file but
/// still trusts the stale `Symbol::span`), or a cache hit can hand back
/// content that's stale relative to disk. This is a cheap per-result `stat`,
/// not a hook into the watcher's internal debounce state
/// (`FileWatcher::next_batch` isn't shared with any query path) — that
/// narrower approach would miss the in-flight-batch window itself, a
/// watcher-overflow re-index, and edits made while the daemon was down.
///
/// A result is possibly stale when any of:
/// 1. the file no longer exists / is unreadable;
/// 2. disk size differs from `files.byte_size`;
/// 3. disk mtime is later than `files.indexed_at`;
/// 4. `files.checksum == 0` — the pending-FTS sentinel
///    (`IndexWorker::index_path_graph_only` writes this until
///    `flush_chunk_batch`'s FTS write commits; see that invariant in
///    CLAUDE.md).
///
/// Known false-negative, documented not hidden: `indexed_at` is stamped in
/// `Extractor::build_extraction` *after* the file was read, so an edit
/// landing between the read and the stamp can have `mtime < indexed_at`.
/// Rule 2 (size) catches most of these; rule 4 (checksum sentinel) catches
/// the rest until the FTS flush commits. Filesystem mtime resolution (1-2s
/// on some filesystems) is the same class of edge case. Returns `false` (not
/// stale) for a path that isn't an indexed file at all — that's a different
/// problem this check isn't responsible for.
pub fn is_possibly_stale(graph_db: &GraphDb, path: &Path) -> bool {
    let Ok(Some(file)) = graph_db.file_by_path(path) else {
        return false;
    };
    if file.checksum == 0 {
        return true;
    }
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return true,
    };
    if meta.len() != file.byte_size {
        return true;
    }
    if let Ok(modified) = meta.modified() {
        if let Ok(indexed_at) = file.indexed_at.parse::<chrono::DateTime<chrono::Utc>>() {
            let modified_dt: chrono::DateTime<chrono::Utc> = modified.into();
            if modified_dt > indexed_at {
                return true;
            }
        }
    }
    false
}

/// Truncate a source snippet so MCP responses stay below Claude Code's inline display limit.
/// Returns the first `max_chars` Unicode scalar values followed by `…` if truncated.
pub fn truncate_snippet(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Plan 23: repo-size-adaptive character budget for a tool response's source
/// content, replacing the old flat 600/800-char literal caps that made every
/// repo — a 100-file project and a 44,000-symbol one — get identically-sized
/// snippets. A competitor tool's benchmarked efficiency edge traced mostly to
/// returning near-full source per result instead of a short fragment, so an
/// agent doesn't need a follow-up `Read` for every promising lead.
///
/// `total_chars` is held constant across tiers deliberately: our own larger
/// benchmark corpora (erpnext, keycloak) already show lower retrieval MRR
/// than the small lexical corpus, so a large repo needs more candidate
/// *breadth* kept visible, not more depth per candidate — shrinking
/// `per_item_chars` while holding the total steady is the right shape for
/// that, not the reverse. These thresholds are a reasoned initial default,
/// not a swept optimum — flagged for a future benchmarking pass, per this
/// project's own convention of shipping a reasoned default rather than
/// blocking on a full sweep first (see `TEST_FILE_PENALTY`'s history).
pub struct ResponseBudget {
    pub per_item_chars: usize,
    pub total_chars: usize,
    pub per_item_floor_chars: usize,
    /// Plan 23 follow-up (2026-09-28): `inspect`'s single-file
    /// `include_source` mode was using `total_chars` (24000, meant to be
    /// *shared* across several ranked results) as a per-call cap for one
    /// call returning one file — measured live at 20-36K chars per call in
    /// a real benchmark run, ~5-9K tokens each, the dominant driver of a
    /// real cost increase after that default flipped to `true`. This is a
    /// separate, smaller tier: still large enough to make most single files
    /// fully readable in one call (the whole point of `include_source`),
    /// but scaled like every other budget here instead of getting the
    /// multi-result total outright.
    pub single_file_chars: usize,
}

pub fn response_budget(total_files: u64) -> ResponseBudget {
    let (per_item_chars, single_file_chars) = if total_files < 500 {
        (4000, 8000)
    } else if total_files < 5000 {
        (2500, 6000)
    } else {
        (1500, 4000)
    };
    ResponseBudget {
        per_item_chars,
        total_chars: 24_000,
        per_item_floor_chars: 300,
        single_file_chars,
    }
}

/// Like `truncate_snippet`, but never cuts mid-line — a syntax-broken
/// fragment isn't usable in place of a real `Read` the way a whole-line
/// truncation is. Falls back to a raw char truncation only when a single
/// line by itself already exceeds `max_chars` (a minified file, one very
/// long generated line). Returns the truncated text and whether truncation
/// happened.
pub fn truncate_snippet_on_line_boundary(s: &str, max_chars: usize) -> (String, bool) {
    if s.chars().count() <= max_chars {
        return (s.trim_end_matches('\n').to_string(), false);
    }
    let mut out = String::new();
    for line in s.split_inclusive('\n') {
        let candidate_len = out.chars().count() + line.chars().count();
        if candidate_len > max_chars {
            if out.is_empty() {
                // A single line already exceeds the whole budget — fall back
                // to a raw char cut rather than returning nothing at all.
                return (truncate_snippet(s, max_chars), true);
            }
            break;
        }
        out.push_str(line);
    }
    (out.trim_end_matches('\n').to_string(), true)
}

/// Distribute `budget.total_chars` across `snippets`, in the order given
/// (callers pass rank order): each item gets `remaining_total /
/// remaining_items` chars, clamped to `[per_item_floor_chars,
/// per_item_chars]`, and the amount actually kept is deducted from the
/// running total before moving to the next item — so one long early result
/// doesn't starve every later one, but no ranked result is ever dropped,
/// only shrunk toward the floor.
pub fn size_snippets_fair_share(snippets: &[String], budget: &ResponseBudget) -> Vec<(String, bool)> {
    let mut remaining_total = budget.total_chars;
    let mut remaining_items = snippets.len();
    let mut out = Vec::with_capacity(snippets.len());
    for s in snippets {
        let fair_share = remaining_total
            .checked_div(remaining_items)
            .unwrap_or(budget.per_item_floor_chars);
        let cap = fair_share.clamp(budget.per_item_floor_chars, budget.per_item_chars);
        let (truncated, was_truncated) = truncate_snippet_on_line_boundary(s, cap);
        remaining_total = remaining_total.saturating_sub(truncated.chars().count());
        remaining_items = remaining_items.saturating_sub(1);
        out.push((truncated, was_truncated));
    }
    out
}

/// `(rank, start_line, end_line)` for one matched symbol — 1-based,
/// inclusive line numbers (Read's convention).
pub type SymbolSpan = (usize, u32, u32);

/// `(full_text, spans, group_ceiling)` for one file-group, as consumed by
/// `size_file_sources_fair_share`.
pub type FileSourceGroup = (String, Vec<SymbolSpan>, usize);

/// Render a file's source as the union of its matched symbols' own line
/// spans (1-based, inclusive — Read's convention), merged where
/// overlapping/adjacent, in file order, with an "...\n" elision marker
/// between kept ranges that aren't adjacent. `spans` is `(rank, start_line,
/// end_line)` per matched symbol in this file, in any order.
///
/// Every span is kept in full unless the whole group doesn't fit
/// `max_chars`, in which case spans are dropped starting from the lowest
/// rank (never the group's own best/rank-1 span) until the rest fits; if
/// even the top span alone doesn't fit, it is truncated on a line boundary
/// (`truncate_snippet_on_line_boundary`) rather than dropped. This mirrors
/// `size_snippets_fair_share`'s own guarantee ("no ranked result is ever
/// silently dropped, only shrunk") — an earlier design centered one window
/// on only the best-ranked symbol in a group, which had no guarantee of
/// covering a second matched symbol far away in the same file; found and
/// rejected during design review before shipping (2026-09-28).
///
/// Returns `(rendered_text, anything_dropped_or_truncated)`.
pub fn spans_source(content: &str, spans: &[SymbolSpan], max_chars: usize) -> (String, bool) {
    if spans.is_empty() {
        return (String::new(), false);
    }
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return (String::new(), false);
    }

    // Best (lowest rank number) first, so truncating the tail drops the
    // lowest-priority spans first.
    let mut by_rank: Vec<SymbolSpan> = spans.to_vec();
    by_rank.sort_by_key(|&(rank, _, _)| rank);

    let render = |subset: &[SymbolSpan]| -> String {
        let mut ranges: Vec<(usize, usize)> = subset
            .iter()
            .map(|&(_, s, e)| {
                let start_idx = (s as usize).saturating_sub(1).min(lines.len().saturating_sub(1));
                let end_idx = (e as usize).saturating_sub(1).min(lines.len().saturating_sub(1));
                (start_idx.min(end_idx), start_idx.max(end_idx))
            })
            .collect();
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for r in ranges.drain(..) {
            if let Some(last) = merged.last_mut() {
                if r.0 <= last.1 + 1 {
                    last.1 = last.1.max(r.1);
                    continue;
                }
            }
            merged.push(r);
        }
        let mut out = String::new();
        for (i, (start, end)) in merged.iter().enumerate() {
            if i > 0 {
                out.push_str("...\n");
            }
            for (idx, line) in lines.iter().enumerate().take(*end + 1).skip(*start) {
                out.push_str(&format!("{}: {line}\n", idx + 1));
            }
        }
        out.trim_end_matches('\n').to_string()
    };

    for k in (1..=by_rank.len()).rev() {
        let subset = &by_rank[..k];
        let rendered = render(subset);
        if rendered.chars().count() <= max_chars {
            return (rendered, k < by_rank.len());
        }
    }

    // Even the single top-ranked span alone doesn't fit — truncate it on a
    // line boundary rather than dropping it entirely.
    let top_only = render(&by_rank[..1]);
    let (truncated_text, _) = truncate_snippet_on_line_boundary(&top_only, max_chars);
    (truncated_text, true)
}

/// Like `size_snippets_fair_share`, but operates on whole file-groups
/// instead of individual snippets: each group's own per-item ceiling is
/// `group_ceiling` (the caller passes `min(single_file_chars, symbol_count *
/// per_item_chars)` — a multi-symbol group earns more room, a single-symbol
/// group stays close to what a single snippet used to cost), fair-shared
/// against the same `total_chars` budget, and rendered via `spans_source`
/// (span-aware — the relevant content isn't always near the start of the
/// file, unlike `truncate_snippet_on_line_boundary`'s from-the-top
/// truncation).
pub fn size_file_sources_fair_share(
    groups: &[FileSourceGroup],
    budget: &ResponseBudget,
) -> Vec<(String, bool)> {
    let mut remaining_total = budget.total_chars;
    let mut remaining_items = groups.len();
    let mut out = Vec::with_capacity(groups.len());
    for (text, spans, group_ceiling) in groups {
        let fair_share = remaining_total
            .checked_div(remaining_items)
            .unwrap_or(budget.per_item_floor_chars);
        let cap = fair_share.clamp(budget.per_item_floor_chars, *group_ceiling);
        let (rendered, truncated) = spans_source(text, spans, cap);
        remaining_total = remaining_total.saturating_sub(rendered.chars().count());
        remaining_items = remaining_items.saturating_sub(1);
        out.push((rendered, truncated));
    }
    out
}

/// Resolve a symbol by qualified name or plain name (highest in-degree wins).
///
/// Ranking must see *all* same-named candidates, not an arbitrary prefix of a fuzzy
/// substring match — a small `LIMIT` taken before ranking can exclude the true
/// highest-in-degree match for a common name (e.g. `run`, `handle`, `init`). We use
/// the indexed exact-name lookup rather than `search_symbols_by_name`'s `LIKE`.
pub fn resolve_symbol(graph_db: &GraphDb, input: &str) -> anyhow::Result<Symbol> {
    resolve_symbol_verbose(graph_db, input).map(|(sym, _note)| sym)
}

/// Same resolution as [`resolve_symbol`], but also returns a `note` whenever the match
/// came from a same-name fallback with real ambiguity (2+ indexed symbols share the
/// resolved name) — worth surfacing to the caller, since silently picking the
/// highest-in-degree candidate for a common name (`validate`, `run`, `handle`) can hand
/// back the wrong symbol's source with no signal that anything was guessed at all.
pub fn resolve_symbol_verbose(graph_db: &GraphDb, input: &str) -> anyhow::Result<(Symbol, Option<String>)> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("symbol name must not be empty"));
    }

    // Try qualified name first — exact, no ambiguity possible.
    if let Some(sym) = graph_db.symbol_by_qualified(input)? {
        return Ok((sym, None));
    }

    // Agents commonly pass a `<repo-relative-path>::<name>` qualified name (mirroring
    // what find_relevant_code/get_file_context echo back), but the real qualified_name
    // is keyed on the indexed *absolute* path — an exact match above then fails even
    // though the symbol exists. Recover by suffix-matching the path portion against
    // indexed files (same approach as get_file_context's path resolution) and looking
    // for the name portion among that file's symbols, which narrows ambiguity before
    // any in-degree tiebreak — a plain bare-name lookup below cannot do this because
    // "name" never contains "::", so a qualified-shaped input always misses it entirely.
    let name_part = if let Some((path_part, name_part)) = input.rsplit_once("::") {
        if let Ok(files) = graph_db.files_by_suffix(Path::new(path_part)) {
            let candidates: Vec<Symbol> = files
                .into_iter()
                .filter_map(|f| graph_db.symbols_in_file(f.id).ok())
                .flatten()
                .filter(|s| s.name == name_part)
                .collect();
            if let Some(sym) = pick_highest_in_degree(graph_db, candidates) {
                return Ok(sym);
            }
        }
        name_part
    } else if let Some((_, last)) = input.rsplit_once('.') {
        // Agents also commonly guess a dotted form — `Class.method` or a fully-qualified
        // Python import path (`erpnext.stock.doctype.x.x.Class.method`) — neither of
        // which is how this index's qualified names or plain `name` field are shaped.
        // Falling through to the bare-name tail below with the *whole* input as `name`
        // can never match (a symbol's `name` never contains "." or a space), so without
        // this the call fails outright even though the target symbol is indexed and
        // reachable by its own bare name. Only tried once "::" has already failed/isn't
        // present, so it never fights that more-precise, file-scoped resolution.
        last
    } else {
        input
    };

    // Exact name match, ranked by in-degree, over every candidate that shares the name.
    let candidates = graph_db.symbols_by_exact_name(name_part, 5000)?;
    pick_highest_in_degree(graph_db, candidates)
        .ok_or_else(|| anyhow::Error::new(SymbolNotFound(input.to_string())))
}

/// Typed so callers can tell "no such symbol" from a database failure.
#[derive(Debug)]
pub struct SymbolNotFound(pub String);

impl std::fmt::Display for SymbolNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "symbol not found: {}", self.0)
    }
}

impl std::error::Error for SymbolNotFound {}

fn pick_highest_in_degree(graph_db: &GraphDb, candidates: Vec<Symbol>) -> Option<(Symbol, Option<String>)> {
    let ambiguous = candidates.len() > 1;
    // One batched query, not one `COUNT(*)` per candidate: a common name
    // (`__init__`, `validate`) can have thousands of same-name candidates.
    let degrees = if ambiguous {
        let ids: Vec<_> = candidates.iter().map(|s| s.id).collect();
        graph_db.in_degrees_batch(&ids).unwrap_or_else(|e| {
            tracing::warn!("in_degrees_batch failed: {e}");
            Default::default()
        })
    } else {
        Default::default()
    };
    let sym = candidates.into_iter().max_by_key(|s| degrees.get(&s.id).copied().unwrap_or(0))?;
    let note = ambiguous.then(|| format!(
        "Resolved by name only (\"{}\"), not an exact indexed path — other symbols share \
         this name; picked the one with the most callers as the likely match, but verify \
         `file`/`symbol` below is really the one you meant, especially for a common name.",
        sym.name
    ));
    Some((sym, note))
}

#[cfg(test)]
mod response_budget_tests {
    use super::*;

    #[test]
    fn small_repo_gets_generous_per_item_cap() {
        let b = response_budget(100);
        assert_eq!(b.per_item_chars, 4000);
        assert_eq!(b.total_chars, 24_000);
    }

    #[test]
    fn mid_repo_gets_medium_per_item_cap() {
        let b = response_budget(1000);
        assert_eq!(b.per_item_chars, 2500);
    }

    #[test]
    fn large_repo_gets_tight_per_item_cap() {
        let b = response_budget(50_000);
        assert_eq!(b.per_item_chars, 1500);
    }

    #[test]
    fn tier_boundaries_are_inclusive_of_the_lower_tier() {
        // total_files == 500 must land in the mid tier, not the small one —
        // the `< 500` check is exclusive on the boundary itself.
        assert_eq!(response_budget(499).per_item_chars, 4000);
        assert_eq!(response_budget(500).per_item_chars, 2500);
        assert_eq!(response_budget(4999).per_item_chars, 2500);
        assert_eq!(response_budget(5000).per_item_chars, 1500);
    }

    #[test]
    fn single_file_chars_scales_with_repo_size_like_everything_else() {
        // Regression pin (2026-09-28): inspect's include_source mode used to
        // use the flat 24000-char total_chars budget as a per-call cap for
        // one file — measured live at 20-36K chars/call, the dominant driver
        // of a real cost increase. single_file_chars must stay well under
        // total_chars and scale down for larger repos the same direction
        // per_item_chars does.
        for total_files in [100, 1000, 10_000] {
            let b = response_budget(total_files);
            assert!(b.single_file_chars < b.total_chars,
                "single_file_chars ({}) must stay below total_chars ({}) at total_files={total_files}",
                b.single_file_chars, b.total_chars);
        }
        assert_eq!(response_budget(100).single_file_chars, 8000);
        assert_eq!(response_budget(1000).single_file_chars, 6000);
        assert_eq!(response_budget(10_000).single_file_chars, 4000);
    }

    #[test]
    fn short_text_is_not_truncated() {
        let (out, truncated) = truncate_snippet_on_line_boundary("fn a() {}\n", 100);
        assert_eq!(out, "fn a() {}\n".trim_end_matches('\n'));
        assert!(!truncated);
    }

    #[test]
    fn truncation_falls_on_a_line_boundary_not_mid_line() {
        let s = "line one\nline two\nline three\n";
        let (out, truncated) = truncate_snippet_on_line_boundary(s, 15);
        assert!(truncated);
        assert_eq!(out, "line one");
        for line in out.lines() {
            assert!(s.lines().any(|l| l == line), "kept a partial line: {line:?}");
        }
    }

    #[test]
    fn single_overlong_line_falls_back_to_char_truncation() {
        let s = "x".repeat(500);
        let (out, truncated) = truncate_snippet_on_line_boundary(&s, 50);
        assert!(truncated);
        assert_eq!(out.chars().count(), 51); // 50 + the truncation ellipsis
    }

    #[test]
    fn fair_share_never_drops_a_result_below_the_floor() {
        let budget = ResponseBudget { per_item_chars: 4000, total_chars: 100, per_item_floor_chars: 30, single_file_chars: 8000 };
        let snippets: Vec<String> = (0..10).map(|_| "x".repeat(1000)).collect();
        let sized = size_snippets_fair_share(&snippets, &budget);
        assert_eq!(sized.len(), 10);
        for (text, truncated) in &sized {
            assert!(truncated);
            // 30 (floor) + 1 for the "…" the single-overlong-line fallback appends.
            assert!(text.chars().count() <= 31, "expected floor-capped, got {}", text.chars().count());
        }
    }

    #[test]
    fn fair_share_gives_short_results_their_natural_length_not_the_full_cap() {
        let budget = ResponseBudget { per_item_chars: 4000, total_chars: 24_000, per_item_floor_chars: 300, single_file_chars: 8000 };
        let snippets = vec!["short\n".to_string(), "y".repeat(10_000)];
        let sized = size_snippets_fair_share(&snippets, &budget);
        assert_eq!(sized[0].0, "short");
        assert!(!sized[0].1);
    }

    fn numbered_lines(n: u32) -> String {
        (1..=n).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n") + "\n"
    }

    #[test]
    fn spans_source_merges_adjacent_spans_with_no_elision_marker() {
        let content = numbered_lines(30);
        let (out, truncated) = spans_source(&content, &[(1, 5, 7), (2, 8, 10)], 10_000);
        assert!(!truncated);
        assert!(!out.contains("..."), "adjacent spans must merge, not show an elision marker: {out}");
        assert!(out.contains("5: line5"));
        assert!(out.contains("10: line10"));
    }

    #[test]
    fn spans_source_inserts_elision_marker_for_a_real_gap() {
        let content = numbered_lines(30);
        let (out, truncated) = spans_source(&content, &[(1, 5, 6), (2, 20, 21)], 10_000);
        assert!(!truncated); // both spans kept in full, just not adjacent
        assert!(out.contains("..."), "a real gap between kept ranges must show an elision marker: {out}");
        assert!(out.contains("5: line5"));
        assert!(out.contains("20: line20"));
    }

    #[test]
    fn spans_source_drops_lowest_rank_span_first_when_group_overflows() {
        let content = numbered_lines(30);
        // Each span alone renders small; together they don't fit a tight budget.
        let (out, truncated) = spans_source(&content, &[(1, 1, 1), (2, 20, 20)], 15);
        assert!(truncated);
        assert!(out.contains("line1"), "the rank-1 (best) span must survive: {out}");
        assert!(!out.contains("line20"), "the rank-2 (worse) span must be the one dropped: {out}");
    }

    #[test]
    fn spans_source_truncates_rather_than_drops_the_top_span_when_it_alone_overflows() {
        let content = format!("{}\n", "x".repeat(200));
        let (out, truncated) = spans_source(&content, &[(1, 1, 1)], 20);
        assert!(truncated);
        assert!(out.chars().count() <= 21, "expected floor-capped with ellipsis, got {}", out.chars().count());
    }

    #[test]
    fn spans_source_empty_spans_returns_empty_not_truncated() {
        let content = numbered_lines(5);
        let (out, truncated) = spans_source(&content, &[], 1000);
        assert_eq!(out, "");
        assert!(!truncated);
    }

    #[test]
    fn file_group_ceiling_caps_below_the_fair_share_when_group_has_few_symbols() {
        let content = numbered_lines(1000);
        let budget = ResponseBudget { per_item_chars: 2500, total_chars: 24_000, per_item_floor_chars: 300, single_file_chars: 6000 };
        // Group A: 1 symbol -> caller passes ceiling = min(6000, 1*2500) = 2500.
        // Group B: 3 symbols -> caller passes ceiling = min(6000, 3*2500) = 6000.
        let groups = vec![
            (content.clone(), vec![(1usize, 1u32, 1000u32)], 2500usize),
            (content.clone(), vec![(1usize, 1u32, 1000u32), (2, 500, 500), (3, 999, 999)], 6000usize),
        ];
        let sized = size_file_sources_fair_share(&groups, &budget);
        assert!(sized[0].0.chars().count() <= 2500, "group A exceeded its own ceiling: {}", sized[0].0.chars().count());
        assert!(sized[1].0.chars().count() <= 6000, "group B exceeded its own ceiling: {}", sized[1].0.chars().count());
        assert!(sized[0].1);
        assert!(sized[1].1);
    }

    #[test]
    fn file_group_fair_share_never_starves_a_group_below_the_floor() {
        let budget = ResponseBudget { per_item_chars: 4000, total_chars: 100, per_item_floor_chars: 30, single_file_chars: 8000 };
        let content = format!("{}\n", "x".repeat(2000));
        let groups: Vec<(String, Vec<(usize, u32, u32)>, usize)> =
            (0..10).map(|_| (content.clone(), vec![(1usize, 1u32, 1u32)], 8000usize)).collect();
        let sized = size_file_sources_fair_share(&groups, &budget);
        for (text, truncated) in &sized {
            assert!(*truncated);
            assert!(text.chars().count() <= 31, "expected floor-capped, got {}", text.chars().count());
        }
    }
}

#[cfg(test)]
mod stale_tests {
    use super::*;
    use graphrelay_core::types::{FileRecord, Language};
    use std::io::Write;

    /// Real file on disk + a real `GraphDb` row whose metadata matches it —
    /// the baseline every other test in this module perturbs one field of.
    fn fresh_file(dir: &std::path::Path, db: &GraphDb, name: &str, contents: &str) -> (std::path::PathBuf, FileRecord) {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        drop(f);
        let indexed_at = chrono::Utc::now().to_rfc3339();
        let fr = FileRecord {
            id: 1, path: path.clone(), checksum: 42,
            language: Language::Rust, indexed_at, byte_size: contents.len() as u64,
        };
        db.upsert_file(&fr).unwrap();
        (path, fr)
    }

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cg-stale-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fresh_unchanged_file_is_not_stale() {
        let db = GraphDb::open_in_memory().unwrap();
        let dir = tmp_dir("fresh");
        let (path, _) = fresh_file(&dir, &db, "a.rs", "fn a() {}\n");
        assert!(!is_possibly_stale(&db, &path));
    }

    #[test]
    fn deleted_file_is_stale() {
        let db = GraphDb::open_in_memory().unwrap();
        let dir = tmp_dir("deleted");
        let (path, _) = fresh_file(&dir, &db, "b.rs", "fn b() {}\n");
        std::fs::remove_file(&path).unwrap();
        assert!(is_possibly_stale(&db, &path));
    }

    #[test]
    fn size_changed_on_disk_is_stale() {
        let db = GraphDb::open_in_memory().unwrap();
        let dir = tmp_dir("size");
        let (path, _) = fresh_file(&dir, &db, "c.rs", "fn c() {}\n");
        // Grow the file without updating the DB row — simulates an edit that
        // landed after indexing but before the watcher re-indexed it.
        std::fs::write(&path, "fn c() { /* much longer now */ }\n").unwrap();
        assert!(is_possibly_stale(&db, &path));
    }

    #[test]
    fn mtime_after_indexed_at_is_stale() {
        let db = GraphDb::open_in_memory().unwrap();
        let dir = tmp_dir("mtime");
        let path = dir.join("d.rs");
        std::fs::write(&path, "fn d() {}\n").unwrap();
        // indexed_at set far in the past relative to the file's real mtime
        // (just written above) — same byte_size, so only rule 3 (mtime) can
        // catch this.
        let fr = FileRecord {
            id: 1, path: path.clone(), checksum: 42, language: Language::Rust,
            indexed_at: "2020-01-01T00:00:00Z".to_string(),
            byte_size: std::fs::metadata(&path).unwrap().len(),
        };
        db.upsert_file(&fr).unwrap();
        assert!(is_possibly_stale(&db, &path));
    }

    #[test]
    fn checksum_zero_pending_sentinel_is_stale() {
        let db = GraphDb::open_in_memory().unwrap();
        let dir = tmp_dir("checksum0");
        let path = dir.join("e.rs");
        std::fs::write(&path, "fn e() {}\n").unwrap();
        let fr = FileRecord {
            id: 1, path: path.clone(), checksum: 0, language: Language::Rust,
            indexed_at: chrono::Utc::now().to_rfc3339(),
            byte_size: std::fs::metadata(&path).unwrap().len(),
        };
        db.upsert_file(&fr).unwrap();
        assert!(is_possibly_stale(&db, &path), "checksum==0 is the pending-FTS sentinel");
    }

    #[test]
    fn unindexed_path_is_not_flagged_stale() {
        // Not this check's job — a path with no `files` row at all (never
        // indexed, or already deleted from the index) isn't "possibly
        // stale", it's just absent.
        let db = GraphDb::open_in_memory().unwrap();
        assert!(!is_possibly_stale(&db, std::path::Path::new("/never/indexed.rs")));
    }
}
