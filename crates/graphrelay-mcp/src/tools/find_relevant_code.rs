use crate::server::McpAppState;
use crate::tools::{capped_names, is_possibly_stale, response_budget, size_file_sources_fair_share, FileSourceGroup, SymbolSpan};
use graphrelay_core::retrieval::{RetrievalQuery, RetrievalResult};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Bounds `top_n`: the candidate pool is ~100-120 symbols anyway, and the
/// response budget is shared across every returned file.
pub(crate) const MAX_TOP_N: usize = 30;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub intent:   String,
    pub top_n:    Option<usize>,
    pub language: Option<String>,
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    tracing::info!(intent = %args.intent, top_n = ?args.top_n, "find_relevant_code");
    let req = RetrievalQuery {
        text:        args.intent.clone(),
        top_k:       state.pipeline.candidate_top_k(),
        final_n:     args.top_n.unwrap_or(8).clamp(1, MAX_TOP_N),
        expand_hops: 1,
        lang_filter: args.language,
    };

    let results = state.pipeline.query(req).await?;

    // Plan 23 follow-up (2026-09-28): group by file instead of one flat
    // per-symbol list. Real transcript evidence found 91% of intent-mode
    // calls had at least one file appear 2+ times among their results, and
    // agents commonly followed up with a same-file `inspect`/`Read` call
    // right after — one merged, real-source block per file (instead of N
    // small per-symbol snippets) lets the agent act on a same-file hit
    // without that extra round trip. Ranking itself is untouched: grouping
    // only reorganizes the pipeline's already-ranked, already-`top_n`-
    // truncated `results`, so `count` keeps its old meaning (total matched
    // symbols) and `scripts/strict-rank-eval.py`'s MRR is unaffected once
    // it reads the new (nested) location for each symbol's own rank.
    //
    // `total_files` is already-materialized in-memory state (not a fresh DB
    // query), since `GraphDb`'s connection mutex is a documented contention
    // risk under load and this value is cheap to keep current already.
    let total_files = state.watch_status.read().await.total_files;
    let budget = response_budget(total_files);

    // A linear "find existing group or push new" scan — not a HashMap —
    // because `results` is already rank-sorted, so first-seen order during
    // this scan is automatically min-rank order (the group containing the
    // #1 overall hit ends up first) with no explicit sort needed.
    let mut groups: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (i, r) in results.iter().enumerate() {
        match groups.iter().position(|(p, _)| p == &r.file_path) {
            Some(pos) => groups[pos].1.push(i),
            None => groups.push((r.file_path.clone(), vec![i])),
        }
    }

    let sizing_input: Vec<FileSourceGroup> = groups.iter().map(|(path, idxs)| {
        let spans: Vec<SymbolSpan> = idxs.iter().map(|&i| {
            let r = &results[i];
            (r.rank, r.symbol.span.start_line + 1, r.symbol.span.end_line + 1)
        }).collect();
        let text = match state.source_loader.full_text(path) {
            Ok(arc) => arc.to_string(),
            Err(e) => {
                tracing::warn!(path = %path.display(), "full_text load failed for search_code grouping: {e}");
                idxs.iter().map(|&i| results[i].snippet.clone()).collect::<Vec<_>>().join("\n...\n")
            }
        };
        let ceiling = (idxs.len() * budget.per_item_chars).min(budget.single_file_chars);
        (text, spans, ceiling)
    }).collect();
    let sized_sources = size_file_sources_fair_share(&sizing_input, &budget);

    let result_jsons: Vec<Value> = groups.iter().zip(sized_sources).map(|((path, idxs), (source, source_truncated))| {
        let symbols_json: Vec<Value> = idxs.iter().map(|&i| {
            let r = &results[i];
            let (caller_names, callers_count, callers_truncated) = capped_names(&r.callers);
            let (callee_names, callees_count, callees_truncated) = capped_names(&r.callees);
            serde_json::json!({
                "rank":      r.rank,
                "score":     r.score,
                "symbol":    r.symbol.qualified_name,
                "kind":      format!("{:?}", r.symbol.kind),
                "lines":     crate::tools::line_range(&r.symbol.span),
                "callers":           caller_names,
                "callers_count":     callers_count,
                "callers_truncated": callers_truncated,
                "callees":           callee_names,
                "callees_count":     callees_count,
                "callees_truncated": callees_truncated,
            })
        }).collect();

        let mut item = serde_json::json!({
            "file":              path.display().to_string(),
            "source":            source,
            "source_truncated":  source_truncated,
            "symbols":           symbols_json,
        });
        // Plan 22 item 2: per-file now (was per-symbol), emitted only when true.
        if is_possibly_stale(&state.graph_db, path) {
            item.as_object_mut().expect("json! built an object")
                .insert("possibly_stale".to_string(), serde_json::json!(true));
        }
        item
    }).collect();
    let any_stale = result_jsons.iter().any(|r| r.get("possibly_stale").is_some());

    let mut response = serde_json::json!({
        "query":      args.intent,
        "count":      results.len(),
        "file_count": groups.len(),
        "results":    result_jsons,
    });

    if let Some(note) = concentration_note(&results) {
        response["note"] = serde_json::json!(note);
    } else if any_stale {
        response["note"] = serde_json::json!(
            "One or more results are marked `possibly_stale` — they may have changed \
             since indexing (recent edit still processing). Read those files directly \
             before relying on `source`/`lines`."
        );
    }

    Ok(response)
}

/// A single natural-language query anchors to one semantic neighborhood. When a
/// question actually has multiple distinct aspects living in different parts of
/// the codebase, one call's top results can end up entirely from one module
/// while a second, equally-relevant aspect never surfaces at all (see the ERPNext
/// benchmark transcripts, e.g. Q05: a "how is X computed, and what prevents Y"
/// question where every one of 10 results covered the X half only). Surfacing
/// that concentration lets the calling agent choose a follow-up `find_relevant_code`
/// call with a narrower/different intent instead of abandoning the tool for
/// unguided Grep — which is what burned the turn budget on those transcripts.
///
/// Grouping is by directory **ancestor containment**, not a fixed path depth.
/// Two false starts before landing here, both caught by testing against the
/// real Q05/Q13/Q21/Q23 transcripts rather than trusting synthetic fixtures
/// alone:
/// 1. Exact immediate-parent-directory grouping treated `work_order/` and
///    `work_order/services/` as different buckets, so a doctype's root file
///    plus its `services/` submodule (a common layout) never crossed the
///    threshold (max group 5/10) — it would have silently never fired.
/// 2. A fixed "first N path components" depth looked right against
///    hand-rolled relative-path fixtures, but `RetrievalResult::file_path` is
///    absolute — the first N components of an absolute path are the
///    filesystem prefix (e.g. `/Users/name/...`), identical for every result
///    in a project, so it fired on 100% of batches regardless of actual
///    content (verified live against the rebuilt binary: all 4 queries
///    "fired", including the two that shouldn't have).
///
/// Ancestor containment sidesteps both: a file's directory is either an
/// ancestor of another result's directory (same module, different nesting —
/// `work_order/` contains `work_order/services/`) or it isn't (genuinely
/// different modules — `work_order/` vs `sales_order/`), and this holds for
/// absolute paths with no project-root plumbing needed. Verified against all
/// 4 transcripts: fires for Q05 (8/10, the real motivating case), does not
/// fire for Q13/Q21 (0.4 each — genuine multi-module coverage, not a miss) or
/// Q23 (0.7 — a documented near-miss, left as-is rather than lowering the
/// threshold to chase one anecdote).
fn concentration_note(results: &[RetrievalResult]) -> Option<String> {
    const MIN_RESULTS: usize = 3;
    const CONCENTRATION_THRESHOLD: f64 = 0.8;

    if results.len() < MIN_RESULTS {
        return None;
    }

    let dirs: Vec<&std::path::Path> = results
        .iter()
        .filter_map(|r| r.file_path.parent())
        .collect();

    let mut distinct: Vec<&std::path::Path> = dirs.clone();
    distinct.sort_unstable();
    distinct.dedup();
    distinct.sort_by_key(|d| d.components().count());

    let mut representatives: Vec<&std::path::Path> = Vec::new();
    let mut dir_to_rep: HashMap<&std::path::Path, &std::path::Path> = HashMap::new();
    for &d in &distinct {
        match representatives.iter().find(|&&rep| d.starts_with(rep)) {
            Some(&rep) => {
                dir_to_rep.insert(d, rep);
            }
            None => {
                representatives.push(d);
                dir_to_rep.insert(d, d);
            }
        }
    }

    let mut counts: HashMap<&std::path::Path, usize> = HashMap::new();
    for d in &dirs {
        let rep = dir_to_rep.get(d).copied().unwrap_or(d);
        *counts.entry(rep).or_insert(0) += 1;
    }

    let max_count = counts.values().copied().max().unwrap_or(0);
    let ratio = max_count as f64 / results.len() as f64;

    if ratio >= CONCENTRATION_THRESHOLD {
        Some(
            "Results are concentrated in one directory. If your question has \
             multiple distinct aspects, they may live in a different part of the \
             codebase that this query didn't surface — try a follow-up \
             search_code call (intent mode) with a narrower or differently-worded \
             intent before falling back to manual search."
                .to_string(),
        )
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphrelay_core::types::{Language, Span, Symbol, SymbolKind};
    use std::path::PathBuf;

    fn fixture(path: &str) -> RetrievalResult {
        RetrievalResult {
            rank: 1,
            score: 1.0,
            symbol: Symbol {
                id: 1,
                file_id: 1,
                name: "f".to_string(),
                qualified_name: format!("{path}::f"),
                kind: SymbolKind::Function,
                span: Span { start_byte: 0, end_byte: 0, start_line: 1, end_line: 1 },
                docstring: None,
                language: Language::Python,
                file_path: PathBuf::from(path),
            },
            file_path: PathBuf::from(path),
            snippet: String::new(),
            callers: vec![],
            callees: vec![],
        }
    }

    #[test]
    fn no_note_below_min_results() {
        let results = vec![fixture("a/x.py"), fixture("a/y.py")];
        assert_eq!(concentration_note(&results), None);
    }

    #[test]
    fn no_note_when_results_are_diverse() {
        let results = vec![
            fixture("a/x.py"),
            fixture("b/y.py"),
            fixture("c/z.py"),
            fixture("d/w.py"),
        ];
        assert_eq!(concentration_note(&results), None);
    }

    #[test]
    fn note_when_results_concentrate_in_one_directory() {
        let results = vec![
            fixture("work_order/a.py"),
            fixture("work_order/b.py"),
            fixture("work_order/c.py"),
            fixture("work_order/d.py"),
            fixture("stock_entry/e.py"),
        ];
        assert!(concentration_note(&results).is_some());
    }

    #[test]
    fn note_fires_across_a_module_root_file_and_its_services_submodule() {
        // The exact candidate pool from the real ERPNext Q05 transcript that
        // motivated this function: 8/10 results are all "the same module" —
        // manufacturing/doctype/work_order's root file plus its services/
        // submodule — even though they're 4 *different* immediate parent
        // directories once nesting is involved. Exact-parent grouping (the
        // first cut of this function) never fired on this real data; ancestor-
        // containment grouping does (8/10 = 0.8, at the threshold), since
        // work_order/ is an ancestor of work_order/services/.
        let results = vec![
            fixture("manufacturing/doctype/production_plan/production_plan.js"),
            fixture("manufacturing/doctype/work_order/work_order.py"),
            fixture("manufacturing/doctype/work_order/services/required_items.py"),
            fixture("manufacturing/doctype/work_order/services/required_items.py"),
            fixture("manufacturing/doctype/work_order/work_order.py"),
            fixture("manufacturing/doctype/job_card/job_card.py"),
            fixture("manufacturing/doctype/work_order/services/status.py"),
            fixture("manufacturing/doctype/work_order/services/required_items.py"),
            fixture("manufacturing/doctype/work_order/services/required_items.py"),
            fixture("manufacturing/doctype/work_order/work_order.py"),
        ];
        assert!(concentration_note(&results).is_some());
    }

    #[test]
    fn no_note_when_split_across_several_modules_even_if_one_leads() {
        // The real Q13 transcript's candidate pool: spread across work_order/,
        // sales_order/, and production_plan/ (max group 4/10 = 0.4). This is
        // genuine multi-module coverage of one topic, not a one-sided miss —
        // the note should not fire and mislead the agent into thinking the
        // tool missed something.
        let results = vec![
            fixture("public/js/stock_reservation.js"),
            fixture("manufacturing/doctype/work_order/services/reservation.py"),
            fixture("selling/doctype/sales_order/sales_order.py"),
            fixture("manufacturing/doctype/production_plan/services/reservation.py"),
            fixture("selling/doctype/sales_order/sales_order.js"),
            fixture("manufacturing/doctype/work_order/services/reservation.py"),
            fixture("selling/doctype/sales_order/services/reservation.py"),
            fixture("selling/doctype/sales_order/sales_order.py"),
            fixture("manufacturing/doctype/production_plan/production_plan.py"),
            fixture("manufacturing/doctype/production_plan/production_plan.js"),
        ];
        assert_eq!(concentration_note(&results), None);
    }

    #[test]
    fn absolute_paths_do_not_falsely_concentrate() {
        // Regression test for a real bug caught only by live-testing against
        // the rebuilt binary, not by these unit tests (which all used bare
        // relative fixtures): a depth-N-from-the-front grouping scheme fires
        // on ~100% of any batch when `file_path` is absolute, because the
        // first N components of an absolute path are the constant filesystem
        // prefix (`/Users/name/project/...`), identical across every result
        // regardless of actual content. This is exactly the real shape
        // `RetrievalResult::file_path` has in production — these results are
        // genuinely spread across four unrelated modules and must not fire.
        let results = vec![
            fixture("/Users/dev/project/repo/manufacturing/doctype/work_order/services/reservation.py"),
            fixture("/Users/dev/project/repo/selling/doctype/sales_order/sales_order.py"),
            fixture("/Users/dev/project/repo/selling/doctype/sales_order/sales_order.js"),
            fixture("/Users/dev/project/repo/manufacturing/doctype/production_plan/production_plan.py"),
            fixture("/Users/dev/project/repo/public/js/stock_reservation.js"),
        ];
        assert_eq!(concentration_note(&results), None);
    }

    #[test]
    fn threshold_is_not_tripped_by_an_even_split() {
        let results = vec![
            fixture("a/x.py"),
            fixture("a/y.py"),
            fixture("b/z.py"),
            fixture("b/w.py"),
        ];
        assert_eq!(concentration_note(&results), None);
    }
}
