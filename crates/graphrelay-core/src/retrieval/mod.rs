use crate::types::{Symbol, SymbolRef};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use pipeline::{tokenize_name, QueryPipeline, RetrievalPipeline};
pub use source_loader::SourceLoader;

pub mod pipeline;
pub mod source_loader;

#[derive(Debug, Clone)]
pub struct RetrievalQuery {
    pub text:        String,
    pub top_k:       usize,
    pub final_n:     usize,
    pub expand_hops: u8,
    pub lang_filter: Option<String>,
}

impl Default for RetrievalQuery {
    fn default() -> Self {
        Self {
            text:        String::new(),
            top_k:       20,
            final_n:     5,
            expand_hops: 1,
            lang_filter: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub rank:      usize,
    pub score:     f32,
    pub symbol:    Symbol,
    pub file_path: PathBuf,
    pub snippet:   String,
    pub callers:   Vec<SymbolRef>,
    pub callees:   Vec<SymbolRef>,
}

/// Internal candidate before final scoring
#[derive(Debug)]
pub struct EnrichedCandidate {
    pub symbol:        Symbol,
    /// Normalized [0,1] score from the SQLite FTS5 lexical candidate-generation
    /// search (`GraphStore::lexical_search`), 0.0 if this candidate was found
    /// only via graph expansion. Merged with the live snippet-BM25 signal in
    /// `rerank()` — see `combined_keyword` there.
    pub lexical_score: f32,
    /// Personalized PageRank mass (`pipeline::personalized_pagerank`), seeded
    /// from this query's own direct hits and propagated through the call
    /// graph. Unlike raw in-degree (a global, query-independent popularity
    /// measure — a function called from everywhere scores the same
    /// regardless of what was searched for), this is query-relevant by
    /// construction: it's only ever nonzero because *this* query's seeds
    /// propagated mass to it. Used in `rerank()`'s weighted branch in place of
    /// in-degree — see `checklist.md` Plan 16.
    pub graph_score:   f32,
    pub snippet:       String,
    /// True when this candidate came directly from the lexical search stage
    /// rather than from graph expansion (1-hop neighbor of a direct hit).
    /// Used in reranking: direct hits keep
    /// their own retrieval score as the primary signal; expanded neighbors are
    /// re-scored using `graph_score` instead, since they have no retrieval
    /// score of their own — they were only pulled in via a relevant neighbor.
    pub is_direct_hit: bool,
    /// Plan 22 item 1: true when `SourceLoader::is_generated` judged this
    /// candidate's file to be machine-generated (filename convention or a
    /// banner in its leading comment block). Used in `rerank()` to apply
    /// `GENERATED_FILE_PENALTY`, gated off when the query is itself about
    /// generated code (`query_mentions_generated_code`) or the candidate is
    /// pinned (Plan 22 item 3).
    pub is_generated:  bool,
}
