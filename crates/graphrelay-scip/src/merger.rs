/// Plan 12 B3–B6, C1 — merge a parsed SCIP index into the store.
#[cfg(feature = "scip")]
use crate::mapper::scip_kind_to_symbol_kind;
#[cfg(feature = "scip")]
use graphrelay_core::types::EdgeKind;
#[cfg(feature = "scip")]
use graphrelay_store::{GraphDb, ScipEnrichment, ScipRef};

#[cfg(feature = "scip")]
use scip::types::{Index, symbol_information::Kind as ScipKind};

/// Statistics returned after a SCIP ingest.
#[derive(Debug, Default)]
pub struct ScipIngestStats {
    pub files_processed:    usize,
    pub symbols_enriched:   usize,
    pub edges_added:        usize,
    pub docstrings_updated: usize,
}

/// Constant: SCIP symbol_roles bit for DEFINITION occurrences.
#[cfg(feature = "scip")]
const ROLE_DEFINITION: i32 = 1;
/// Constant: SCIP symbol_roles bit for IMPORT occurrences.
#[cfg(feature = "scip")]
const ROLE_IMPORT: i32 = 2;

/// Plan 21 follow-up fix (item 11/SCIP investigation): `root` is the
/// project root `doc.relative_path` is relative to — needed to resolve each
/// document's `file_id` **once per document** via the indexed exact-match
/// `GraphDb::file_by_path`, instead of the old design implicitly doing an
/// unindexable `files.path LIKE '%' || relative_path` table scan on every
/// single *occurrence*.
/// That was a genuine O(occurrences × files) scan — measured, a direct
/// isolated call against erpnext's real 54MB `.scip` file did not complete
/// within 25+ minutes before this fix.
#[cfg(feature = "scip")]
pub fn merge_index(index: &Index, graph_db: &GraphDb, root: &std::path::Path) -> anyhow::Result<ScipIngestStats> {
    use std::collections::HashMap;

    let mut stats = ScipIngestStats::default();

    // ── Step 1: build a symbol-string → kind map from top-level SymbolInformation
    // (both document-level and external_symbols).
    let mut scip_kind_map: HashMap<&str, ScipKind> = HashMap::new();
    let mut scip_doc_map:  HashMap<&str, &str>     = HashMap::new();

    for doc in &index.documents {
        for si in &doc.symbols {
            if !si.symbol.is_empty() {
                let k = si.kind.enum_value_or(ScipKind::UnspecifiedKind);
                if k != ScipKind::UnspecifiedKind {
                    scip_kind_map.insert(&si.symbol, k);
                }
                if let Some(d) = si.documentation.first() {
                    if !d.is_empty() {
                        scip_doc_map.insert(&si.symbol, d.as_str());
                    }
                }
            }
        }
    }
    for si in &index.external_symbols {
        if !si.symbol.is_empty() {
            let k = si.kind.enum_value_or(ScipKind::UnspecifiedKind);
            if k != ScipKind::UnspecifiedKind {
                scip_kind_map.insert(&si.symbol, k);
            }
        }
    }

    // ── Step 2: classify every occurrence in memory. Nothing is written until
    // `apply_scip_merge`, which applies the whole ingest in one transaction
    // (Plan 24 item 1.7) — the previous design deleted all semantic edges up front
    // in autocommit mode and re-inserted at the end, so any failure in between
    // left the project with no semantic layer at all.
    let mut enrichments: Vec<ScipEnrichment> = Vec::new();
    let mut refs: Vec<ScipRef> = Vec::new();

    for doc in &index.documents {
        stats.files_processed += 1;
        let rel_path = &doc.relative_path;

        // Resolve file_id ONCE per document, not once per occurrence — see
        // this function's doc comment for why that distinction is the whole
        // fix. A document whose file isn't in our own index (e.g. SCIP saw
        // a file our own extractor's language/ignore filters skipped, or —
        // in these tests — an empty DB with nothing seeded at all) skips
        // just this document's occurrence processing; `files_processed`
        // still counts every document the .scip file itself covers.
        let Some(file_record) = graph_db.file_by_path(&root.join(rel_path))? else {
            continue;
        };
        let file_id = file_record.id;

        for occ in &doc.occurrences {
            if occ.range.is_empty() || occ.symbol.is_empty() { continue; }
            // `local N` ids are only meaningful inside one document; resolving
            // them through the global `scip_symbol` index linked unrelated
            // symbols across files (Plan 24 M1).
            if occ.symbol.starts_with("local ") { continue; }

            // SCIP range is 0-indexed, as is graphrelay-parser's `start_line`
            // (tree-sitter `start_position().row`, used as-is) — no conversion.
            let start_line = occ.range[0] as u32;
            let name = scip_symbol_display_name(&occ.symbol);
            if name.is_empty() { continue; }

            let is_definition = (occ.symbol_roles & ROLE_DEFINITION) != 0;
            let is_import     = (occ.symbol_roles & ROLE_IMPORT)     != 0;

            if is_definition {
                let scip_kind = scip_kind_map.get(occ.symbol.as_str())
                    .copied()
                    .unwrap_or(ScipKind::UnspecifiedKind);
                enrichments.push(ScipEnrichment {
                    file_id,
                    line: start_line,
                    name,
                    scip_symbol: occ.symbol.clone(),
                    kind: scip_kind_to_symbol_kind(scip_kind),
                    docstring: scip_doc_map.get(occ.symbol.as_str()).map(|d| d.to_string()),
                });
            } else if is_import {
                refs.push(ScipRef {
                    file_id,
                    line: start_line,
                    scip_symbol: occ.symbol.clone(),
                    to_name: name,
                    kind: EdgeKind::Imports,
                });
            } else if occ.symbol.ends_with(").") {
                // Only method/function descriptors are calls. Every other
                // reference (field reads, types, constants) used to become a
                // `Calls` edge too (Plan 24 M4).
                refs.push(ScipRef {
                    file_id,
                    line: start_line,
                    scip_symbol: occ.symbol.clone(),
                    to_name: name,
                    kind: EdgeKind::Calls,
                });
            }
        }
    }

    let counts = graph_db.apply_scip_merge(&enrichments, &refs)?;
    stats.symbols_enriched   = counts.enriched;
    stats.docstrings_updated = counts.docstrings;
    stats.edges_added        = counts.edges;

    Ok(stats)
}

/// Extract a human-readable display name from a SCIP symbol string.
/// SCIP format: "scip-python pip/requests 1.0 requests/Session#get()."
/// We take the last path component before the trailing `.`.
#[cfg(feature = "scip")]
fn scip_symbol_display_name(symbol: &str) -> String {
    let s = symbol.trim_end_matches('.');
    let last = s.rsplit(['/', '#']).next().unwrap_or(s);
    // Cut at the first '(' so overload descriptors (`foo(+2).`) yield `foo`,
    // not `foo(+2` — 51k keycloak semantic edges had such unresolvable names.
    let base = last.split('(').next().unwrap_or(last);
    base.trim_matches(['`', '.', ' ']).to_string()
}

#[cfg(test)]
#[cfg(feature = "scip")]
mod tests {
    use super::*;
    #[test]
    fn test_scip_symbol_display_name() {
        assert_eq!(scip_symbol_display_name("scip-python pip/req 1.0 req/Session#get()."), "get");
        assert_eq!(scip_symbol_display_name("scip-go go/fmt 1.0 fmt/Println()."), "Println");
        assert_eq!(scip_symbol_display_name("simple"), "simple");
        assert_eq!(scip_symbol_display_name("semanticdb maven/org.x/x 1.0 org/x/Foo#bar(+2)."), "bar");
        assert_eq!(scip_symbol_display_name("semanticdb maven/org.x/x 1.0 org/x/Foo#`<init>`()."), "<init>");
        assert_eq!(scip_symbol_display_name("scip-typescript npm x 1.0 `src/a.ts`/helper()."), "helper");
        assert_eq!(scip_symbol_display_name("semanticdb maven/org.x/x 1.0 org/x/Foo#field."), "field");
    }
}
