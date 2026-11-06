use crate::types::{Chunk, GraphMeta, Symbol, SymbolKind};

pub struct ChunkBuilder;

impl ChunkBuilder {
    /// Build a text chunk carrying structural + class context as a prefix.
    ///
    /// Format:
    ///   [file: src/query/engine.rs | symbol: QueryEngine::execute | kind: Function]
    ///   [class: QueryEngine | siblings: search, validate, reset]   ← Improvement #3
    ///   [callers: search_handler, route_handler]
    ///   [callees: GraphDb::neighbors, GraphStore::lexical_search, rerank]
    ///   <docstring if present>
    ///   <symbol body — truncated to max_chars>
    pub fn build(
        symbol:    &Symbol,
        body:      &str,
        callers:   &[String],
        callees:   &[String],
        max_chars: usize,
    ) -> Chunk {
        Self::build_with_class(symbol, body, callers, callees, max_chars, None, &[])
    }

    /// Extended build that includes the enclosing class name and sibling method names.
    /// Called from IndexWorker when file-level symbol context is available.
    pub fn build_with_class(
        symbol:       &Symbol,
        body:         &str,
        callers:      &[String],
        callees:      &[String],
        max_chars:    usize,
        class_name:   Option<&str>,
        siblings:     &[&str],
    ) -> Chunk {
        let mut parts = Vec::new();

        // Structural prefix
        parts.push(format!(
            "[file: {} | symbol: {} | kind: {:?}]",
            symbol.file_path.display(),
            symbol.qualified_name,
            symbol.kind,
        ));

        if !callers.is_empty() {
            let names: Vec<_> = callers.iter().take(5).map(String::as_str).collect();
            parts.push(format!("[callers: {}]", names.join(", ")));
        }
        if !callees.is_empty() {
            let names: Vec<_> = callees.iter().take(5).map(String::as_str).collect();
            parts.push(format!("[callees: {}]", names.join(", ")));
        }

        if let Some(doc) = &symbol.docstring {
            parts.push(doc.chars().take(300).collect::<String>());
        }

        // Body — fill remaining budget (class context placed AFTER body so FTS5's
        // bm25() weighs the function body's own vocabulary over structural metadata)
        let prefix = parts.join("\n");
        let body_budget = max_chars.saturating_sub(prefix.len() + 200); // reserve 200 for class ctx
        let body_truncated: String = body.chars().take(body_budget).collect();
        parts.push(body_truncated);

        // Improvement #3: class context appended AFTER body — structural hint without
        // displacing the primary function-purpose signal from the indexed text
        if let Some(cls) = class_name {
            if siblings.is_empty() {
                parts.push(format!("[class: {}]", cls));
            } else {
                // Only include class name + count — listing all siblings introduces noise
                parts.push(format!("[class: {} | {}_methods_total]", cls, siblings.len()));
            }
        }

        let text = parts.join("\n");

        Chunk {
            symbol_id: symbol.id,
            text,
            graph_meta: GraphMeta {
                qualified_name: symbol.qualified_name.clone(),
                file_path:      symbol.file_path.clone(),
                caller_names:   callers.to_vec(),
                callee_names:   callees.to_vec(),
            },
        }
    }
}

/// Find the enclosing class symbol for a given function symbol within a file's symbol list.
/// Returns `(class_name, sibling_method_names)`.
pub fn find_class_context<'a>(
    symbol:       &Symbol,
    file_symbols: &'a [Symbol],
) -> (Option<&'a str>, Vec<&'a str>) {
    // Find the smallest class/struct/interface whose span contains this symbol's span.
    let class_kinds = [
        SymbolKind::Class, SymbolKind::Struct, SymbolKind::Interface, SymbolKind::Trait,
    ];
    let method_kinds = [SymbolKind::Function, SymbolKind::Method];

    let enclosing = file_symbols.iter()
        .filter(|s| class_kinds.contains(&s.kind))
        .filter(|s| {
            s.span.start_byte <= symbol.span.start_byte
                && s.span.end_byte >= symbol.span.end_byte
                && s.id != symbol.id
        })
        // Pick the narrowest enclosing class
        .min_by_key(|s| s.span.end_byte - s.span.start_byte);

    if let Some(cls) = enclosing {
        // Collect sibling methods (other functions inside the same class, excluding self)
        let siblings: Vec<&str> = file_symbols.iter()
            .filter(|s| method_kinds.contains(&s.kind))
            .filter(|s| s.id != symbol.id)
            .filter(|s| {
                s.span.start_byte >= cls.span.start_byte
                    && s.span.end_byte <= cls.span.end_byte
            })
            .map(|s| s.name.as_str())
            .take(10)
            .collect();
        (Some(cls.name.as_str()), siblings)
    } else {
        (None, vec![])
    }
}
