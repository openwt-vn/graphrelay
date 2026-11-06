//! Heuristic dynamic-dispatch edge synthesis — an additive post-pass run
//! after the existing 3-stage call resolver (and optional SCIP enrichment).
//! Never mutates or removes edges those stages produce; only adds new ones,
//! tagged `EdgeSource::Synthesized` so callers can tell an edge came from a
//! heuristic rather than a literal call-site expression.

pub mod c_fnptr;
pub mod js_callback;
pub mod py_self_calls;
pub mod py_string_dispatch;

use graphrelay_core::types::{Edge, Symbol};
use std::collections::HashMap;
use std::path::PathBuf;

/// Run every enabled synthesis pass and return the union of their edges,
/// deduplicated. `sources` must map each symbol's `file_path` to that file's
/// full text (the same bytes the symbols were extracted from) — passes work
/// from in-memory symbols/text only, no file I/O of their own, so they stay
/// pure and independently testable.
pub fn synthesize_all(symbols: &[Symbol], sources: &HashMap<PathBuf, String>) -> Vec<Edge> {
    let mut edges = c_fnptr::synthesize(symbols, sources);
    edges.extend(js_callback::synthesize(symbols, sources));
    edges.extend(py_string_dispatch::synthesize(symbols, sources));
    edges.sort_by(|a, b| a.from_id.cmp(&b.from_id).then(a.to_id.cmp(&b.to_id)));
    edges.dedup_by(|a, b| a.from_id == b.from_id && a.to_id == b.to_id);
    edges
}
