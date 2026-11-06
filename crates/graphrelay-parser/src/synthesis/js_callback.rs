//! Heuristic JS/TS string-keyed callback (`EventEmitter`-style) dispatch
//! synthesis.
//!
//! Recovers `emitter -> handler` call edges for the common Node.js/DOM
//! event-registration idiom:
//!
//! ```js
//! class Bus {
//!     constructor() { this.on('save', this.handleSave); }
//!     handleSave() { /* ... */ }
//!     trigger() { this.emit('save'); }
//! }
//! ```
//!
//! A plain tree-sitter call-expression query never sees an edge from
//! `trigger` to `handleSave` — there is no literal `handleSave(...)` call
//! site, only a string-keyed dispatch through the event name `'save'`. This
//! pass is a text-pattern heuristic, not a type checker: it extracts (a)
//! which functions/methods get registered against which event-name string
//! via `.on(...)`/`.addEventListener(...)`, then (b) links every `.emit(...)`
//! call site through that same event name to every handler ever registered
//! for it anywhere in the corpus.
//!
//! Known imprecision, by design (same posture as `c_fnptr.rs`): if two
//! unrelated emitters happen to use the same event-name string, an `.emit`
//! on either is linked to handlers registered on both — an
//! over-approximation, not a false structural miss. `.dispatchEvent(...)`
//! (which typically takes a constructed `Event`/`CustomEvent` object rather
//! than a bare string) and `.removeEventListener` are not handled — scoped to
//! the single most common `on`/`emit`/`addEventListener` triad.

use graphrelay_core::types::{Edge, EdgeKind, EdgeSource, Language, Symbol, SymbolId};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

fn registration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // .on('event', handler) / .addEventListener('event', this.handler)
    RE.get_or_init(|| {
        Regex::new(r#"\.(?:on|addEventListener)\s*\(\s*['"]([^'"]+)['"]\s*,\s*(?:this\.)?(\w+)\s*\)"#).unwrap()
    })
}

fn emission_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // .emit('event', ...) — trailing args (if any) are not needed here.
    RE.get_or_init(|| Regex::new(r#"\.emit\s*\(\s*['"]([^'"]+)['"]"#).unwrap())
}

fn is_js_like(lang: Language) -> bool {
    matches!(lang, Language::JavaScript | Language::TypeScript)
}

/// `name -> matching JS/TS symbols` index. Cheap (no source text touched) —
/// computed once from the corpus's full symbol set and reused across both
/// phases, and across every chunk when the caller processes files in bounded
/// batches (see `build_event_handlers`/`scan_emissions`).
pub fn js_functions_by_name(symbols: &[Symbol]) -> HashMap<&str, Vec<&Symbol>> {
    let mut functions_by_name: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    for sym in symbols {
        if is_js_like(sym.language) {
            functions_by_name.entry(sym.name.as_str()).or_default().push(sym);
        }
    }
    functions_by_name
}

/// Phase 1: scan `sources`' text for `.on`/`.addEventListener` registration
/// sites, resolving handler names against the full-corpus `functions_by_name`
/// index. `sources` may be any subset of the corpus's JS/TS files (e.g. one
/// bounded chunk) — see `c_fnptr::build_field_handlers`'s doc comment for why
/// this is split out from a single all-at-once pass (a real large JS/TS
/// corpus's full source text held in one `HashMap` risks the same
/// double-digit-GB memory blowup observed on `apple/swift`'s C++ files).
pub fn build_event_handlers(
    functions_by_name: &HashMap<&str, Vec<&Symbol>>,
    sources: &HashMap<PathBuf, String>,
) -> HashMap<String, HashSet<SymbolId>> {
    let mut event_handlers: HashMap<String, HashSet<SymbolId>> = HashMap::new();
    for source in sources.values() {
        for cap in registration_re().captures_iter(source) {
            let event = &cap[1];
            let handler_name = &cap[2];
            if let Some(candidates) = functions_by_name.get(handler_name) {
                let entry = event_handlers.entry(event.to_string()).or_default();
                for sym in candidates {
                    entry.insert(sym.id);
                }
            }
        }
    }
    event_handlers
}

/// Phase 2: call sites through `.emit(event, ...)`, scoped to each symbol's
/// own byte span so the edge's `from_id` is the enclosing function/method.
/// `event_handlers` must already be complete across the whole corpus (built
/// via one or more `build_event_handlers` calls) before this runs.
pub fn scan_emissions(
    symbols: &[Symbol],
    sources: &HashMap<PathBuf, String>,
    event_handlers: &HashMap<String, HashSet<SymbolId>>,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    if event_handlers.is_empty() {
        return edges;
    }

    for sym in symbols {
        if !is_js_like(sym.language) {
            continue;
        }
        let Some(source) = sources.get(&sym.file_path) else { continue };
        let start = sym.span.start_byte as usize;
        let end = (sym.span.end_byte as usize).min(source.len());
        if start >= source.len() || start >= end {
            continue;
        }
        let Some(body_text) = source.get(start..end) else { continue };

        for cap in emission_re().captures_iter(body_text) {
            let event = &cap[1];
            let Some(handlers) = event_handlers.get(event) else { continue };
            for &to_id in handlers {
                if to_id == sym.id {
                    continue; // skip trivial self-loop noise
                }
                edges.push(Edge {
                    from_id: sym.id,
                    to_id: Some(to_id),
                    to_name: None,
                    kind: EdgeKind::Calls,
                    source: EdgeSource::Synthesized,
                });
            }
        }
    }

    edges
}

/// Convenience single-shot API for small/synthetic inputs (tests, small
/// repos) where materializing every file's text in one `HashMap` is
/// harmless. Real large-repo callers should use `build_event_handlers` /
/// `scan_emissions` directly in bounded chunks instead — see their doc
/// comments.
pub fn synthesize(symbols: &[Symbol], sources: &HashMap<PathBuf, String>) -> Vec<Edge> {
    let functions_by_name = js_functions_by_name(symbols);
    if functions_by_name.is_empty() {
        return Vec::new();
    }
    let event_handlers = build_event_handlers(&functions_by_name, sources);
    if event_handlers.is_empty() {
        return Vec::new();
    }
    scan_emissions(symbols, sources, &event_handlers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphrelay_core::types::{Span, SymbolKind};
    use std::path::PathBuf;

    fn make_symbol(id: SymbolId, name: &str, source: &str, needle: &str, lang: Language) -> Symbol {
        let start = source.find(needle).expect("needle not found in fixture source");
        Symbol {
            id,
            file_id: 1,
            name: name.to_string(),
            qualified_name: format!("bus.js::{name}"),
            kind: SymbolKind::Method,
            span: Span {
                start_byte: start as u32,
                end_byte: (start + needle.len()) as u32,
                start_line: 0,
                end_line: 0,
            },
            docstring: None,
            language: lang,
            file_path: PathBuf::from("bus.js"),
        }
    }

    /// The exact scenario from this module's own header comment: a
    /// `.on('save', this.handleSave)` registration plus a `.emit('save')`
    /// call site inside a different method, with no literal
    /// `handleSave(...)` call anywhere.
    #[test]
    fn event_emitter_pattern_produces_synthesized_edge() {
        let source = r#"
class Bus {
    constructor() {
        this.on('save', this.handleSave);
    }

    handleSave() {
        console.log('saved');
    }

    trigger() {
        this.emit('save');
    }
}
"#
        .to_string();

        let trigger_needle = "trigger() {\n        this.emit('save');\n    }";
        let handle_save_needle = "handleSave() {\n        console.log('saved');\n    }";

        assert!(
            !trigger_needle.contains("handleSave("),
            "fixture invalid: trigger() must not contain a literal call to handleSave",
        );

        let trigger_sym = make_symbol(1, "trigger", &source, trigger_needle, Language::JavaScript);
        let handle_save_sym = make_symbol(2, "handleSave", &source, handle_save_needle, Language::JavaScript);
        let symbols = vec![trigger_sym.clone(), handle_save_sym.clone()];

        let mut sources = HashMap::new();
        sources.insert(PathBuf::from("bus.js"), source);

        let edges = synthesize(&symbols, &sources);
        assert!(
            edges.iter().any(|e| e.from_id == trigger_sym.id && e.to_id == Some(handle_save_sym.id)),
            "expected trigger->handleSave synthesized edge, got: {edges:?}",
        );
        assert!(edges.iter().all(|e| e.source == EdgeSource::Synthesized));
    }

    #[test]
    fn add_event_listener_pattern_produces_synthesized_edge() {
        let source = r#"
function onResize() {}
function bindHandlers() {
    window.addEventListener('resize', onResize);
}
function fireResize() {
    window.emit('resize');
}
"#
        .to_string();

        let on_resize_needle = "function onResize() {}";
        let fire_needle = "function fireResize() {\n    window.emit('resize');\n}";

        let on_resize_sym = make_symbol(10, "onResize", &source, on_resize_needle, Language::JavaScript);
        let fire_sym = make_symbol(11, "fireResize", &source, fire_needle, Language::JavaScript);
        let symbols = vec![on_resize_sym.clone(), fire_sym.clone()];

        let mut sources = HashMap::new();
        sources.insert(PathBuf::from("bus.js"), source);

        let edges = synthesize(&symbols, &sources);
        assert!(
            edges.iter().any(|e| e.from_id == fire_sym.id && e.to_id == Some(on_resize_sym.id)),
            "expected fireResize->onResize synthesized edge, got: {edges:?}",
        );
    }

    #[test]
    fn non_js_language_symbols_produce_no_edges() {
        let source = "fn trigger() { self.emit(\"save\"); }".to_string();
        let sym = Symbol {
            id: 1,
            file_id: 1,
            name: "trigger".to_string(),
            qualified_name: "bus.rs::trigger".to_string(),
            kind: SymbolKind::Function,
            span: Span { start_byte: 0, end_byte: source.len() as u32, start_line: 0, end_line: 0 },
            docstring: None,
            language: Language::Rust,
            file_path: PathBuf::from("bus.rs"),
        };
        let mut sources = HashMap::new();
        sources.insert(PathBuf::from("bus.rs"), source);
        let edges = synthesize(&[sym], &sources);
        assert!(edges.is_empty(), "synthesis pass must be a no-op for non-JS/TS symbols");
    }

    #[test]
    fn empty_input_returns_no_edges() {
        assert!(synthesize(&[], &HashMap::new()).is_empty());
    }

    /// Same regression coverage as `c_fnptr`'s
    /// `cross_chunk_field_handlers_merge_resolves_dispatch_across_files`: the
    /// registration site (`.on('save', handleSave)`) lives in one file/chunk,
    /// the emission site (`.emit('save')`) lives in a different file/chunk,
    /// and the two chunks' `event_handlers` maps are merged exactly like
    /// `run_dispatch_synthesis` does before `scan_emissions` runs.
    #[test]
    fn cross_chunk_event_handlers_merge_resolves_emission_across_files() {
        let bus_source = r#"
class Bus {
    constructor() {
        this.on('save', this.handleSave);
    }

    handleSave() {
        console.log('saved');
    }
}
"#
        .to_string();

        let trigger_source = r#"
function trigger() {
    bus.emit('save');
}
"#
        .to_string();

        let handle_save_needle = "handleSave() {\n        console.log('saved');\n    }";
        let handle_save_sym = make_symbol(1, "handleSave", &bus_source, handle_save_needle, Language::JavaScript);
        let trigger_needle = "function trigger() {\n    bus.emit('save');\n}";
        let mut trigger_sym = make_symbol(2, "trigger", &trigger_source, trigger_needle, Language::JavaScript);
        trigger_sym.file_path = PathBuf::from("trigger.js");

        let symbols = vec![handle_save_sym.clone(), trigger_sym.clone()];
        let functions_by_name = js_functions_by_name(&symbols);

        // Chunk A: only the file containing the `.on(...)` registration.
        let mut chunk_a_sources = HashMap::new();
        chunk_a_sources.insert(PathBuf::from("bus.js"), bus_source);
        let handlers_a = build_event_handlers(&functions_by_name, &chunk_a_sources);

        // Chunk B: only the file containing the `.emit(...)` call — no
        // registration site visible in this chunk at all.
        let mut chunk_b_sources = HashMap::new();
        chunk_b_sources.insert(PathBuf::from("trigger.js"), trigger_source);
        let handlers_b = build_event_handlers(&functions_by_name, &chunk_b_sources);
        assert!(
            handlers_b.is_empty(),
            "chunk B alone (emission-site file only) must find no handlers — proves the test isn't trivially passing",
        );

        let mut event_handlers: HashMap<String, HashSet<SymbolId>> = HashMap::new();
        for (event, ids) in handlers_a {
            event_handlers.entry(event).or_default().extend(ids);
        }
        for (event, ids) in handlers_b {
            event_handlers.entry(event).or_default().extend(ids);
        }

        let edges = scan_emissions(&[trigger_sym.clone()], &chunk_b_sources, &event_handlers);
        assert!(
            edges.iter().any(|e| e.from_id == trigger_sym.id && e.to_id == Some(handle_save_sym.id)),
            "expected trigger->handleSave edge resolved across chunks, got: {edges:?}",
        );
    }
}
