use crate::server::McpAppState;
use crate::tools::{
    is_possibly_stale, resolve_file_lookup, response_budget, truncate_snippet,
    truncate_snippet_on_line_boundary, PathLookup,
};
use anyhow::anyhow;
use graphrelay_core::retrieval::SourceLoader;
use graphrelay_core::types::Symbol;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Args {
    pub path:           String,
    pub focus_line:     Option<u32>,
    /// Plan 23: return the file's full text (line-numbered, budget-capped)
    /// instead of just the one-line-per-symbol `signature` list. Additive —
    /// default `false` leaves every existing caller's response unchanged.
    pub include_source: Option<bool>,
}

/// Plan 23: full file text as `"{1-based line}: {text}"`, matching `Read`'s
/// own line-numbering convention (as are every `lines` field and
/// `focus_line` since Plan 28 — they used to be tree-sitter's 0-based lines,
/// so `lines: "41-60"` sat next to source numbered from 42). Centers the window on `focus_line`
/// when given (expanding outward, alternating hi/lo, until the char budget
/// is exhausted); otherwise takes from the top of the file.
fn windowed_source(content: &str, focus_line: Option<u32>, max_chars: usize) -> (String, bool) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return (String::new(), false);
    }
    let Some(focus) = focus_line else {
        let numbered: String = lines.iter().enumerate()
            .map(|(i, l)| format!("{}: {}\n", i + 1, l))
            .collect();
        let (out, truncated) = truncate_snippet_on_line_boundary(&numbered, max_chars);
        return (out, truncated);
    };

    let numbered_line = |i: usize| format!("{}: {}\n", i + 1, lines[i]);
    let focus_idx = (focus as usize).saturating_sub(1).min(lines.len() - 1);

    let mut total_len = numbered_line(focus_idx).chars().count();
    let mut lo = focus_idx;
    let mut hi = focus_idx;
    loop {
        let mut expanded = false;
        if hi + 1 < lines.len() {
            let cand_len = numbered_line(hi + 1).chars().count();
            if total_len + cand_len <= max_chars {
                hi += 1;
                total_len += cand_len;
                expanded = true;
            }
        }
        if lo > 0 {
            let cand_len = numbered_line(lo - 1).chars().count();
            if total_len + cand_len <= max_chars {
                lo -= 1;
                total_len += cand_len;
                expanded = true;
            }
        }
        if !expanded {
            break;
        }
    }
    let truncated = lo > 0 || hi + 1 < lines.len();
    let text: String = (lo..=hi).map(numbered_line).collect();
    (text.trim_end_matches('\n').to_string(), truncated)
}

/// First non-blank line of a symbol's own span (its signature, or a leading
/// decorator/annotation line) — not the full body. A flat `symbols` list with
/// only name/kind/line-range and no way to tell what a symbol actually does
/// forces the caller into one Read/Grep per candidate to find out; a file
/// with dozens of symbols (verified on a real 93-symbol ERPNext file during
/// the Q23 benchmark investigation — the three functions the question needed
/// were present in the list but indistinguishable from 90 unrelated ones)
/// turns that into a multi-turn slog. This is a cheap enough signal to
/// include for every symbol without the response size blowing up the way a
/// full snippet per symbol would.
///
/// Returns `None` (never errors) if the source file isn't readable at query
/// time — a stale index entry or an already-deleted file must not fail the
/// whole `get_file_context` call over one symbol's missing source.
fn signature_line(source_loader: &SourceLoader, s: &Symbol) -> Option<String> {
    let snippet = source_loader.snippet(s).ok()?;
    first_nonblank(snippet.lines())
}

/// `signature_line` over a file already split into lines, so a file's whole
/// symbol list costs one split instead of one per symbol.
fn signature_from_lines(lines: &[&str], s: &Symbol) -> Option<String> {
    let start = s.span.start_line as usize;
    let end = (s.span.end_line as usize).saturating_add(1).min(lines.len());
    first_nonblank(lines.get(start..end)?.iter().copied())
}

fn first_nonblank<'a>(mut lines: impl Iterator<Item = &'a str>) -> Option<String> {
    let line = lines.find(|l| !l.trim().is_empty())?.trim();
    Some(truncate_snippet(line, 120))
}

/// Cap on the per-symbol list: Plan 23's budget covered only `source`, so a
/// file with thousands of symbols (generated code) produced an unbounded list
/// on top of it. With `include_source` the text already shows every symbol,
/// so the list is tighter there.
const MAX_LISTED_SYMBOLS: usize = 200;
const MAX_LISTED_SYMBOLS_WITH_SOURCE: usize = 50;

/// Thin wrapper over the shared `resolve_file_lookup` (also used by
/// `inspect` to decide whether a `path` argument means "this file" or "this
/// module") that turns its `NotAFile` case into this tool's own terminal
/// error — unlike `inspect`, `get_file_context` has no directory-summary
/// fallback to try next, so "not a file" really is the end of the line here.
fn resolve_file(state: &McpAppState, path: &str) -> anyhow::Result<graphrelay_core::types::FileRecord> {
    match resolve_file_lookup(state, path)? {
        PathLookup::File(f) => Ok(f),
        PathLookup::Ambiguous(e) => Err(e),
        PathLookup::NotAFile => Err(anyhow!(
            "file not indexed: {path} (tried an exact match and a relative-suffix match; \
             pass the absolute `file` path from a search_code result, or double-check \
             the path is under the indexed project root)"
        )),
    }
}

pub async fn handle(state: &McpAppState, args: Args) -> anyhow::Result<Value> {
    tracing::info!(
        path = %args.path, focus_line = ?args.focus_line,
        include_source = ?args.include_source, "get_file_context"
    );
    let file = resolve_file(state, &args.path)?;

    let mut syms = state.graph_db.symbols_in_file(file.id)?;
    syms.sort_by_key(|s| (s.span.start_line, s.span.end_line, s.id));
    let content = state.source_loader.full_text(&file.path);
    let file_lines: Vec<&str> = content.as_deref().map(|c| c.lines().collect()).unwrap_or_default();
    let signature = |s: &Symbol| match &content {
        Ok(_) => signature_from_lines(&file_lines, s),
        Err(_) => signature_line(&state.source_loader, s),
    };
    let describe = |s: &Symbol| serde_json::json!({
        "name":      s.qualified_name,
        "kind":      format!("{:?}", s.kind),
        "lines":     crate::tools::line_range(&s.span),
        "signature": signature(s),
    });

    let focused = args.focus_line.and_then(|line| {
        // `focus_line` is 1-based (like `Read` and `source`); spans are 0-based.
        let line = line.checked_sub(1)?;
        // Pick the innermost (smallest) enclosing span, not the first one returned —
        // otherwise a containing class/module symbol can be reported instead of the
        // specific method/function at this line.
        syms.iter()
            .filter(|s| s.span.start_line <= line && line <= s.span.end_line)
            .min_by_key(|s| s.span.end_line.saturating_sub(s.span.start_line))
            .map(describe)
    });

    let include_source = args.include_source.unwrap_or(false);
    let cap = if include_source { MAX_LISTED_SYMBOLS_WITH_SOURCE } else { MAX_LISTED_SYMBOLS };
    let mut out = serde_json::json!({
        "path":           file.path.to_string_lossy(),
        "language":       file.language.as_str(),
        "symbol_count":   syms.len(),
        "focused_symbol": focused,
        "symbols":        syms.iter().take(cap).map(describe).collect::<Vec<_>>(),
    });
    if syms.len() > cap {
        let obj = out.as_object_mut().expect("json! built an object");
        obj.insert("symbols_truncated".to_string(), serde_json::json!(true));
    }

    // Plan 23: opt-in full-file-text mode, additive — only touches the
    // response when explicitly requested.
    if include_source {
        match &content {
            Ok(content) => {
                let total_files = state.watch_status.read().await.total_files;
                let budget = response_budget(total_files);
                let total_lines = content.lines().count();
                let (source, source_truncated) =
                    windowed_source(content, args.focus_line, budget.single_file_chars);
                let obj = out.as_object_mut().expect("json! built an object");
                obj.insert("source".to_string(), serde_json::json!(source));
                obj.insert("source_truncated".to_string(), serde_json::json!(source_truncated));
                obj.insert("total_lines".to_string(), serde_json::json!(total_lines));
            }
            Err(e) => {
                tracing::warn!(path = %file.path.display(), "full_text load failed for include_source: {e}");
            }
        }
    }

    // Plan 22 item 2: whole file, emitted only when true.
    if is_possibly_stale(&state.graph_db, &file.path) {
        let obj = out.as_object_mut().expect("json! built an object");
        obj.insert("possibly_stale".to_string(), serde_json::json!(true));
        obj.insert("note".to_string(), serde_json::json!(
            "This file may have changed since it was indexed (recent edit still processing). \
             Read it directly before relying on `symbols`/`lines`/`signature`."
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod windowed_source_tests {
    use super::*;

    #[test]
    fn no_focus_line_takes_from_the_top_with_1_based_numbering() {
        let content = "a\nb\nc\n";
        let (out, truncated) = windowed_source(content, None, 1000);
        assert_eq!(out, "1: a\n2: b\n3: c");
        assert!(!truncated);
    }

    #[test]
    fn no_focus_line_truncates_on_a_line_boundary_when_over_budget() {
        let content = "a\nb\nc\nd\n";
        let (out, truncated) = windowed_source(content, None, 10);
        assert!(truncated);
        assert_eq!(out, "1: a\n2: b");
    }

    #[test]
    fn focus_line_centers_the_window_when_budget_allows_the_whole_file() {
        let content = "a\nb\nc\nd\ne\n";
        let (out, truncated) = windowed_source(content, Some(3), 1000);
        assert!(!truncated);
        assert_eq!(out, "1: a\n2: b\n3: c\n4: d\n5: e");
    }

    #[test]
    fn focus_line_centers_the_window_when_budget_is_tight() {
        let content = "a\nb\nc\nd\ne\n";
        // Budget only fits 3 lines' worth — must include the focus line (3)
        // and expand outward, not just take from the top.
        let (out, truncated) = windowed_source(content, Some(3), 15);
        assert!(truncated);
        assert!(out.contains("3: c"));
    }

    #[test]
    fn focus_line_past_end_of_file_clamps_to_last_line() {
        let content = "a\nb\n";
        let (out, _) = windowed_source(content, Some(99), 1000);
        assert!(out.contains("2: b"));
    }

    #[test]
    fn empty_file_returns_empty_not_truncated() {
        let (out, truncated) = windowed_source("", None, 1000);
        assert_eq!(out, "");
        assert!(!truncated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphrelay_core::types::{Language, Span, SymbolKind};
    use std::io::Write;

    fn temp_symbol(dir: &std::path::Path, filename: &str, contents: &str, start_line: u32, end_line: u32) -> Symbol {
        let path = dir.join(filename);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        Symbol {
            id: 1,
            file_id: 1,
            name: "f".to_string(),
            qualified_name: format!("{filename}::f"),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: contents.len() as u32, start_line, end_line },
            docstring: None,
            language: Language::Python,
            file_path: path,
        }
    }

    #[test]
    fn signature_line_returns_first_nonblank_line_of_span() {
        let dir = std::env::temp_dir().join(format!("cg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let contents = "def set_previous_operation_serial_batch(row):\n    pass\n";
        let sym = temp_symbol(&dir, "sig1.py", contents, 0, 1);

        let loader = SourceLoader::new();
        let sig = signature_line(&loader, &sym);
        assert_eq!(sig.as_deref(), Some("def set_previous_operation_serial_batch(row):"));
    }

    #[test]
    fn signature_line_skips_leading_blank_lines() {
        let dir = std::env::temp_dir().join(format!("cg-test-{}", std::process::id() + 1));
        std::fs::create_dir_all(&dir).unwrap();
        let contents = "\n\ndef real_start():\n    pass\n";
        let sym = temp_symbol(&dir, "sig2.py", contents, 0, 3);

        let loader = SourceLoader::new();
        let sig = signature_line(&loader, &sym);
        assert_eq!(sig.as_deref(), Some("def real_start():"));
    }

    #[test]
    fn signature_line_none_when_source_unreadable() {
        // Nonexistent file — must degrade gracefully, not panic or error the
        // whole get_file_context call over one symbol's missing source.
        let sym = Symbol {
            id: 1,
            file_id: 1,
            name: "f".to_string(),
            qualified_name: "gone.py::f".to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: 0, start_line: 1, end_line: 1 },
            docstring: None,
            language: Language::Python,
            file_path: std::path::PathBuf::from("/nonexistent/path/gone.py"),
        };
        let loader = SourceLoader::new();
        assert_eq!(signature_line(&loader, &sym), None);
    }

    #[test]
    fn signature_line_truncates_long_lines() {
        let dir = std::env::temp_dir().join(format!("cg-test-{}", std::process::id() + 2));
        std::fs::create_dir_all(&dir).unwrap();
        let long_args = "a".repeat(200);
        let contents = format!("def f({long_args}):\n    pass\n");
        let sym = temp_symbol(&dir, "sig3.py", &contents, 0, 1);

        let loader = SourceLoader::new();
        let sig = signature_line(&loader, &sym).unwrap();
        assert!(sig.chars().count() <= 121); // 120 + the truncation ellipsis
        assert!(sig.ends_with('…'));
    }
}
