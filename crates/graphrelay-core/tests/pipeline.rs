use graphrelay_core::{
    relevance_judge::RelevanceJudge,
    retrieval::{
        pipeline::{GraphStore, RetrievalPipeline},
        RetrievalQuery, SourceLoader,
    },
    types::{Language, Span, Symbol, SymbolId, SymbolKind},
    config::{PathPinMode, RetrievalConfig},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

// ── Mock implementation ────────────────────────────────────────────────────────

struct MockGraph {
    symbols:      HashMap<SymbolId, Symbol>,
    callers:      HashMap<SymbolId, Vec<Symbol>>,
    callees:      HashMap<SymbolId, Vec<Symbol>>,
    lexical_hits: Vec<(SymbolId, f32)>,
    /// Plan 22 item 3: mention text -> resolved file path(s), mirroring
    /// `GraphStore::resolve_pinned_files`.
    pinned_files:   HashMap<String, Vec<String>>,
    /// file path -> (SymbolId, score) pairs, mirroring
    /// `GraphStore::symbols_for_path_pin`.
    pinned_symbols: HashMap<String, Vec<(SymbolId, f32)>>,
}

impl MockGraph {
    fn new() -> Self {
        Self {
            symbols:      HashMap::new(),
            callers:      HashMap::new(),
            callees:      HashMap::new(),
            lexical_hits: Vec::new(),
            pinned_files:   HashMap::new(),
            pinned_symbols: HashMap::new(),
        }
    }

    fn add(&mut self, s: Symbol) {
        self.symbols.insert(s.id, s);
    }

    fn add_lexical_hit(&mut self, id: SymbolId, score: f32) {
        self.lexical_hits.push((id, score));
    }

    fn set_pinned_files(&mut self, mention: &str, files: Vec<String>) {
        self.pinned_files.insert(mention.to_string(), files);
    }

    fn set_pinned_symbols(&mut self, file_path: &str, symbols: Vec<(SymbolId, f32)>) {
        self.pinned_symbols.insert(file_path.to_string(), symbols);
    }

    fn add_call(&mut self, caller: Symbol, callee: Symbol) {
        let caller_id = caller.id;
        let callee_id = callee.id;
        self.add(caller.clone());
        self.add(callee.clone());
        self.callers.entry(callee_id).or_default().push(caller.clone());
        self.callees.entry(caller_id).or_default().push(callee);
        // ensure caller is in symbols even if not added separately
        self.symbols.entry(caller_id).or_insert(caller);
    }
}

impl GraphStore for MockGraph {
    fn symbol_by_id(&self, id: SymbolId) -> anyhow::Result<Option<Symbol>> {
        Ok(self.symbols.get(&id).cloned())
    }
    fn callers_of(&self, id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        Ok(self.callers.get(&id).cloned().unwrap_or_default())
    }
    fn callees_of(&self, id: SymbolId) -> anyhow::Result<Vec<Symbol>> {
        Ok(self.callees.get(&id).cloned().unwrap_or_default())
    }
    fn neighbors(&self, id: SymbolId, _hops: u8) -> anyhow::Result<Vec<Symbol>> {
        let mut out = self.callers.get(&id).cloned().unwrap_or_default();
        out.extend(self.callees.get(&id).cloned().unwrap_or_default());
        Ok(out)
    }
    fn in_degree(&self, id: SymbolId) -> anyhow::Result<u32> {
        Ok(self.callers.get(&id).map(|v| v.len() as u32).unwrap_or(0))
    }
    fn lexical_search(&self, terms: &[String], _synonym_terms: &[String], top_k: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        if terms.is_empty() {
            return Ok(vec![]);
        }
        Ok(self.lexical_hits.iter().take(top_k).cloned().collect())
    }
    fn resolve_pinned_files(&self, path_text: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.pinned_files.get(path_text).cloned().unwrap_or_default())
    }
    fn symbols_for_path_pin(&self, file_path: &str, _remaining_terms: &[String], max_symbols: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        Ok(self.pinned_symbols.get(file_path).cloned().unwrap_or_default().into_iter().take(max_symbols).collect())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_symbol(id: u64, name: &str) -> Symbol {
    Symbol {
        id,
        file_id:        0,
        name:           name.to_string(),
        qualified_name: format!("test.rs::{name}"),
        kind:           SymbolKind::Function,
        span:           Span { start_byte: 0, end_byte: 50, start_line: 0, end_line: 5 },
        docstring:      None,
        language:       Language::Rust,
        file_path:      PathBuf::from("test.rs"),
    }
}

fn pipeline(graph: MockGraph) -> RetrievalPipeline<MockGraph> {
    RetrievalPipeline {
        graph_db:      Arc::new(graph),
        source_loader: Arc::new(SourceLoader::new()),
        config:        RetrievalConfig::default(),
        jev:           None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
// Candidate generation is lexical-only (SQLite FTS5 + graph expansion) — see
// `plans/20-remove-vector-search.md`. These tests seed the pool via
// `MockGraph::add_lexical_hit` (mirroring `GraphStore::lexical_search`)
// instead of a vector-search oracle.

#[tokio::test]
async fn empty_lexical_search_returns_empty() {
    let mut g = MockGraph::new();
    g.add(make_symbol(1, "fn_a"));
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "find something".to_string(),
        top_k:       5,
        final_n:     3,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert!(results.is_empty());
}

/// Plan 21 item 6: `RetrievalQuery.lang_filter` was parsed by the
/// `find_relevant_code` tool and threaded all the way down to
/// `RetrievalQuery`, but `query()` never actually read it — a caller asking
/// for e.g. Python-only results silently got every language back. This pins
/// that it now actually filters the pool.
#[tokio::test]
async fn lang_filter_restricts_results_to_the_requested_language() {
    let mut py_sym = make_symbol(1, "py_fn");
    py_sym.language = Language::Python;
    let mut js_sym = make_symbol(2, "js_fn");
    js_sym.language = Language::JavaScript;

    let mut g = MockGraph::new();
    g.add(py_sym);
    g.add(js_sym);
    g.add_lexical_hit(1, 0.9);
    g.add_lexical_hit(2, 0.8);
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "function".to_string(),
        top_k:       5,
        final_n:     5,
        expand_hops: 0,
        lang_filter: Some("python".to_string()),
    }).await.unwrap();

    assert_eq!(results.len(), 1, "only the Python symbol should survive the filter");
    assert_eq!(results[0].symbol.id, 1);
}

/// Other-language hits must not use up the lexical budget before the filter
/// runs: with `top_k = 2` and two stronger JavaScript hits, the Python symbol
/// used to be cut before the filter ever saw it.
#[tokio::test]
async fn lang_filter_is_applied_before_the_candidate_budget_is_spent() {
    let mut g = MockGraph::new();
    for (id, lang, score) in [(1, Language::JavaScript, 0.9), (2, Language::JavaScript, 0.8), (3, Language::Python, 0.7)] {
        let mut sym = make_symbol(id, &format!("fn_{id}"));
        sym.language = lang;
        g.add(sym);
        g.add_lexical_hit(id, score);
    }
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "function".to_string(),
        top_k:       2,
        final_n:     5,
        expand_hops: 0,
        lang_filter: Some("python".to_string()),
    }).await.unwrap();

    let ids: Vec<u64> = results.iter().map(|r| r.symbol.id).collect();
    assert_eq!(ids, vec![3]);
}

#[tokio::test]
async fn single_hit_returns_one_result() {
    let sym = make_symbol(42, "compute");
    let mut g = MockGraph::new();
    g.add(sym.clone());
    g.add_lexical_hit(42, 0.9);
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "compute".to_string(),
        top_k:       5,
        final_n:     5,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].symbol.id, 42);
    assert_eq!(results[0].rank, 1);
}

#[tokio::test]
async fn results_are_limited_to_final_n() {
    let mut g = MockGraph::new();
    for i in 1u64..=10 {
        g.add(make_symbol(i, &format!("fn_{i}")));
        g.add_lexical_hit(i, 0.9 - (i as f32 * 0.05));
    }
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "query".to_string(),
        top_k:       20,
        final_n:     3,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn higher_score_ranks_first() {
    let mut g = MockGraph::new();
    g.add(make_symbol(10, "low_score"));
    g.add(make_symbol(11, "high_score"));
    g.add_lexical_hit(10, 0.3);
    g.add_lexical_hit(11, 0.9);
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "test".to_string(),
        top_k:       10,
        final_n:     2,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].symbol.id, 11, "high_score should rank first");
    assert_eq!(results[1].symbol.id, 10);
}

#[tokio::test]
async fn result_ranks_are_sequential() {
    let mut g = MockGraph::new();
    for i in 1u64..=5 {
        g.add(make_symbol(i, &format!("fn_{i}")));
        g.add_lexical_hit(i, 1.0 - (i as f32 * 0.1));
    }
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "query".to_string(),
        top_k:       10,
        final_n:     5,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    for (i, r) in results.iter().enumerate() {
        assert_eq!(r.rank, i + 1, "rank should be {}, got {}", i + 1, r.rank);
    }
}

#[tokio::test]
async fn callers_included_in_result() {
    let callee = make_symbol(20, "callee");
    let caller = make_symbol(21, "caller");
    let mut g = MockGraph::new();
    g.add_call(caller, callee);
    g.add_lexical_hit(20, 0.95);

    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "callee".to_string(),
        top_k:       5,
        final_n:     1,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].callers.len(), 1, "callee should have one caller");
    assert_eq!(results[0].callers[0].id, 21);
}

#[tokio::test]
async fn unknown_symbol_id_in_hits_is_skipped() {
    let mut g = MockGraph::new(); // no symbols registered
    g.add_lexical_hit(999, 0.9);
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "query".to_string(),
        top_k:       5,
        final_n:     5,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert!(results.is_empty(), "hits for unknown symbol IDs should be silently skipped");
}

#[tokio::test]
async fn direct_hits_rank_by_keyword_strength() {
    let mut g = MockGraph::new();
    g.add(make_symbol(30, "handlerA"));
    g.add(make_symbol(31, "handlerB"));
    g.add_lexical_hit(30, 0.9); // strong keyword match
    g.add_lexical_hit(31, 0.1); // weak keyword match
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "parse configuration file".to_string(),
        top_k:       10,
        final_n:     2,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].symbol.id, 30,
        "the stronger lexical/keyword match should outrank the weaker one among direct hits");
}

#[tokio::test]
async fn tied_scores_break_ties_deterministically_by_symbol_id() {
    let mut g = MockGraph::new();
    g.add(make_symbol(100, "fn_a"));
    g.add(make_symbol(5, "fn_b"));
    g.add(make_symbol(50, "fn_c"));
    // No keyword/name overlap with the query for any of them, and identical
    // lexical score — every candidate ties exactly on final score.
    g.add_lexical_hit(100, 0.5);
    g.add_lexical_hit(5, 0.5);
    g.add_lexical_hit(50, 0.5);
    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "xyz".to_string(),
        top_k:       10,
        final_n:     3,
        expand_hops: 0,
        lang_filter: None,
    }).await.unwrap();

    assert_eq!(
        results.iter().map(|r| r.symbol.id).collect::<Vec<_>>(),
        vec![5, 50, 100],
        "tied scores must resolve deterministically by ascending SymbolId, not by \
         HashSet iteration order (randomized per process in Rust)"
    );
}

// ── Personalized PageRank (graph-centrality is query-relevant, not global) ──

#[tokio::test]
async fn graph_centrality_reflects_query_relevance_not_raw_global_popularity() {
    let mut g = MockGraph::new();
    let seed     = make_symbol(70, "queryTarget");
    let relevant = make_symbol(71, "relevantNeighbor");
    let hub      = make_symbol(72, "globallyPopularHub");
    g.add(seed.clone());
    // `relevant` and `hub` are both exactly one hop from `seed` (its callees),
    // so graph expansion decay treats them identically.
    g.add_call(seed.clone(), relevant.clone());
    g.add_call(seed.clone(), hub.clone());
    // Inflate `hub`'s GLOBAL in-degree with 20 callers that have nothing to
    // do with this query or its seed — exactly what raw in-degree used to
    // reward regardless of relevance, and what personalized PageRank should
    // not, since none of this mass originates from `seed`.
    for i in 0..20 {
        g.add_call(make_symbol(1000 + i, &format!("unrelatedCaller{i}")), hub.clone());
    }
    g.add_lexical_hit(70, 0.9); // seed is the only direct hit

    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "query target".to_string(),
        top_k:       10,
        final_n:     10,
        expand_hops: 1,
        lang_filter: None,
    }).await.unwrap();

    let score_of = |id: u64| results.iter().find(|r| r.symbol.id == id).map(|r| r.score);
    let relevant_score = score_of(71).expect("relevant neighbor should be retrieved");
    let hub_score = score_of(72).expect("hub neighbor should be retrieved");

    // Both are equidistant from the same seed with identical decayed lexical
    // score and zero name/keyword overlap, so the *only* thing that can
    // separate them is the graph-centrality term. Under a raw-in-degree
    // scheme `hub` (21 total callers) would have dominated `relevant` (1
    // caller) by roughly in_degree_norm 1.0 vs ~0.05. Personalized PageRank
    // only propagates mass that actually originates from `seed`, so `hub`'s
    // 20 irrelevant callers must not let it outscore `relevant`.
    assert!(
        (relevant_score - hub_score).abs() < 1e-4,
        "hub's 20 irrelevant callers are never traversed by a callees-only directed walk, \
         so it should score identically to `relevant` (both are plain callees of the same \
         seed with no callees of their own) — hub_score={hub_score}, relevant_score={relevant_score}"
    );
}

#[tokio::test]
async fn graph_centrality_rewards_an_actual_multi_hop_relevant_path() {
    let mut g = MockGraph::new();
    let seed       = make_symbol(80, "queryTarget");
    let deep_chain = make_symbol(81, "deeplyRelevant");
    let dead_end   = make_symbol(82, "unrelatedDeadEnd");
    g.add(seed.clone());
    // `deep_chain` gets reinforced: seed calls it directly, AND it's also
    // called by something seed calls — two independent paths of mass from
    // the same seed. `dead_end` is a plain one-hop callee with nothing
    // feeding into it beyond that single edge.
    let intermediate = make_symbol(83, "intermediate");
    g.add_call(seed.clone(), intermediate.clone());
    g.add_call(seed.clone(), deep_chain.clone());
    g.add_call(intermediate, deep_chain.clone());
    g.add_call(seed.clone(), dead_end.clone());
    g.add_lexical_hit(80, 0.9);

    let p = pipeline(g);

    let results = p.query(RetrievalQuery {
        text:        "query target".to_string(),
        top_k:       10,
        final_n:     10,
        expand_hops: 1,
        lang_filter: None,
    }).await.unwrap();

    let score_of = |id: u64| results.iter().find(|r| r.symbol.id == id).map(|r| r.score);
    let deep_score = score_of(81).expect("deeply relevant node should be retrieved");
    let dead_end_score = score_of(82).expect("dead-end neighbor should be retrieved");

    assert!(
        deep_score > dead_end_score,
        "a node reinforced by two independent paths from the seed (deep_score={deep_score}) \
         should score higher than a plain single-edge callee (dead_end_score={dead_end_score})"
    );
}

// ── Plan 22 item 4: config-driven PPR iteration count ───────────────────────

/// `RetrievalConfig::ppr.iterations` must actually reach `personalized_pagerank`
/// — confirmed by an iteration count low enough that a real 2-hop
/// reinforcement path hasn't had time to propagate yet. Same fixture shape as
/// `graph_centrality_rewards_an_actual_multi_hop_relevant_path`: at
/// `iterations = 1`, only the *direct* seed->deep_chain edge has fired (the
/// seed->intermediate->deep_chain path needs a second iteration to reach
/// deep_chain), so deep_chain and the plain single-edge dead_end should be
/// indistinguishable; at the default 4 iterations, the second path's extra
/// mass should pull deep_chain ahead.
#[tokio::test]
async fn ppr_iterations_config_actually_changes_how_far_mass_propagates() {
    let build = || {
        let mut g = MockGraph::new();
        let seed = make_symbol(80, "queryTarget");
        let deep_chain = make_symbol(81, "deeplyRelevant");
        let dead_end = make_symbol(82, "unrelatedDeadEnd");
        let intermediate = make_symbol(83, "intermediate");
        g.add(seed.clone());
        g.add_call(seed.clone(), intermediate.clone());
        g.add_call(seed.clone(), deep_chain.clone());
        g.add_call(intermediate, deep_chain.clone());
        g.add_call(seed.clone(), dead_end.clone());
        g.add_lexical_hit(80, 0.9);
        g
    };
    let query = RetrievalQuery {
        text: "query target".to_string(),
        top_k: 10, final_n: 10, expand_hops: 1, lang_filter: None,
    };

    let mut one_iter_config = RetrievalConfig::default();
    one_iter_config.ppr.iterations = 1;
    let p_one_iter = RetrievalPipeline {
        graph_db: Arc::new(build()), source_loader: Arc::new(SourceLoader::new()),
        config: one_iter_config, jev: None,
    };
    let results_one_iter = p_one_iter.query(query.clone()).await.unwrap();
    let score_of = |results: &[graphrelay_core::retrieval::RetrievalResult], id: u64| {
        results.iter().find(|r| r.symbol.id == id).map(|r| r.score)
    };
    let deep_one = score_of(&results_one_iter, 81).expect("deep_chain retrieved");
    let dead_one = score_of(&results_one_iter, 82).expect("dead_end retrieved");
    assert!(
        (deep_one - dead_one).abs() < 1e-4,
        "at 1 iteration, the 2-hop reinforcement path hasn't propagated yet — \
         deep_chain ({deep_one}) and dead_end ({dead_one}) should score equally"
    );

    // Default config (4 iterations) — the same fixture, freshly built.
    let p_default = pipeline(build());
    let results_default = p_default.query(query).await.unwrap();
    let deep_default = score_of(&results_default, 81).expect("deep_chain retrieved");
    let dead_default = score_of(&results_default, 82).expect("dead_end retrieved");
    assert!(
        deep_default > dead_default,
        "at the default 4 iterations, deep_chain's second reinforcement path should have \
         propagated: deep={deep_default} dead_end={dead_default}"
    );
}

/// `early_stop_epsilon` set to a very large value stops PPR after its first
/// iteration (every delta is below it) — same observable effect as
/// `iterations = 1` on the same fixture, confirming the early-stop path
/// actually terminates the walk rather than being silently ignored.
#[tokio::test]
async fn ppr_early_stop_epsilon_terminates_before_full_iteration_count() {
    let mut g = MockGraph::new();
    let seed = make_symbol(80, "queryTarget");
    let deep_chain = make_symbol(81, "deeplyRelevant");
    let dead_end = make_symbol(82, "unrelatedDeadEnd");
    let intermediate = make_symbol(83, "intermediate");
    g.add(seed.clone());
    g.add_call(seed.clone(), intermediate.clone());
    g.add_call(seed.clone(), deep_chain.clone());
    g.add_call(intermediate, deep_chain.clone());
    g.add_call(seed.clone(), dead_end.clone());
    g.add_lexical_hit(80, 0.9);

    let mut config = RetrievalConfig::default(); // iterations=4 by default
    config.ppr.early_stop_epsilon = Some(1000.0); // never a real delta this large — fires after iteration 1
    let p = RetrievalPipeline {
        graph_db: Arc::new(g), source_loader: Arc::new(SourceLoader::new()), config, jev: None,
    };
    let results = p.query(RetrievalQuery {
        text: "query target".to_string(),
        top_k: 10, final_n: 10, expand_hops: 1, lang_filter: None,
    }).await.unwrap();

    let score_of = |id: u64| results.iter().find(|r| r.symbol.id == id).map(|r| r.score);
    let deep = score_of(81).expect("deep_chain retrieved");
    let dead = score_of(82).expect("dead_end retrieved");
    assert!(
        (deep - dead).abs() < 1e-4,
        "an early stop after iteration 1 should look identical to iterations=1: deep={deep} dead={dead}"
    );
}

// ── Plan 20 Phase 8: optional cloud-judge (Jev) rerank signal ───────────────

/// Returns a fixed relevance score per candidate, or an error when `fail` is
/// set — exercises both `apply_jev`'s success path and its graceful fallback
/// on a judging failure.
struct MockJudge {
    scores: HashMap<SymbolId, f32>,
    fail:   bool,
}

#[async_trait::async_trait]
impl RelevanceJudge for MockJudge {
    async fn judge_relevance(
        &self,
        _query: &str,
        _candidates: &[(SymbolId, String)],
    ) -> anyhow::Result<HashMap<SymbolId, f32>> {
        if self.fail {
            anyhow::bail!("mock judge failure");
        }
        Ok(self.scores.clone())
    }
}

/// Captures every `candidates` argument it's called with, for asserting on
/// what `apply_jev` actually sends over the wire (truncation, in particular)
/// — `MockJudge` above only cares about what it returns, not what it saw.
struct RecordingJudge {
    seen: std::sync::Mutex<Vec<(SymbolId, String)>>,
}

#[async_trait::async_trait]
impl RelevanceJudge for RecordingJudge {
    async fn judge_relevance(
        &self,
        _query: &str,
        candidates: &[(SymbolId, String)],
    ) -> anyhow::Result<HashMap<SymbolId, f32>> {
        *self.seen.lock().unwrap() = candidates.to_vec();
        Ok(HashMap::new())
    }
}

fn pipeline_with_jev(
    graph: MockGraph,
    judge: MockJudge,
    enabled: bool,
    top_n: usize,
    weight: f32,
) -> RetrievalPipeline<MockGraph> {
    let mut config = RetrievalConfig::default();
    config.jev.enabled = enabled;
    config.jev.top_n = top_n;
    config.jev.weight = weight;
    RetrievalPipeline {
        graph_db:      Arc::new(graph),
        source_loader: Arc::new(SourceLoader::new()),
        config,
        jev: Some(Arc::new(judge)),
    }
}

/// Six isolated (no callers/callees) direct hits with strictly descending
/// lexical scores, so absent any Jev influence they rank 1..=6 by id in that
/// order and `final_n=5` would naturally cut off id 6.
fn six_candidate_graph() -> MockGraph {
    let mut g = MockGraph::new();
    for (i, score) in [0.6, 0.5, 0.4, 0.3, 0.2, 0.1].into_iter().enumerate() {
        let id = (i + 1) as u64;
        g.add(make_symbol(id, &format!("fn_{id}")));
        g.add_lexical_hit(id, score);
    }
    g
}

/// `n` candidates with strictly descending lexical scores (id order = rank
/// order), for testing recall-width behavior beyond what `six_candidate_graph`
/// can exercise.
fn many_candidates_graph(n: u64) -> MockGraph {
    let mut g = MockGraph::new();
    for i in 1..=n {
        g.add(make_symbol(i, &format!("fn_{i}")));
        g.add_lexical_hit(i, 1.0 - (i as f32 * 0.01));
    }
    g
}

/// A judge that scored every candidate in `1..=n`: `hit` as fully relevant,
/// the rest as irrelevant. Unscored ids get the mean of returned scores, so a
/// test that wants one candidate promoted must have the others judged too.
fn judged_all(n: u64, hit: u64) -> HashMap<u64, f32> {
    (1..=n).map(|id| (id, if id == hit { 1.0 } else { 0.0 })).collect()
}

/// Same as `pipeline_with_jev` but enabling the Laya section only (Plan 25): the pipeline
/// must read `laya.top_n` / `laya.weight`, never Jev's, and report the provider.
fn pipeline_with_laya(
    graph: MockGraph,
    judge: MockJudge,
    top_n: usize,
    weight: f32,
) -> RetrievalPipeline<MockGraph> {
    let mut config = RetrievalConfig::default();
    config.laya.enabled = true;
    config.laya.top_n = top_n;
    config.laya.weight = weight;
    // Deliberately different Jev values, to prove they are not consulted.
    config.jev.top_n = 1;
    config.jev.weight = 0.0;
    RetrievalPipeline {
        graph_db:      Arc::new(graph),
        source_loader: Arc::new(SourceLoader::new()),
        config,
        jev: Some(Arc::new(judge)),
    }
}

#[tokio::test]
async fn a_laya_judge_uses_the_laya_pool_width_and_weight_and_reports_its_provider() {
    let scores = judged_all(25, 20);
    let p = pipeline_with_laya(many_candidates_graph(25), MockJudge { scores, fail: false }, 25, 1.0);
    assert_eq!(graphrelay_core::retrieval::QueryPipeline::judge_provider(&p), Some("laya"));
    assert!(graphrelay_core::retrieval::QueryPipeline::jev_active(&p), "jev_active now means any judge");

    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(results.iter().any(|r| r.symbol.id == 20),
        "recall must widen to laya.top_n=25 (jev.top_n is 1 here) and the laya weight (1.0, not jev's 0.0) must apply");
    assert_eq!(results[0].symbol.id, 20, "a +1.0 bonus at weight 1.0 ranks the judged candidate first");
}

#[tokio::test]
async fn a_failing_laya_judge_falls_back_to_the_base_ranking() {
    let p = pipeline_with_laya(six_candidate_graph(), MockJudge { scores: HashMap::new(), fail: true }, 10, 1.0);
    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();
    let ids: Vec<u64> = results.iter().map(|r| r.symbol.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5], "judge failure must never fail or reorder the query");
}

#[tokio::test]
async fn no_judge_enabled_reports_no_provider() {
    let p = pipeline_with_jev(six_candidate_graph(), MockJudge { scores: HashMap::new(), fail: false }, false, 10, 1.0);
    assert_eq!(graphrelay_core::retrieval::QueryPipeline::judge_provider(&p), None);
    assert!(!graphrelay_core::retrieval::QueryPipeline::jev_active(&p));
}

#[tokio::test]
async fn jev_active_widens_lexical_recall_beyond_the_requested_top_k() {
    // 25 candidates; req.top_k=5 alone would only ever reach MockGraph's
    // lexical_search with top_k=5, so candidate 20 could never be a seed at
    // all unless jev.top_n (25, > req.top_k) widens what's actually asked
    // for — this is the Plan 20 Phase 8 follow-up (keycloak's kc-sg-12).
    let scores = judged_all(25, 20);
    let p = pipeline_with_jev(many_candidates_graph(25), MockJudge { scores, fail: false }, true, 25, 1.0);

    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(results.iter().any(|r| r.symbol.id == 20),
        "candidate 20 is only reachable if lexical recall widened beyond req.top_k=5 to jev.top_n=25");
}

#[tokio::test]
async fn jev_disabled_leaves_lexical_recall_at_the_requested_top_k() {
    // Same setup as above, but jev disabled — candidate 20 must NOT appear,
    // confirming the widening is strictly gated to jev_active and doesn't
    // leak into the default (no-Jev) retrieval path, which is already
    // validated at 40/40 and must not change.
    let scores = HashMap::from([(20u64, 1.0)]);
    let p = pipeline_with_jev(many_candidates_graph(25), MockJudge { scores, fail: false }, false, 25, 1.0);

    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(!results.iter().any(|r| r.symbol.id == 20),
        "with jev disabled, lexical recall must stay at req.top_k=5 — no widening leak");
}

#[tokio::test]
async fn jev_disabled_never_invokes_the_judge_even_when_one_is_configured() {
    // A judge that would panic-worthy promote id 6 to first place if called —
    // proves `config.jev.enabled = false` actually gates the call, not just
    // `jev.is_some()`.
    let scores = HashMap::from([(6u64, 1.0)]);
    let p = pipeline_with_jev(six_candidate_graph(), MockJudge { scores, fail: false }, false, 10, 1.0);

    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 5);
    assert!(!results.iter().any(|r| r.symbol.id == 6),
        "id 6 should still be cut at final_n=5 — jev.enabled=false must skip the judge entirely");
}

#[tokio::test]
async fn jev_enabled_can_promote_a_candidate_ranked_just_outside_final_n() {
    let scores = judged_all(6, 6);
    let p = pipeline_with_jev(six_candidate_graph(), MockJudge { scores, fail: false }, true, 10, 1.0);

    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 5);
    assert!(results.iter().any(|r| r.symbol.id == 6),
        "a strong Jev relevance score should be able to promote id 6 past the naive lexical+graph cutoff");
    assert_eq!(results[0].symbol.id, 6, "the +1.0 bonus should be enough to rank it first");
    // Ranks must be renumbered 1..=5 after the post-Jev re-sort, not left stale.
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r.rank, i + 1);
    }
}

/// The `RelevanceJudge` contract: a missing id is "no judgment", not "zero
/// relevance". A judge that returned only two (equal) scores carries no
/// ranking information, so it must not lift those two over the unjudged rest:
/// with "missing = 0" ids 5 and 6 jumped to the top.
#[tokio::test]
async fn a_partial_judgment_does_not_penalize_unscored_candidates() {
    let scores = HashMap::from([(5u64, 0.5), (6u64, 0.5)]);
    let p = pipeline_with_jev(six_candidate_graph(), MockJudge { scores, fail: false }, true, 10, 1.0);
    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();
    let ids: Vec<u64> = results.iter().map(|r| r.symbol.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5], "unscored candidates keep their base order relative to the judged ones");
}

#[tokio::test]
async fn jev_failure_falls_back_to_lexical_graph_ranking_without_failing_the_query() {
    let p_no_jev = pipeline(six_candidate_graph());
    let baseline = p_no_jev.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    let scores = HashMap::from([(6u64, 1.0)]);
    let p_jev = pipeline_with_jev(six_candidate_graph(), MockJudge { scores, fail: true }, true, 10, 1.0);
    let with_failing_jev = p_jev.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    let baseline_ids: Vec<_> = baseline.iter().map(|r| r.symbol.id).collect();
    let fallback_ids: Vec<_> = with_failing_jev.iter().map(|r| r.symbol.id).collect();
    assert_eq!(baseline_ids, fallback_ids,
        "a judging error must fall back to exactly the lexical+graph ranking, not fail the query or return a partial/broken result");
}

#[tokio::test]
async fn jev_partial_results_leave_unscored_candidates_at_their_pre_jev_score() {
    // Only id 6 gets a (weak) relevance score; ids 1-5 are absent from the
    // map entirely — must be treated as "no judgment," not "zero relevance",
    // so their relative order among themselves is untouched.
    let scores = HashMap::from([(6u64, 0.05)]); // too weak to promote past id 1-5
    let p = pipeline_with_jev(six_candidate_graph(), MockJudge { scores, fail: false }, true, 10, 0.1);

    let results = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    let ids: Vec<_> = results.iter().map(|r| r.symbol.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5],
        "unscored candidates 1-5 must keep their original relative lexical+graph order: {ids:?}");
}

#[tokio::test]
async fn jev_candidate_text_is_truncated_not_the_full_snippet() {
    // Real file with a body well over JEV_CANDIDATE_SNIPPET_CHARS (300) —
    // regression test for the keycloak top_n=50 finding
    // (bench-reports/plan20-phase8-jev-keycloak-benchmark-2026-09-25.md):
    // sending every candidate's full, untruncated snippet in one wide batch
    // measurably regressed judgment quality; truncation is the fix.
    let long_body = "x".repeat(1000);
    let file_path = std::env::temp_dir().join(format!("jev_truncation_test_{}.rs", std::process::id()));
    std::fs::write(&file_path, format!("fn long_fn() {{\n{long_body}\n}}\n")).unwrap();

    let mut sym = make_symbol(1, "long_fn");
    sym.file_path = file_path.clone();
    sym.span = Span { start_byte: 0, end_byte: 1000, start_line: 0, end_line: 2 };

    let mut g = MockGraph::new();
    g.add(sym);
    g.add_lexical_hit(1, 0.9);

    let judge = Arc::new(RecordingJudge { seen: std::sync::Mutex::new(Vec::new()) });
    let mut config = RetrievalConfig::default();
    config.jev.enabled = true;
    let p = RetrievalPipeline {
        graph_db:      Arc::new(g),
        source_loader: Arc::new(SourceLoader::new()),
        config,
        jev: Some(judge.clone() as Arc<dyn RelevanceJudge>),
    };

    let _ = p.query(RetrievalQuery {
        text: "query".to_string(), top_k: 10, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    std::fs::remove_file(&file_path).ok();

    let seen = judge.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (_, text) = &seen[0];
    assert!(
        text.len() < 1000,
        "candidate text sent to Jev must be truncated, not the full ~1000-char snippet: got {} chars",
        text.len()
    );
}

// ── Plan 22 item 1: generated-file soft down-rank ───────────────────────────

/// A file that's both test-shaped and generated-shaped must be penalized by
/// `max(TEST_FILE_PENALTY, GENERATED_FILE_PENALTY)`, not their sum — the
/// plan's explicit design choice ("a mockgen mock under /test/ is one kind
/// of noise, not two").
#[tokio::test]
async fn test_and_generated_penalties_combine_via_max_not_sum() {
    let mut g = MockGraph::new();

    let mut plain = make_symbol(1, "plainFn");
    plain.file_path = PathBuf::from("src/plain.rs");
    g.add(plain);
    g.add_lexical_hit(1, 0.9);

    let mut test_only = make_symbol(2, "testOnlyFn");
    test_only.file_path = PathBuf::from("src/foo_test.rs");
    g.add(test_only);
    g.add_lexical_hit(2, 0.9);

    let mut generated_only = make_symbol(3, "generatedOnlyFn");
    generated_only.file_path = PathBuf::from("src/foo_pb2.py");
    g.add(generated_only);
    g.add_lexical_hit(3, 0.9);

    // Matches BOTH is_test_file (starts_with "test_") and
    // is_generated_filename (ends_with "_pb2.py").
    let mut both = make_symbol(4, "bothFn");
    both.file_path = PathBuf::from("src/test_foo_pb2.py");
    g.add(both);
    g.add_lexical_hit(4, 0.9);

    let p = pipeline(g);
    let results = p.query(RetrievalQuery {
        text: "plain query mentioning neither penalty-gate word".to_string(),
        top_k: 10, final_n: 10, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    let score_of = |id: u64| results.iter().find(|r| r.symbol.id == id).unwrap().score;
    let plain_score = score_of(1);
    let test_score = score_of(2);
    let generated_score = score_of(3);
    let both_score = score_of(4);

    assert!(plain_score > test_score, "a test file must be penalized relative to a plain one");
    assert!(plain_score > generated_score, "a generated file must be penalized relative to a plain one");
    assert!(
        (test_score - generated_score).abs() < 1e-6,
        "TEST_FILE_PENALTY and GENERATED_FILE_PENALTY are both 0.15 in this codebase: {test_score} vs {generated_score}"
    );
    assert!(
        (both_score - test_score).abs() < 1e-6,
        "combining both penalties must take max, not sum — a file that's both must not \
         score lower than a file that's only one: both={both_score} test_only={test_score}"
    );
}

/// The generated-file penalty must be skipped when the query is itself about
/// generated code — same gating shape as `query_mentions_tests`. Both runs
/// query for the *same* symbol name (so `name_token_match_score` and the
/// (empty, unreadable-fake-path) snippet-BM25 term stay identical between
/// them) and differ only in whether a generated-code gate word is present —
/// isolating the penalty-gate as the only thing that can explain a score
/// difference.
#[tokio::test]
async fn generated_file_penalty_is_gated_off_when_query_is_about_generated_code() {
    let build = || {
        let mut g = MockGraph::new();
        let mut generated = make_symbol(1, "xyzzyHandler");
        generated.file_path = PathBuf::from("src/foo_pb2.py");
        g.add(generated);
        g.add_lexical_hit(1, 0.9);
        g
    };

    let p1 = pipeline(build());
    let with_penalty = p1.query(RetrievalQuery {
        text: "find xyzzyHandler".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    let p2 = pipeline(build());
    let gated_off = p2.query(RetrievalQuery {
        text: "find xyzzyHandler protobuf".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(
        gated_off[0].score > with_penalty[0].score,
        "a query mentioning 'protobuf' must not apply the generated-file penalty: \
         gated_off={} with_penalty={}",
        gated_off[0].score, with_penalty[0].score
    );
}

// ── Plan 22 item 3: file-path pinning ───────────────────────────────────────

fn pipeline_with_path_pin(graph: MockGraph, mode: PathPinMode) -> RetrievalPipeline<MockGraph> {
    let mut config = RetrievalConfig::default();
    config.path_pin.mode = mode;
    RetrievalPipeline {
        graph_db:      Arc::new(graph),
        source_loader: Arc::new(SourceLoader::new()),
        config,
        jev: None,
    }
}

/// The core promise of pinning: a symbol with zero lexical score (FTS alone
/// would never surface it) is admitted anyway because the query names its
/// file, in both "strip" and "keep" mode.
#[tokio::test]
async fn path_pin_admits_a_symbol_fts_alone_would_never_find() {
    for mode in [PathPinMode::Strip, PathPinMode::Keep] {
        let mut g = MockGraph::new();
        g.add(make_symbol(1, "unrelated_helper"));
        g.add_lexical_hit(1, 0.9);

        let mut target = make_symbol(2, "specificFunction");
        target.file_path = PathBuf::from("src/billing/status.py");
        g.add(target);
        g.set_pinned_files("src/billing/status.py", vec!["src/billing/status.py".to_string()]);
        g.set_pinned_symbols("src/billing/status.py", vec![(2, 0.0)]);

        let p = pipeline_with_path_pin(g, mode);
        let results = p.query(RetrievalQuery {
            text: "what does src/billing/status.py do".to_string(),
            top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
        }).await.unwrap();

        assert!(
            results.iter().any(|r| r.symbol.id == 2),
            "{mode:?}: pinned symbol with zero lexical score must still be admitted: {:?}",
            results.iter().map(|r| r.symbol.id).collect::<Vec<_>>()
        );
    }
}

/// `mode: Off` (the default) must be a true no-op: detection doesn't even
/// run, so a path mention in the query behaves exactly as it did before item
/// 3 existed — the pinned-only symbol must NOT appear.
#[tokio::test]
async fn path_pin_off_explicitly_does_not_admit_pinned_symbols() {
    // `Off` is no longer the shipped default (arm A/"strip" is, after
    // measurement — see PathPinMode's doc comment) but must still work as an
    // explicit opt-out.
    let mut g = MockGraph::new();
    g.add(make_symbol(1, "unrelated_helper"));
    g.add_lexical_hit(1, 0.9);

    let mut target = make_symbol(2, "specificFunction");
    target.file_path = PathBuf::from("src/billing/status.py");
    g.add(target);
    g.set_pinned_files("src/billing/status.py", vec!["src/billing/status.py".to_string()]);
    g.set_pinned_symbols("src/billing/status.py", vec![(2, 0.0)]);

    let p = pipeline_with_path_pin(g, PathPinMode::Off);
    let results = p.query(RetrievalQuery {
        text: "what does src/billing/status.py do".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(
        !results.iter().any(|r| r.symbol.id == 2),
        "path_pin.mode == Off must be a no-op — pinned symbol must not appear"
    );
}

/// Ambiguity policy: a mention resolving to exactly 1 file pins it.
#[tokio::test]
async fn path_pin_ambiguity_single_match_pins() {
    let mut g = MockGraph::new();
    let mut target = make_symbol(2, "specificFunction");
    target.file_path = PathBuf::from("status.py");
    g.add(target);
    g.set_pinned_files("status.py", vec!["status.py".to_string()]);
    g.set_pinned_symbols("status.py", vec![(2, 0.0)]);

    let p = pipeline_with_path_pin(g, PathPinMode::Keep);
    let results = p.query(RetrievalQuery {
        text: "status.py".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(results.iter().any(|r| r.symbol.id == 2), "exactly 1 match must pin");
}

/// Ambiguity policy: a mention resolving to 2-3 files pins all of them (a
/// bare filename is genuinely ambiguous; the user named it, so every
/// candidate is fair).
#[tokio::test]
async fn path_pin_ambiguity_two_to_three_matches_pins_all() {
    let mut g = MockGraph::new();
    let mut a = make_symbol(2, "fnA");
    a.file_path = PathBuf::from("pkg_a/status.py");
    let mut b = make_symbol(3, "fnB");
    b.file_path = PathBuf::from("pkg_b/status.py");
    g.add(a);
    g.add(b);
    g.set_pinned_files("status.py", vec!["pkg_a/status.py".to_string(), "pkg_b/status.py".to_string()]);
    g.set_pinned_symbols("pkg_a/status.py", vec![(2, 0.0)]);
    g.set_pinned_symbols("pkg_b/status.py", vec![(3, 0.0)]);

    let p = pipeline_with_path_pin(g, PathPinMode::Keep);
    let results = p.query(RetrievalQuery {
        text: "status.py".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    let ids: Vec<_> = results.iter().map(|r| r.symbol.id).collect();
    assert!(ids.contains(&2) && ids.contains(&3), "2-3 matches must pin all of them: {ids:?}");
}

/// Ambiguity policy: more than 3 matches is too vague to trust — none of
/// them get pinned (the mention is left as plain, unpinned query text).
#[tokio::test]
async fn path_pin_ambiguity_more_than_three_matches_pins_none() {
    let mut g = MockGraph::new();
    let mut targets = vec![];
    for i in 0..4u64 {
        let id = 100 + i;
        let mut s = make_symbol(id, &format!("fn{i}"));
        s.file_path = PathBuf::from(format!("pkg_{i}/status.py"));
        g.add(s);
        targets.push(id);
    }
    g.set_pinned_files(
        "status.py",
        (0..4).map(|i| format!("pkg_{i}/status.py")).collect(),
    );
    for i in 0..4u64 {
        g.set_pinned_symbols(&format!("pkg_{i}/status.py"), vec![(100 + i, 0.0)]);
    }
    // No FTS hits at all — if pinning fired for any of these it would be the
    // only way any of them appears.
    let p = pipeline_with_path_pin(g, PathPinMode::Keep);
    let results = p.query(RetrievalQuery {
        text: "status.py".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert!(
        results.is_empty(),
        ">3 matches must pin nothing (too vague to trust): {:?}",
        results.iter().map(|r| r.symbol.id).collect::<Vec<_>>()
    );
    let _ = targets;
}

/// A pinned symbol gets `path_pin.bonus` added to its score — confirmed by
/// comparing against the same setup with pinning off (score must strictly
/// increase, all else equal).
#[tokio::test]
async fn path_pin_bonus_is_applied_to_pinned_candidates() {
    let build = || {
        let mut g = MockGraph::new();
        let mut target = make_symbol(2, "specificFunction");
        target.file_path = PathBuf::from("status.py");
        g.add(target);
        g.add_lexical_hit(2, 0.5); // give it a real lexical score too, so it appears even when pinning is off
        g.set_pinned_files("status.py", vec!["status.py".to_string()]);
        g.set_pinned_symbols("status.py", vec![(2, 0.5)]);
        g
    };

    let p_off = pipeline_with_path_pin(build(), PathPinMode::Off);
    let results_off = p_off.query(RetrievalQuery {
        text: "status.py".to_string(), top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();
    let score_off = results_off.iter().find(|r| r.symbol.id == 2).unwrap().score;

    let mut config = RetrievalConfig::default();
    config.path_pin.mode = PathPinMode::Keep;
    config.path_pin.bonus = 0.3;
    let p_on = RetrievalPipeline {
        graph_db: Arc::new(build()), source_loader: Arc::new(SourceLoader::new()), config, jev: None,
    };
    let results_on = p_on.query(RetrievalQuery {
        text: "status.py".to_string(), top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();
    let score_on = results_on.iter().find(|r| r.symbol.id == 2).unwrap().score;

    assert!(
        score_on > score_off,
        "pinned candidate's score ({score_on}) should exceed the unpinned baseline ({score_off})"
    );
}

/// A mention that resolves to zero files (a URL-like string, or genuine
/// non-existent path) must not pin anything and must not error the query —
/// it's silently left as plain, unpinned query text.
#[tokio::test]
async fn path_pin_unresolved_mention_pins_nothing_and_does_not_error() {
    let mut g = MockGraph::new();
    g.add(make_symbol(1, "some_fn"));
    g.add_lexical_hit(1, 0.9);
    // No `set_pinned_files` call for this mention — mirrors a real
    // `resolve_pinned_files` returning empty for e.g. a URL.
    let p = pipeline_with_path_pin(g, PathPinMode::Keep);

    let results = p.query(RetrievalQuery {
        text: "see https://example.com/foo.py for details".to_string(),
        top_k: 5, final_n: 5, expand_hops: 0, lang_filter: None,
    }).await.unwrap();

    assert_eq!(results.len(), 1, "unresolved mention must not error or otherwise change the result set");
    assert_eq!(results[0].symbol.id, 1);
}

/// The language filter's budget counts unpinned hits only: a pinned symbol that
/// scores below the cutoff must still be admitted.
#[tokio::test]
async fn lang_filter_budget_never_drops_a_pinned_symbol() {
    let mut g = MockGraph::new();
    for id in 1..=4u64 {
        let mut sym = make_symbol(id, &format!("hot_{id}"));
        sym.language = Language::Python;
        sym.file_path = PathBuf::from("hot.py");
        g.add(sym);
        g.add_lexical_hit(id, 0.9 - id as f32 * 0.01);
    }
    let mut pinned = make_symbol(9, "pinned_target");
    pinned.language = Language::Python;
    pinned.file_path = PathBuf::from("status.py");
    g.add(pinned);
    g.set_pinned_files("status.py", vec!["status.py".to_string()]);
    g.set_pinned_symbols("status.py", vec![(9, 0.0)]);
    let p = pipeline_with_path_pin(g, PathPinMode::Keep);

    let results = p.query(RetrievalQuery {
        text: "status.py".to_string(), top_k: 2, final_n: 10, expand_hops: 0, lang_filter: Some("python".to_string()),
    }).await.unwrap();

    assert!(results.iter().any(|r| r.symbol.id == 9), "pinned symbol was truncated away: {:?}", results.iter().map(|r| r.symbol.id).collect::<Vec<_>>());
}
