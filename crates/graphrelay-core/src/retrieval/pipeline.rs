use crate::config::{PathPinMode, RetrievalConfig};
use crate::relevance_judge::RelevanceJudge;
use crate::retrieval::{EnrichedCandidate, RetrievalQuery, RetrievalResult};
use crate::retrieval::source_loader::SourceLoader;
use crate::types::{SymbolId, SymbolRef};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Object-safe async trait for querying the retrieval pipeline.
/// Use `Arc<dyn QueryPipeline>` to avoid generic parameters leaking into dependents.
#[async_trait::async_trait]
pub trait QueryPipeline: Send + Sync {
    async fn query(&self, req: RetrievalQuery) -> anyhow::Result<Vec<RetrievalResult>>;

    /// True when Plan 20 Phase 8's optional cloud-judge rerank signal
    /// (`config.jev.enabled` AND a judge is actually wired up — not just the
    /// config flag) is active for this pipeline. Exposed so `watch_status`
    /// and `/api/status` can surface it to agents/clients, the same spirit
    /// as the removed `embedding_degraded` field before Plan 20 — this is a
    /// mode an agent may want to know about, not a mode swap. Default
    /// `false` — only the concrete `RetrievalPipeline` impl overrides it.
    fn jev_active(&self) -> bool {
        false
    }

    /// Lexical candidate pool size callers should request (`[retrieval]
    /// candidate_top_k`).
    fn candidate_top_k(&self) -> usize {
        20
    }

    /// Result count for callers with no tool-level default of their own
    /// (`[retrieval] final_top_n`).
    fn final_top_n(&self) -> usize {
        5
    }

    /// Which relevance judge is active (`"jev"` or `"laya"`), or `None`. Same
    /// condition as `jev_active` (a judge is configured AND wired up); the name
    /// `jev_active` is kept for API compatibility and now means "any judge".
    fn judge_provider(&self) -> Option<&'static str> {
        None
    }
}

/// Trait for the graph database operations needed by the retrieval pipeline.
/// This avoids a hard dependency on graphrelay-store from graphrelay-core.
pub trait GraphStore: Send + Sync {
    fn symbol_by_id(&self, id: SymbolId) -> anyhow::Result<Option<crate::types::Symbol>>;
    fn callers_of(&self, id: SymbolId) -> anyhow::Result<Vec<crate::types::Symbol>>;
    fn callees_of(&self, id: SymbolId) -> anyhow::Result<Vec<crate::types::Symbol>>;
    fn neighbors(&self, id: SymbolId, hops: u8) -> anyhow::Result<Vec<crate::types::Symbol>>;
    fn in_degree(&self, id: SymbolId) -> anyhow::Result<u32>;

    /// Batched `symbol_by_id`; ids with no row are skipped. The default loops,
    /// `GraphDb` overrides it with one `IN (...)` query.
    fn symbols_by_ids(&self, ids: &[SymbolId]) -> anyhow::Result<Vec<crate::types::Symbol>> {
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(s) = self.symbol_by_id(id)? {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// The ids `callees_of` would return, without materializing full symbols
    /// (personalized PageRank only needs ids).
    fn callee_ids_of(&self, id: SymbolId) -> anyhow::Result<Vec<SymbolId>> {
        Ok(self.callees_of(id)?.into_iter().map(|s| s.id).collect())
    }

    /// Lexical candidate generation (SQLite FTS5 + `bm25()`, real corpus IDF).
    /// Returns `(SymbolId, normalized_score)` pairs — the sole candidate-generation
    /// channel since Plan 20 removed vector search. Default: no lexical backend
    /// (empty result) — only `GraphDb` overrides this today, so any other
    /// `GraphStore` implementor (mocks, future backends) degrades gracefully to
    /// an empty candidate pool instead of failing to compile.
    ///
    /// `terms` is the full original+synonym term set (same as `tokenize_query`'s
    /// output); `synonym_terms` (Plan 21 item 14) is the subset of `terms` that
    /// exists only because of synonym expansion — implementations should weight
    /// matches on synonym-only terms lower than matches on the user's own
    /// words, per `tokenize_query_split`'s doc comment.
    fn lexical_search(&self, _terms: &[String], _synonym_terms: &[String], _top_k: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        Ok(vec![])
    }

    /// Plan 22 item 3 — file-path pinning, step 1: resolve a query-detected
    /// path-like substring to indexed file path(s) via exact match, falling
    /// back to a suffix match (same precedence as `get_file_context`'s
    /// `resolve_file`). The caller (`RetrievalPipeline::resolve_path_pins`)
    /// applies the ambiguity policy (1 or 2-3 matches pin all of them, >3 is
    /// too vague and pins none) — this method just returns the raw match
    /// set. Empty default like `lexical_search`, so `graphrelay-core` and
    /// mocks stay I/O-free.
    fn resolve_pinned_files(&self, _path_text: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    /// Plan 22 item 3 — file-path pinning, step 2: for one already-resolved
    /// pinned file, return up to `max_symbols` `(SymbolId, score)` pairs —
    /// scored by FTS `bm25()` of `remaining_terms` restricted to this file's
    /// own symbols (the "does the rest of the query point at a specific part
    /// of this file" signal), falling back to a stable slice (score 0.0) when
    /// no remaining term matches anything in the file — e.g. the query is
    /// *just* the path, with nothing else to disambiguate which symbol.
    /// `remaining_terms` has the pinned path mention(s) already stripped out
    /// (see `resolve_path_pins`) regardless of the strip/keep arm, since
    /// matching the path substring's own stems against symbol text inside
    /// the file it names is redundant once the file is already pinned. Empty
    /// default like `lexical_search`.
    fn symbols_for_path_pin(&self, _file_path: &str, _remaining_terms: &[String], _max_symbols: usize) -> anyhow::Result<Vec<(SymbolId, f32)>> {
        Ok(vec![])
    }
}

/// `(candidate_ids, lexical_score_by_id, direct_hit_ids)` — see `expand_graph`'s
/// doc comment for what each set/map means.
type ExpandedCandidates = (HashSet<SymbolId>, HashMap<SymbolId, f32>, HashSet<SymbolId>);

/// The orchestrating retrieval pipeline: lexical (FTS5/BM25) candidate
/// generation + graph expansion + personalized PageRank + rerank. No vector
/// search — Plan 20 removed the embedding-backed candidate-generation and
/// reranking legs entirely (see `plans/20-remove-vector-search.md`); this is
/// the lexical+graph path that pipeline previously only used when the
/// embedder was a stub, now the only path.
/// Generic over the graph store impl to keep graphrelay-core dependency-free.
pub struct RetrievalPipeline<G> {
    pub graph_db:      Arc<G>,
    pub source_loader: Arc<SourceLoader>,
    pub config:        RetrievalConfig,
    /// Plan 20 Phase 8: optional cloud-judge rerank signal. `None` when no
    /// provider is configured (the common case) — a missing judge is treated
    /// identically to `config.jev.enabled = false`, not an error, so callers
    /// don't need to construct a no-op implementation just to leave it off.
    pub jev: Option<Arc<dyn RelevanceJudge>>,
}

impl<G> RetrievalPipeline<G>
where
    G: GraphStore,
{
    pub async fn query(&self, req: RetrievalQuery) -> anyhow::Result<Vec<RetrievalResult>> {
        // Tokenize the query once — shared by lexical candidate generation
        // (Stage 1) and reranking (Stage 4), instead of re-splitting the same
        // text twice with two copies of the same filter logic. `synonym_terms`
        // (Plan 21 item 14) is only consumed by Stage 1's `lexical_search` —
        // reranking still scores against the full combined `query_terms`,
        // since down-weighting synonyms specifically in FTS candidate
        // generation is what the audit measured as overweighted, not rerank's
        // name/keyword scoring.
        // Plan 22 item 3 (Tier 2, gated off by default via
        // `config.path_pin.mode`): resolve path-like substrings in the raw
        // query *before* tokenizing — `tokenize_query_split` would otherwise
        // shred a path into generic stems that can't match anything
        // meaningfully (`chunk_fts.file_path` is untokenized; see the plan's
        // arm C finding). `fts_query_text` is `req.text` unchanged in
        // "off"/"keep" mode, or with the matched mention(s) removed in
        // "strip" mode — see `resolve_path_pins`'s doc comment.
        let (pinned_hits, fts_query_text) = self.resolve_path_pins(&req.text);
        let pinned_ids: HashSet<SymbolId> = pinned_hits.keys().copied().collect();

        let (query_terms, synonym_terms) = tokenize_query_split(&fts_query_text);
        // Whole camelCase identifiers as extra FTS-stage terms only (rerank keeps
        // `query_terms` untouched): `split_words` never emits the compound and
        // drops sub-tokens under 3 chars, so `getId`/`toUI` produced no terms at
        // all while the index holds them verbatim (Plan 24 item 3.1).
        let mut lexical_terms = query_terms.clone();
        for t in identifier_terms(&fts_query_text) {
            if !lexical_terms.contains(&t) {
                lexical_terms.push(t);
            }
        }

        // Stage 1: lexical candidate generation (SQLite FTS5 + `bm25()`) — the
        // sole candidate-generation channel. Widened beyond the caller's own
        // `top_k` when Jev is active (see `JevConfig::top_n`'s doc comment) —
        // gated specifically because a wider *unfiltered* pool was already
        // tried and regressed this exact benchmark twice before Jev existed
        // to filter it; it's Jev's presence downstream that makes recall
        // widening viable now, not something safe to do unconditionally.
        let jev_active = self.config.judge_provider().is_some() && self.jev.is_some();
        let base_top_k = if jev_active {
            req.top_k.max(self.config.judge_top_n())
        } else {
            req.top_k
        };
        // With a language filter, other-language hits would spend the candidate
        // budget and then be discarded post-enrich, so a minority language in a
        // mixed repo could come back nearly empty. Over-fetch, drop
        // other-language hits up front, and keep the same budget for the rest.
        let lang_filter = req.lang_filter.as_deref();
        let lexical_top_k = if lang_filter.is_some() { base_top_k * LANG_FILTER_OVERFETCH } else { base_top_k };
        let mut lexical_hits = self
            .graph_db
            .lexical_search(&lexical_terms, &synonym_terms, lexical_top_k)
            .unwrap_or_else(|e| {
                tracing::warn!("lexical_search failed: {e}");
                vec![]
            });

        // Merge pinned candidates in as additional direct hits —
        // `expand_graph` treats every entry passed to it as a direct hit
        // regardless of the 100-candidate cap (see its own doc comment), so a
        // pinned symbol is guaranteed admission into the pool even when
        // plain FTS alone would miss it entirely — the whole point of
        // pinning. Dedup by symbol id, keeping the stronger of the two scores
        // when a symbol is both an FTS hit and pinned.
        if !pinned_hits.is_empty() {
            let mut by_id: HashMap<SymbolId, f32> = lexical_hits.iter().copied().collect();
            for (&id, &score) in &pinned_hits {
                by_id.entry(id).and_modify(|s| *s = s.max(score)).or_insert(score);
            }
            lexical_hits = by_id.into_iter().collect();
            // HashMap iteration order is per-process random; `expand_graph`'s
            // 100-candidate cap admits neighbors in seed order, so an unsorted
            // merge made results nondeterministic (Plan 24 item 0.3).
            lexical_hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        }

        if let Some(lang) = lang_filter {
            let ids: Vec<SymbolId> = lexical_hits.iter().map(|h| h.0).collect();
            let keep: HashSet<SymbolId> = self
                .graph_db
                .symbols_by_ids(&ids)?
                .into_iter()
                .filter(|s| s.language.as_str().eq_ignore_ascii_case(lang))
                .map(|s| s.id)
                .collect();
            lexical_hits.retain(|h| keep.contains(&h.0) || pinned_ids.contains(&h.0));
            // Budget counts unpinned hits only; a pinned hit past the cutoff must survive.
            let mut unpinned_kept = 0;
            lexical_hits.retain(|h| {
                pinned_ids.contains(&h.0) || {
                    unpinned_kept += 1;
                    unpinned_kept <= base_top_k
                }
            });
        }

        if lexical_hits.is_empty() {
            return Ok(vec![]);
        }

        // Stage 2: graph expansion. Widened hops (>= 2) vs. the raw request —
        // structural signal (graph adjacency) is the one channel that never
        // depended on an embedder, so let it reach further to compensate for
        // there being no vector recall at all. Carried over unconditionally
        // from what was previously the `embedding_degraded`-only branch.
        let expand_hops = req.expand_hops.max(2);
        let (candidates, lexical_scores, direct_ids) =
            self.expand_graph(&lexical_hits, expand_hops, lang_filter)?;

        // Stage 2b: personalized PageRank, seeded from this query's direct
        // hits (their own lexical strength as restart weight) — see
        // `personalized_pagerank`'s doc comment for why this replaces raw
        // in-degree as the graph-centrality signal in `rerank`.
        let seed_weights: HashMap<SymbolId, f32> = direct_ids
            .iter()
            .map(|&id| {
                let strength = lexical_scores.get(&id).copied().unwrap_or(0.0)
                    .max(1e-3); // floor: every direct hit should seed *some* mass
                (id, strength)
            })
            .collect();
        let graph_scores = personalized_pagerank(
            self.graph_db.as_ref(),
            &seed_weights,
            PPR_RESTART_PROB,
            self.config.ppr.iterations,
            PPR_MAX_NODES,
            self.config.ppr.early_stop_epsilon,
        );

        // Stage 3: enrichment
        let mut enriched = self.enrich(&candidates, &lexical_scores, &graph_scores, &direct_ids)?;

        // Stage 3b: language filter (Plan 21 item 6). Lexical hits and graph
        // neighbors were already restricted above (Plan 28) so the candidate
        // budget goes to the requested language; this pass still guards
        // pinned files and keeps rerank's pool-normalized terms
        // (`max_graph_score`, `max_bm25`, live-BM25's `avg_len`) computed over
        // the language-filtered set only.
        if let Some(lang) = &req.lang_filter {
            enriched.retain(|r| r.symbol.language.as_str().eq_ignore_ascii_case(lang));
        }

        // Stage 4: rerank (includes BM25 boost if enabled)
        let mentions_tests = query_mentions_tests(&req.text);
        let mentions_generated = query_mentions_generated_code(&req.text);
        // Stage 5 (Plan 20 Phase 8, optional): widen the pool rerank() keeps
        // when Jev is active, so it can promote a candidate that ranked just
        // outside `final_n`, not just re-order what was already returned.
        // `jev_active` computed once above (Stage 1) — reused here.
        let rerank_n = if jev_active {
            req.final_n.max(self.config.judge_top_n())
        } else {
            req.final_n
        };
        let reranked = self.rerank(enriched, rerank_n, &query_terms, mentions_tests, mentions_generated, &pinned_ids);

        let mut results = if jev_active {
            self.apply_jev(reranked, &req.text, req.final_n).await
        } else {
            reranked
        };
        self.attach_call_neighbors(&mut results);
        Ok(results)
    }

    /// Callers/callees are only shown for returned results, so they are fetched
    /// after the final truncation, not for every one of the ~100 candidates
    /// (Plan 28: that was two 200-row queries per candidate, nearly all discarded).
    fn attach_call_neighbors(&self, results: &mut [RetrievalResult]) {
        for r in results.iter_mut() {
            r.callers = self.graph_db.callers_of(r.symbol.id).unwrap_or_else(|e| {
                tracing::warn!(symbol = %r.symbol.qualified_name, "callers_of failed: {e}");
                vec![]
            }).iter().map(SymbolRef::from).collect();
            r.callees = self.graph_db.callees_of(r.symbol.id).unwrap_or_else(|e| {
                tracing::warn!(symbol = %r.symbol.qualified_name, "callees_of failed: {e}");
                vec![]
            }).iter().map(SymbolRef::from).collect();
        }
    }

    /// Stage 5 (Plan 20 Phase 8, optional): blend in Jev's per-candidate
    /// relevance judgment as an additive bonus (`config.jev.weight *
    /// relevance`), re-sort, and truncate to `final_n`. Never fails the
    /// query — a judging error or an empty/partial result just falls back to
    /// the lexical+graph ranking `rerank()` already produced.
    async fn apply_jev(
        &self,
        mut results: Vec<RetrievalResult>,
        query_text: &str,
        final_n: usize,
    ) -> Vec<RetrievalResult> {
        let Some(judge) = &self.jev else {
            results.truncate(final_n);
            renumber_ranks(&mut results);
            return results;
        };

        let candidates: Vec<(SymbolId, String)> = results
            .iter()
            .map(|r| {
                // Truncated, not the full (often 500-1500+ char, fully
                // untruncated — see `SourceLoader::snippet`) symbol body —
                // "keep chunks tight" is TypeSafe's own operational guidance
                // for judging many candidates in one batched call, and a
                // wider pool (`jev.top_n` widening, see that field's doc
                // comment) previously regressed keycloak when every
                // candidate carried its full snippet. `.chars().take(...)`,
                // not byte slicing, so this can't panic on a multi-byte UTF-8
                // boundary (same pattern `ChunkBuilder` already uses).
                let snippet: String = r.snippet.chars().take(JEV_CANDIDATE_SNIPPET_CHARS).collect();
                (r.symbol.id, format!("{}\n{}", r.symbol.qualified_name, snippet))
            })
            .collect();

        let scores = match judge.judge_relevance(query_text, &candidates).await {
            Ok(scores) => scores,
            Err(e) => {
                tracing::warn!(
                    "Jev relevance judging failed, falling back to lexical+graph ranking: {e}"
                );
                HashMap::new()
            }
        };

        if !scores.is_empty() {
            let weight = self.config.judge_weight();
            // A candidate the judge never scored (deadline hit, partial batch) gets
            // the mean of the returned scores, not an implicit 0: the bonus is
            // never negative, so "no score" used to rank exactly like "judged
            // irrelevant", which the `RelevanceJudge` contract rules out.
            let neutral = neutral_relevance(&scores);
            for r in &mut results {
                let relevance = scores.get(&r.symbol.id).copied().unwrap_or(neutral);
                r.score += weight * relevance;
            }
            results.sort_by(|a, b| {
                b.score.total_cmp(&a.score).then_with(|| a.symbol.id.cmp(&b.symbol.id))
            });
        }

        results.truncate(final_n);
        renumber_ranks(&mut results);
        results
    }

    /// Plan 22 item 3: resolve this query's path mentions (when
    /// `config.path_pin.mode != Off`) to `(pinned SymbolId -> score)` pairs
    /// ready to merge into Stage 1's direct-hit set, plus the query text to
    /// actually feed `lexical_search` — `req_text` unchanged in "off"/"keep"
    /// mode, or with the pinned mention(s) removed in "strip" mode (arm A vs
    /// B in the plan).
    ///
    /// Ambiguity policy: a mention resolving to exactly 1, or to 2-3, indexed
    /// files pins all of them (the user named it; a handful of same-named
    /// files are all fair candidates). More than 3 is too vague to trust and
    /// is left as plain query text instead — not pinned, and (even in
    /// "strip" mode) not stripped either, since only mentions that actually
    /// got pinned are removed from the FTS text.
    fn resolve_path_pins(&self, req_text: &str) -> (HashMap<SymbolId, f32>, String) {
        if self.config.path_pin.mode == PathPinMode::Off {
            return (HashMap::new(), req_text.to_string());
        }

        let mentions = detect_path_mentions(req_text);
        if mentions.is_empty() {
            return (HashMap::new(), req_text.to_string());
        }

        let mut pinned_mentions: HashSet<String> = HashSet::new();
        let mut pinned_files: Vec<String> = Vec::new();
        for mention in &mentions {
            let matches = self.graph_db.resolve_pinned_files(mention).unwrap_or_else(|e| {
                tracing::warn!("resolve_pinned_files({mention:?}) failed: {e}");
                vec![]
            });
            if matches.is_empty() || matches.len() > 3 {
                continue;
            }
            pinned_mentions.insert(mention.clone());
            for f in matches {
                if !pinned_files.contains(&f) {
                    pinned_files.push(f);
                }
            }
        }

        if pinned_files.is_empty() {
            return (HashMap::new(), req_text.to_string());
        }

        // Always strip pinned mentions before computing each pinned file's
        // own within-file selection terms, regardless of the strip/keep arm
        // — matching the path substring's own stems against a file's own
        // symbol text is redundant once the file is already pinned (arm C,
        // tokenizing `chunk_fts.file_path` itself, is a separate, later
        // measured change).
        let stripped_text = strip_path_mentions(req_text, &pinned_mentions);
        let (remaining_terms, _remaining_synonyms) = tokenize_query_split(&stripped_text);

        let mut pinned: HashMap<SymbolId, f32> = HashMap::new();
        for file in &pinned_files {
            let hits = self
                .graph_db
                .symbols_for_path_pin(file, &remaining_terms, self.config.path_pin.max_symbols_per_file)
                .unwrap_or_else(|e| {
                    tracing::warn!("symbols_for_path_pin({file:?}) failed: {e}");
                    vec![]
                });
            for (id, score) in hits {
                pinned.entry(id).and_modify(|s| *s = s.max(score)).or_insert(score);
            }
        }

        let fts_query_text = match self.config.path_pin.mode {
            PathPinMode::Strip => stripped_text,
            PathPinMode::Keep | PathPinMode::Off => req_text.to_string(),
        };

        (pinned, fts_query_text)
    }

    /// Stage 2: graph expansion.
    ///
    /// Seeds from the lexical candidate-generation channel (`lexical_hits`,
    /// SQLite FTS5 + `bm25()`) and expands each seed's neighbors, decaying
    /// score by hop distance. This is what the pipeline previously did only
    /// under a stubbed embedder — Plan 20 made it the only path, since
    /// there's no vector channel to unify it with anymore.
    ///
    /// Returns:
    /// - `candidates`: set of every SymbolId in the pool (direct hits + their
    ///   expanded neighbors).
    /// - `lexical_scores`: map of SymbolId → best lexical score (direct, or
    ///   decayed from a seed's neighbor expansion). 0.0 for a neighbor with
    ///   no lexical score of its own.
    /// - `direct_ids`: set of SymbolIds that came directly from lexical
    ///   search, as opposed to graph-expanded neighbors.
    ///
    /// The 100-candidate cap admits neighbors round-robin across seeds (Plan
    /// 21 item 15) rather than exhausting one seed's neighbor list before
    /// moving to the next — the old first-come order meant a single
    /// high-fan-in seed (measured: 593 keycloak symbols have 80+ neighbors)
    /// could consume the entire cap and leave every other seed with zero
    /// expansion. Each seed's own neighbor list is still whatever order
    /// `neighbors()`'s SQL returns (effectively symbol-id hash order,
    /// `ORDER BY 1` — a separate, NOT-yet-fixed issue: reordering it
    /// meaningfully would mean threading an edge-kind priority through the
    /// shared `row_to_symbol` row-mapping, not attempted here). Direct hits
    /// themselves are still admitted in `lexical_hits`' own order, unaffected
    /// by round-robin — only neighbor admission is round-robin'd.
    ///
    /// True 2-hop expansion is a separate, NOT-attempted-here change:
    /// `neighbors()` only ever walks one hop regardless of the `hops`
    /// argument's value (it just changes this function's score-decay
    /// exponent) — per this plan's own execution notes, that's a genuine
    /// experiment to measure in isolation, not something to bundle
    /// silently into the cap fix.
    ///
    /// Separating direct hits from expanded neighbors is critical for reranking:
    /// direct hits keep their own retrieval score as the primary signal, while
    /// expanded neighbors rely on `graph_score` (personalized PageRank, not
    /// raw in_degree — see `personalized_pagerank`'s doc comment) to surface
    /// at all. Direct hits also get a damped `graph_score` credit in `rerank`
    /// (see `rerank`'s `graph_credit`), so this distinction is about which
    /// score dominates, not an exclusive either/or.
    fn expand_graph(
        &self,
        lexical_hits: &[(SymbolId, f32)],
        hops: u8,
        lang_filter: Option<&str>,
    ) -> anyhow::Result<ExpandedCandidates> {
        let mut candidates: HashSet<SymbolId> = HashSet::new();
        let mut lexical: HashMap<SymbolId, f32> = HashMap::new();
        let mut direct_ids: HashSet<SymbolId>   = HashSet::new();

        let decay = 0.6_f32.powi(hops as i32);

        // Direct hits are never capped — every lexical hit becomes a
        // candidate and a seed regardless of the 100-candidate limit, which
        // only governs how many *expanded neighbors* get admitted.
        let mut seed_queues: Vec<(f32, std::collections::VecDeque<crate::types::Symbol>)> = Vec::with_capacity(lexical_hits.len());
        for &(id, score) in lexical_hits {
            candidates.insert(id);
            lexical
                .entry(id)
                .and_modify(|s| *s = s.max(score))
                .or_insert(score);
            direct_ids.insert(id);

            let mut neighbors = self.graph_db.neighbors(id, hops)?;
            if let Some(lang) = lang_filter {
                neighbors.retain(|n| n.language.as_str().eq_ignore_ascii_case(lang));
            }
            seed_queues.push((score, neighbors.into()));
        }

        // Round-robin: take one neighbor from each seed's queue in turn,
        // instead of draining one seed's whole queue before moving to the
        // next — this is what actually fixes the starvation the old
        // first-come order caused, not just where the loop iterates from.
        'admit: loop {
            let mut any_progress = false;
            for (score, queue) in seed_queues.iter_mut() {
                let Some(neighbor) = queue.pop_front() else { continue };
                any_progress = true;
                if candidates.len() >= 100 {
                    break 'admit;
                }
                let decayed = *score * decay;
                candidates.insert(neighbor.id);
                lexical
                    .entry(neighbor.id)
                    .and_modify(|s| *s = s.max(decayed))
                    .or_insert(decayed);
            }
            if !any_progress {
                break;
            }
        }

        Ok((candidates, lexical, direct_ids))
    }

    fn enrich(
        &self,
        candidates: &HashSet<SymbolId>,
        lexical_scores: &HashMap<SymbolId, f32>,
        graph_scores: &HashMap<SymbolId, f32>,
        direct_ids: &HashSet<SymbolId>,
    ) -> anyhow::Result<Vec<EnrichedCandidate>> {
        let mut out = Vec::with_capacity(candidates.len());
        let mut ids: Vec<SymbolId> = candidates.iter().copied().collect();
        ids.sort_unstable();

        for symbol in self.graph_db.symbols_by_ids(&ids)? {
            let sym_id = symbol.id;
            let lexical_score = lexical_scores.get(&sym_id).copied().unwrap_or(0.0);
            let graph_score = graph_scores.get(&sym_id).copied().unwrap_or(0.0);
            let snippet   = self.source_loader.snippet(&symbol).unwrap_or_else(|e| {
                tracing::warn!(symbol = %symbol.qualified_name, "snippet load failed: {e}");
                String::new()
            });
            let is_direct_hit = direct_ids.contains(&sym_id);
            // Plan 22 item 1: `SourceLoader` already loaded (and cached) this
            // file's content for `snippet` above — the banner scan reuses
            // that cache, so this is essentially free beyond the first
            // candidate from any given file.
            let is_generated = self.source_loader.is_generated(&symbol.file_path);

            out.push(EnrichedCandidate {
                symbol,
                lexical_score,
                graph_score,
                snippet,
                is_direct_hit,
                is_generated,
            });
        }

        Ok(out)
    }

    fn rerank(
        &self,
        candidates: Vec<EnrichedCandidate>,
        final_n: usize,
        query_terms: &[String],
        query_is_about_tests: bool,
        query_is_about_generated: bool,
        pinned_ids: &HashSet<SymbolId>,
    ) -> Vec<RetrievalResult> {
        let max_graph_score = candidates
            .iter()
            .map(|c| c.graph_score)
            .fold(0f32, f32::max)
            .max(1e-6);

        // Improvement #3: BM25 — captures exact keyword matches in snippets.
        let bm25_scores: Vec<f32> = if self.config.rerank.enabled {
            // Tokenize once and use the same tokens for `avg_len` and each
            // `doc_len`: the average used to count whitespace words while the
            // per-document length counted `tokenize_name` tokens, so the length
            // ratio drifted with how identifier-dense each snippet was.
            let docs: Vec<Vec<String>> = candidates.iter().map(|c| tokenize_name(&c.snippet)).collect();
            let avg_len = docs.iter().map(Vec::len).sum::<usize>() as f32 / docs.len().max(1) as f32;
            docs.iter()
                .map(|d| bm25_score_terms(query_terms, d, avg_len,
                    self.config.rerank.k1, self.config.rerank.b))
                .collect()
        } else {
            vec![0.0; candidates.len()]
        };

        // Normalize BM25 scores to [0, 1].
        let max_bm25 = bm25_scores.iter().cloned().fold(0f32, f32::max);
        let bm25_norm: Vec<f32> = if max_bm25 > 0.0 {
            bm25_scores.iter().map(|&s| s / max_bm25).collect()
        } else {
            vec![0.0; bm25_scores.len()]
        };

        // Name-token scoring: split the symbol's camelCase/snake_case name into tokens
        // and score by fraction of tokens that appear in the query.
        // Applied to ALL candidates (direct + expanded) so navigation queries like
        // "create resume router" can surface `createResumeRouter` even when vec score
        // is borderline.
        let name_scores: Vec<f32> = if self.config.rerank.enabled {
            candidates.iter()
                .map(|c| name_token_match_score(query_terms, &c.symbol.name))
                .collect()
        } else {
            vec![0.0; candidates.len()]
        };

        let bonus = self.config.rerank.name_match_bonus;
        let (beta, gamma) = (self.config.beta, self.config.gamma);

        let mut scored: Vec<(f32, SymbolId, EnrichedCandidate)> = candidates
            .into_iter()
            .zip(bm25_norm)
            .zip(name_scores)
            .map(|((c, bm25), name_score)| {
                // Two independent keyword signals: live per-snippet BM25 (no
                // corpus IDF, computed fresh over just this candidate's text)
                // and `lexical_score` (SQLite FTS5's `bm25()`, real corpus-wide
                // IDF, computed once at candidate-generation time). Take the
                // stronger of the two rather than adding a separate weight —
                // both represent "keyword relevance" just computed two
                // different ways, and a new weight would break the
                // beta+gamma≈1.0 invariant `Config::validate()` checks.
                let keyword = bm25.max(c.lexical_score);
                // Test-file de-prioritization: a flat penalty (subtracted, not multiplied,
                // so it doesn't get amplified on already-low scores) for candidates whose
                // file looks like a test file — skipped when the query itself is about
                // tests, so "find the test for X" isn't punished for correctly wanting one.
                // Applied identically to direct hits and expanded neighbors: a test file is
                // equally undesirable regardless of how it entered the pool.
                // Plan 22 item 3: a pinned file is exempt from both the
                // test-file and generated-file penalties — the user named it
                // explicitly, so it isn't noise to de-prioritize the way an
                // incidentally-matched test/generated file is.
                let is_pinned = pinned_ids.contains(&c.symbol.id);
                let test_penalty = if !query_is_about_tests && !is_pinned && is_test_file(&c.symbol.file_path) {
                    TEST_FILE_PENALTY
                } else {
                    0.0
                };
                // Plan 22 item 1: a `mockgen`-style mock under a test
                // directory is one kind of noise, not two — `max`, not sum,
                // so it doesn't stack to a scale (0.30) never validated.
                let generated_penalty = if !query_is_about_generated && !is_pinned && c.is_generated {
                    GENERATED_FILE_PENALTY
                } else {
                    0.0
                };
                let test_penalty = test_penalty.max(generated_penalty);
                // "Guaranteed admission," not a hard override of ranking —
                // rank is still decided by the rest of this scoring closure,
                // plus this flat bonus (the named file isn't always where the
                // answer lives, per the plan's erp-58/erp-62 finding).
                let path_pin_bonus = if is_pinned { self.config.path_pin.bonus } else { 0.0 };
                let score = if c.is_direct_hit {
                    // Direct hits (lexical search) keep their own retrieval
                    // score as the primary signal, with keyword + name-token
                    // bonuses layered on top.
                    //
                    // A damped graph-score credit (see
                    // `DIRECT_HIT_GRAPH_CREDIT_DAMPING`'s doc comment for the
                    // dual-corpus sweep behind the current value): a direct
                    // hit can carry real structural centrality that would
                    // otherwise be discarded entirely (only expanded
                    // neighbors got a graph term) — measured case: a direct
                    // hit with 3x a competing expanded neighbor's raw PPR
                    // mass still lost to it by a razor-thin margin under
                    // live-BM25 noise, because this branch gave the direct
                    // hit zero credit for that centrality (checklist.md Plan
                    // 16, 2026-08-18 — originally shipped gated to
                    // `embedding_degraded` only; Plan 20 made that mode
                    // permanent, so the gate is gone, not the behavior).
                    // Damped below full `beta` so it stays a tie-break
                    // nudge, not a co-equal signal with keyword/name-token.
                    let graph_credit =
                        self.config.direct_hit_graph_credit_damping * beta * (c.graph_score / max_graph_score);
                    gamma * keyword + name_score * bonus + graph_credit - test_penalty + path_pin_bonus
                } else {
                    // `graph_score` (personalized PageRank, seeded from this
                    // query's own direct hits) replaces raw in-degree here —
                    // see `EnrichedCandidate::graph_score`'s doc comment for
                    // why a query-independent popularity measure was letting
                    // irrelevant hub functions dominate this term.
                    let graph_score_norm = c.graph_score / max_graph_score;
                    beta  * graph_score_norm
                        + gamma * keyword
                        + name_score * bonus
                        - test_penalty
                        + path_pin_bonus
                };
                (score, c.symbol.id, c)
            })
            .collect();

        // Sort by score, breaking ties by SymbolId. `candidates` arrived as a
        // `HashSet` iteration (Rust randomizes hash order per process), so
        // without an explicit tiebreak, equal-score candidates — routine
        // since several can share the same lexical/graph score — would
        // resolve to a different, unreproducible order on every run instead
        // of a stable one.
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

        scored
            .into_iter()
            .take(final_n)
            .enumerate()
            .map(|(rank, (score, _id, c))| {
                let file_path = c.symbol.file_path.clone();
                RetrievalResult {
                    rank: rank + 1,
                    score,
                    file_path,
                    snippet: c.snippet,
                    callers: Vec::new(),
                    callees: Vec::new(),
                    symbol:  c.symbol,
                }
            })
            .collect()
    }
}

/// How many times `top_k` lexical hits to fetch when a language filter will
/// discard the other-language ones.
const LANG_FILTER_OVERFETCH: usize = 4;

/// Mean of the judge's returned scores, in id order so the f32 sum is
/// reproducible across processes.
fn neutral_relevance(scores: &HashMap<SymbolId, f32>) -> f32 {
    let mut v: Vec<(SymbolId, f32)> = scores.iter().map(|(&k, &s)| (k, s)).collect();
    v.sort_unstable_by_key(|p| p.0);
    v.iter().map(|p| p.1).sum::<f32>() / v.len().max(1) as f32
}

/// Max chars of each candidate's snippet sent to Jev in `apply_jev` — not the
/// full (often 500-1500+ char) symbol body. Sized to keep a wide `jev.top_n`
/// batch's total payload small: 50 candidates × an untruncated snippet each
/// measurably regressed keycloak (36/40 → 34/40, see
/// `bench-reports/plan20-phase8-jev-keycloak-benchmark-2026-09-25.md`),
/// consistent with TypeSafe's own "keep chunks tight when scoring many
/// candidates at once" guidance for a 32K-token context budget.
const JEV_CANDIDATE_SNIPPET_CHARS: usize = 300;

/// Reassign 1-based `rank` after any post-`rerank()` re-sort or truncation
/// (Plan 20 Phase 8's `apply_jev`) — `rerank()` itself already does this for
/// its own output, but a later stage that reorders or shortens the list must
/// redo it or `RetrievalResult.rank` would silently go stale.
fn renumber_ranks(results: &mut [RetrievalResult]) {
    for (i, r) in results.iter_mut().enumerate() {
        r.rank = i + 1;
    }
}

#[async_trait::async_trait]
impl<G> QueryPipeline for RetrievalPipeline<G>
where
    G: GraphStore + Send + Sync,
{
    async fn query(&self, req: RetrievalQuery) -> anyhow::Result<Vec<RetrievalResult>> {
        RetrievalPipeline::query(self, req).await
    }

    fn jev_active(&self) -> bool {
        self.config.judge_provider().is_some() && self.jev.is_some()
    }

    fn candidate_top_k(&self) -> usize {
        self.config.candidate_top_k.max(1)
    }

    fn final_top_n(&self) -> usize {
        self.config.final_top_n.max(1)
    }

    fn judge_provider(&self) -> Option<&'static str> {
        self.config.judge_provider().filter(|_| self.jev.is_some()).map(|p| p.as_str())
    }
}

/// Common English function words dropped from every tokenized query — see
/// `tokenize_query`'s doc comment for why this exists. Not exhaustive, just
/// the high-frequency words that show up constantly in this kind of
/// "I want to... which function... I need to..." natural-language phrasing.
const STOPWORDS: &[&str] = &[
    "the", "and", "that", "this", "these", "those", "for", "with", "from", "into", "onto",
    "its", "their", "your", "you", "are", "was", "were", "been", "being", "does", "did",
    "have", "has", "had", "will", "would", "should", "could", "can", "not", "but", "than",
    "then", "also", "just", "only", "very", "really", "most", "more", "less", "much", "many",
    "some", "any", "all", "each", "every", "other", "such", "own", "same", "too", "about",
    "after", "before", "during", "while", "until", "unless", "because", "since", "though",
    "although", "want", "need", "find", "add", "know", "understand", "when", "what", "how",
    "who", "why", "here", "there", "now", "get", "using", "used", "use", "one",
];

/// Groups of near-synonymous programming verbs/nouns, expanded on the
/// *stemmed* form of each word (entries are written in natural form and
/// stemmed at lookup time in `synonym_expansions`, not hand-computed, so
/// correctness doesn't depend on knowing the stemmer's exact output). A
/// model-free bridge for queries that paraphrase rather than name an
/// identifier (e.g. "clears the editor content" → `resetEditor`) — curated
/// and necessarily incomplete, not a thesaurus. See `checklist.md` Plan 16.
const SYNONYM_GROUPS: &[&[&str]] = &[
    &["create", "build", "make", "init", "initialize", "new", "generate", "construct"],
    &["delete", "remove", "destroy", "clear", "wipe", "erase", "reset"],
    &["get", "fetch", "retrieve", "read", "load", "obtain"],
    &["set", "update", "modify", "change", "write", "mutate"],
    &["register", "attach", "bind", "subscribe", "listen"],
    &["parse", "deserialize", "decode"],
    &["export", "serialize", "encode", "dump"],
    &["validate", "check", "verify", "ensure"],
    &["handle", "process", "manage"],
    &["start", "begin", "launch"],
    &["stop", "end", "finish", "terminate", "halt"],
    &["enable", "activate"],
    &["disable", "deactivate"],
];

/// Stem one term (Porter/Snowball English algorithm via `rust-stemmers`) —
/// a fixed rule table, not a trained model: no weights to ship or load, and
/// `Stemmer::create` is cheap enough to call per term. Collapses inflections
/// ("resets"/"resetting"/"reset" → one root) so a query and an identifier
/// written in different grammatical forms still match — a distinct gap from
/// camelCase splitting, which doesn't help "resets" match `resetEditor`
/// since they're different strings even after splitting.
fn stem_term(term: &str) -> String {
    rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English)
        .stem(term)
        .into_owned()
}

/// Stemmed synonyms for `stemmed_term` (itself already stemmed — see
/// `tokenize_query`), or empty if it's not in any `SYNONYM_GROUPS` entry.
fn synonym_expansions(stemmed_term: &str) -> Vec<String> {
    for group in SYNONYM_GROUPS {
        if group.iter().any(|w| stem_term(w) == stemmed_term) {
            return group.iter()
                .map(|w| stem_term(w))
                .filter(|s| s != stemmed_term)
                .collect();
        }
    }
    vec![]
}

/// Whether an already-stemmed term matches a `STOPWORDS` entry, comparing
/// against each entry's own stem rather than its literal spelling — same
/// reasoning as `synonym_expansions`, and necessary for the same class of
/// reason: `STOPWORDS` is written in base form, so a literal-string check
/// against a stemmed query term would miss e.g. "understand"/"understood"
/// stemming differently, or catch a stopword only in its exact inflection.
fn is_stopword(stemmed_term: &str) -> bool {
    STOPWORDS.iter().any(|w| stem_term(w) == stemmed_term)
}

/// Tokenize a natural-language query into lowercase, stemmed terms for
/// lexical matching, expanded with known synonyms. Shared by candidate
/// generation (`GraphStore::lexical_search`) and reranking (BM25 + name-token
/// scoring) so both stages see exactly the same terms.
///
/// Stopword filtering matters because `GraphDb::lexical_search` joins every
/// term with `OR` into one FTS5 MATCH query — an unfiltered long question
/// lets common words ("the", "function", "state") dilute the few terms that
/// actually discriminate. See `checklist.md` Plan 16 for the measured case.
///
/// Shares `split_words`' boundary rules with `tokenize_name` deliberately:
/// several questions in this project's benchmark literally name the target
/// identifier in camelCase ("Which `$applyTransforms` function..."), and a
/// query-side tokenizer with different boundary rules than the index's would
/// tokenize that identifier differently than the index did, so it could
/// never match — even though the query spelled the answer out verbatim.
/// Words written fully in capitals in the *original* query text (before
/// `split_words` lowercases everything), e.g. `GET` in "restrict via HTTP
/// GET". `split_words` loses case information as it tokenizes, so this scan
/// has to run separately over the raw string — by the time `tokenize_query`
/// sees a token list, "GET" and "get" are indistinguishable.
///
/// Exists because "get" sits in `STOPWORDS` (correctly — as a generic verb
/// it's noise on almost every query, see the comment below on why it can't
/// just be removed) but is also the single most load-bearing word in a
/// question about HTTP method restriction, where it names the protocol verb
/// being blocked, not a generic "fetch something" request. Investigated
/// after Q04 of the GLM-5.3-Flash benchmark ("...restrict...via HTTP GET
/// and only allow POST...") scored 42% with the target functions never
/// entering the candidate pool — the indexed chunk text for those functions
/// already contained the right vocabulary (`@frappe.whitelist(methods=
/// ["POST"])`), so this isn't a missing-data bug, it's the query itself
/// losing its most specific term before search ever runs.
fn uppercase_acronyms(query: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut current = String::new();
    for ch in query.chars().chain(std::iter::once(' ')) {
        if ch.is_alphabetic() {
            current.push(ch);
        } else {
            if current.len() >= 2 && current.chars().all(|c| c.is_uppercase()) {
                out.insert(current.to_lowercase());
            }
            current.clear();
        }
    }
    out
}

/// Whole camelCase identifiers in the raw query text, lowercased and NOT stemmed.
/// `chunk_fts`'s `unicode61` tokenizer indexes an identifier like `getId` as the
/// single raw token `getid` (in `qualified_name` and the body), so this is the
/// only form that matches it exactly. Only runs with an internal case boundary
/// qualify: plain words and ALL-CAPS acronyms already work through the normal
/// path, and snake_case/kebab-case identifiers are split into their parts by
/// unicode61 itself. Deliberately not added to the synonym or rerank term sets.
fn identifier_terms(query: &str) -> Vec<String> {
    const MAX_TERMS: usize = 8;
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_alphanumeric() {
            i += 1;
        }
        let run = &chars[start..i];
        let has_boundary = (1..run.len()).any(|j| {
            let (p, c) = (run[j - 1], run[j]);
            let next_lower = run.get(j + 1).is_some_and(|n| n.is_lowercase());
            (p.is_lowercase() && c.is_uppercase())
                || (p.is_uppercase() && c.is_uppercase() && next_lower)
                || (p.is_ascii_digit() && c.is_uppercase())
        });
        if has_boundary && run.len() >= 4 {
            let term: String = run.iter().collect::<String>().to_lowercase();
            if !out.contains(&term) {
                out.push(term);
                if out.len() == MAX_TERMS {
                    break;
                }
            }
        }
    }
    out
}

/// Test-only convenience wrapper — `query()` (Stage 1) calls
/// `tokenize_query_split` directly since Plan 21 item 14 needs both halves
/// of its return value; this one just discards the synonym-only half.
#[cfg(test)]
fn tokenize_query(query: &str) -> Vec<String> {
    tokenize_query_split(query).0
}

/// Same output as `tokenize_query` (the combined, deduped original+synonym
/// term list, as `.0`), plus the synonym-only subset as `.1` — terms that
/// exist *only* because synonym expansion added them, not because the user
/// typed them or a different original term happened to stem to the same
/// value. Plan 21 item 14: `lexical_search` uses this split to down-weight
/// synonym-sourced FTS matches relative to the user's own words (see its
/// doc comment) instead of treating every expanded term as equally strong
/// evidence — a query using none of a symbol's real vocabulary but
/// happening to share a synonym-group word with it was previously scored
/// the same as an exact literal match.
fn tokenize_query_split(query: &str) -> (Vec<String>, Vec<String>) {
    // Case-sensitive acronym scan first — before `split_words` throws case
    // away — so a query's own "GET"/"PUT"/"DELETE" survives the stopword
    // filter below even though their lowercase forms would otherwise be
    // dropped as generic verbs. This never touches indexing (`tokenize_name`
    // has no equivalent exemption; identifiers are rarely spelled as a bare
    // all-caps English word) or the synonym-expansion path below, whose own
    // `get` exclusion is a deliberately separate, already-measured fix — an
    // acronym in the query text is a different, narrower signal than a verb
    // that merely stems to one of `get`'s synonyms.
    let acronyms: std::collections::HashSet<String> = uppercase_acronyms(query)
        .into_iter()
        .map(|w| stem_term(&w))
        .collect();

    // Stem *before* checking STOPWORDS, not after: filtering on the raw
    // pre-stem token let inflected stopwords slip through ("needs"/"gets"
    // aren't literal STOPWORDS entries, so they'd pass the old filter, then
    // stem down to "need"/"get" — both of which *are* stopwords). Comparing
    // post-stem against each STOPWORDS entry's own stem (`is_stopword`)
    // catches the inflected form too, not just the exact literal spelling.
    let base: Vec<String> = split_words(query)
        .into_iter()
        .map(|t| stem_term(&t))
        .filter(|t| acronyms.contains(t) || !is_stopword(t))
        .collect();

    let mut expanded = base.clone();
    let mut synonym_only: Vec<String> = Vec::new();
    for term in &base {
        for syn in synonym_expansions(term) {
            // `get` sits in both STOPWORDS and the get/fetch/retrieve/read/
            // load/obtain SYNONYM_GROUPS entry (the only overlap across all
            // groups) — without this check, expanding "read" silently
            // smuggled "get" back into the term set after the stopword
            // filter above had already dropped it, inflating every
            // getXxx()-named candidate's score on any query using one of
            // that group's other verbs (measured: readEditorState vs.
            // getActiveEditorState in bench-reports/lexical-semantic-vs-no-semantic-2026-08-07.md,
            // raw FTS5 rank #157 → #135 once both this leak and the
            // pre-stem filtering gap above are closed — real but not
            // sufficient on its own to clear top_k; see checklist.md Plan 16
            // for why the rest of that gap is structural, not this bug).
            if is_stopword(&syn) {
                continue;
            }
            if !expanded.contains(&syn) {
                expanded.push(syn.clone());
                synonym_only.push(syn);
            }
        }
    }
    (expanded, synonym_only)
}

// ── Plan 22 item 3: file-path pinning — query-text detection ────────────────

/// Strip backticks/quotes from both ends, then trailing sentence punctuation
/// from the right end only (deliberately *not* the left end too — that would
/// also eat a dotfile's leading `.`). Shared by `detect_path_mentions` and
/// `strip_path_mentions` so both sides agree on exactly what "the path
/// mention" text is.
fn normalize_path_token(raw: &str) -> &str {
    const SURROUND: &[char] = &['`', '\'', '"'];
    const TRAILING: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}'];
    raw.trim_matches(SURROUND).trim_end_matches(TRAILING)
}

/// True if `token` (already normalized via `normalize_path_token`) has the
/// shape of a path mention: contains a path separator, or is a bare
/// `name.ext` with a short (1-5 char) alphanumeric, non-numeric-only
/// extension. Deliberately loose — `graphrelay-core` can't hard-code a
/// 30+-language extension list (no dependency on `graphrelay-parser`); the
/// store lookup in `resolve_path_pins` is the real validator. A shape match
/// that resolves to zero indexed files (`session.getId`, `e.g.`, a bare URL)
/// costs one lookup and is then simply discarded — never pinned.
fn looks_like_path(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.contains('/') || token.contains('\\') {
        return true;
    }
    match token.rsplit_once('.') {
        Some((name, ext)) => {
            !name.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && !ext.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Plan 22 item 3: scan the raw query for path-like substrings *before*
/// `tokenize_query_split` shreds them into generic stems that can't match
/// `chunk_fts.file_path` (untokenized). Returns each detected token's
/// normalized text, deduplicated, in first-seen order (deterministic
/// ambiguity-policy processing).
fn detect_path_mentions(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in query.split_whitespace() {
        let t = normalize_path_token(raw);
        if looks_like_path(t) && seen.insert(t.to_string()) {
            out.push(t.to_string());
        }
    }
    out
}

/// Rebuild the query text with every token whose normalized form is in
/// `remove` dropped — used by "strip" mode (arm A) to keep Stage 1's
/// `lexical_search` from diluting its term set with a path mention that's
/// already guaranteed admission via pinning. Whitespace-only reconstruction
/// (not a precise substring removal) is fine: the result only ever feeds
/// `tokenize_query_split`, which itself just extracts words.
fn strip_path_mentions(query: &str, remove: &HashSet<String>) -> String {
    query
        .split_whitespace()
        .filter(|raw| !remove.contains(normalize_path_token(raw)))
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Improvement #3: BM25 scoring ─────────────────────────────────────────────

/// Fraction of `name` tokens that appear in `query_terms` [0.0, 1.0].
///
/// Splits the symbol name on camelCase boundaries and underscores, lowercases everything,
/// filters tokens shorter than 3 chars, then counts matches against the query term set.
fn name_token_match_score(query_terms: &[String], name: &str) -> f32 {
    let tokens = tokenize_name(name);
    if tokens.is_empty() { return 0.0; }
    let matched = tokens.iter().filter(|t| query_terms.contains(t)).count();
    matched as f32 / tokens.len() as f32
}

/// Flat score deduction for a candidate whose file looks like a test file (see
/// `is_test_file`), skipped when the query is about tests (see `query_mentions_tests`).
/// Subtracted rather than multiplied so it doesn't get amplified on already-low scores.
/// Sized relative to this project's observed rerank score range for direct hits
/// (roughly 0.3-0.9). Swept against 0.10/0.25 on the keycloak no-semantic benchmark
/// (warm-cache, 2 repeated runs per value): 0.10 → 33/40 (kc-sg-02 flips to a win, a
/// second task — `removeClientFromOfflineUserSession` — does not); 0.15 and 0.25 both
/// → 34/40, identical task set, kc-sg-02 and that second task both winning. 0.15 sits
/// inside that 0.15-0.25 plateau rather than at a knife-edge, so kept at the smaller
/// value in the plateau rather than 0.25.
const TEST_FILE_PENALTY: f32 = 0.15;

// `DIRECT_HIT_GRAPH_CREDIT_DAMPING`'s value and full sweep history moved to
// `graphrelay_core::config::default_direct_hit_graph_credit_damping` (Plan 22
// item 4 — config-driven so a future re-sweep is a config edit, not a
// rebuild). See that function's doc comment for the 2026-08-18/2026-09-25
// sweep tables this constant's 0.75 default came from.

/// Whether a file path looks like a test file — name/directory patterns, not content
/// inspection. Mirrors a validated technique from a comparable model-free retrieval tool:
/// a query almost never wants a test helper's incidental name-paraphrase to outrank the
/// real implementation it's testing (e.g. a JUnit `listApplicationsThirdParty` in some
/// `FooTest.java` beating the actual `getConsentsStream` production method it exercises,
/// purely on name overlap).
fn is_test_file(path: &std::path::Path) -> bool {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let file_lower = file_name.to_lowercase();
    let path_lower = path.to_string_lossy().to_lowercase();

    // Filename patterns: test_foo.py, foo_test.go, foo.test.ts, foo-spec.rb.
    if file_lower.starts_with("test_") || file_lower.starts_with("test.") {
        return true;
    }
    if suffix_before_extension_matches(&file_lower, &["_test", "_tests", "_spec", "_specs", ".test", ".spec"]) {
        return true;
    }
    // CamelCase suffix before the extension, case-SENSITIVE on the original name
    // (so "latest.kt"/"manifest.kt" — lowercase "test" — aren't caught): FooTest.java,
    // BarTests.kt, BazSpec.scala, QuxTestCase.java.
    if let Some(stem) = file_name.rsplit_once('.').map(|(s, _)| s) {
        for suffix in ["Test", "Tests", "TestCase", "Tester", "Spec", "Specs"] {
            if stem.ends_with(suffix) && stem.len() > suffix.len() {
                return true;
            }
        }
    }
    // Directory patterns.
    path_lower.contains("/test/") || path_lower.contains("/tests/")
        || path_lower.contains("/spec/") || path_lower.contains("/specs/")
        || path_lower.contains("/__tests__/")
}

/// Suffix-before-extension check without pulling in the `regex` crate for one helper.
fn suffix_before_extension_matches(file_lower: &str, suffixes: &[&str]) -> bool {
    if let Some(dot) = file_lower.rfind('.') {
        let stem = &file_lower[..dot];
        suffixes.iter().any(|s| stem.ends_with(s))
    } else {
        false
    }
}

/// Whether the query itself is *about* tests — gates `is_test_file`'s penalty off so a
/// legitimate "find the test for X" / "verify Y" query isn't penalized for correctly
/// wanting a test file. Checked against the raw query text (not the stemmed/synonym-
/// expanded `query_terms`) deliberately: `"verify"` sits in the same `SYNONYM_GROUPS` entry
/// as `"check"`/`"validate"`/`"ensure"`, so a stemmed-term check would treat any query
/// merely containing "check" as being "about tests" too. Word-boundary matched (not a raw
/// substring check) so e.g. "latest" doesn't false-positive on "test".
fn query_mentions_tests(text: &str) -> bool {
    const TEST_WORDS: &[&str] = &[
        "test", "tests", "testing", "spec", "specs", "verify", "verifies", "verifying",
    ];
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| TEST_WORDS.contains(&w))
}

/// Plan 22 item 1: filename convention for machine-generated code — a short,
/// conservative list of unambiguous codegen suffixes, deliberately not
/// per-language exhaustive (the content-banner scan in `source_loader` is
/// the general mechanism; this just catches the common cases where the
/// generator's own naming convention already says so). `pub` so
/// `SourceLoader::is_generated` (a sibling module) can call it before
/// falling back to the content scan.
pub fn is_generated_filename(path: &std::path::Path) -> bool {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let lower = file_name.to_lowercase();

    const SUFFIXES: &[&str] = &[
        ".pb.go", "_grpc.pb.go", ".pb.h", ".pb.cc",
        "_pb2.py", "_pb2_grpc.py",
        ".g.dart", ".freezed.dart",
        ".designer.cs", ".g.cs",
    ];
    if SUFFIXES.iter().any(|s| lower.ends_with(s)) {
        return true;
    }
    if lower.starts_with("zz_generated.") {
        return true;
    }
    // `*.generated.*` / `*_generated.*` — a "generated" segment anywhere
    // between dot/underscore-delimited parts (`foo.generated.go`,
    // `foo_generated_types.py`), not just as the literal suffix.
    lower.contains(".generated.") || lower.contains("_generated.") || lower.contains("_generated_")
}

/// Flat score deduction for a candidate whose file looks machine-generated
/// (see `is_generated_filename`/`SourceLoader::is_generated`), skipped when
/// the query is itself about generated code — same reasoning and shape as
/// `TEST_FILE_PENALTY`/`query_mentions_tests`. Corpus-scan-validated as a
/// regression-neutral value on all three benchmark corpora (none of which
/// contain real codegen output to show a *positive* case) rather than
/// independently swept — see `bench-reports/` for the item 1 write-up.
const GENERATED_FILE_PENALTY: f32 = 0.15;

/// Whether the query itself is about generated code — gates
/// `GENERATED_FILE_PENALTY` off, same reasoning as `query_mentions_tests`.
/// Word-boundary matched against the raw query text.
fn query_mentions_generated_code(text: &str) -> bool {
    const GENERATED_WORDS: &[&str] = &[
        "generated", "codegen", "proto", "protobuf", "stub", "stubs", "schema",
    ];
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| GENERATED_WORDS.contains(&w))
}

/// Split an identifier (or a whole qualified name) into lowercase, stemmed
/// tokens by camelCase and snake_case boundaries.
///
/// Examples (exact stems are `stem_term`'s implementation detail — what
/// matters is `tokenize_query` applies the same stemming, so both sides line up):
///   `createResumeRouter`         → stem("create"), stem("resume"), stem("router")
///   `routes::createResumeRouter` → stem("routes"), stem("create"), stem("resume"), stem("router")
///
/// Also used by `GraphDb::upsert_fts_batch` to decompose a full
/// `qualified_name` for FTS5 indexing, which is why `split_words` treats
/// `::`/`/`/`\` as boundaries too, not just `_`/`-`/`.`. `pub` so
/// graphrelay-store can call it directly.
pub fn tokenize_name(name: &str) -> Vec<String> {
    split_words(name).into_iter().map(|t| stem_term(&t)).collect()
}

/// Split text into lowercase sub-word tokens (>= 3 chars each), treating any
/// non-alphanumeric character and every camelCase transition as a boundary.
/// The single source of truth for decomposing both identifiers
/// (`tokenize_name`) and natural-language query text (`tokenize_query`) —
/// see `tokenize_query`'s doc comment for why they must agree.
///
/// Plan 21 item 13: two extra boundary rules beyond plain lowercase→uppercase,
/// both needing one character of lookahead (hence collecting into `chars`
/// up front instead of a plain `Chars` iterator):
/// - **Acronym-to-word**: an uppercase run followed by a lowercase letter
///   splits before the last uppercase char, so `isSSOAuthentication` yields
///   `sso`+`authentication` instead of one token (`SSOAuthentication`) that
///   only ever stemmed down to an unrecognizable blob (`ssoauthent`) — the
///   old rule only fired on a *single* lowercase→uppercase transition, so a
///   run of 2+ capitals (the acronym itself) never got an internal boundary.
/// - **Digit-to-uppercase**: `sha256UrlEncodedHash` yields `sha256`+`url`+
///   `encoded`+`hash` instead of gluing the digit run onto the next word
///   (`sha256url…`) — digits aren't `char::is_lowercase()`, so the original
///   lowercase→uppercase rule silently never fired at a digit/letter
///   boundary. Deliberately NOT splitting letter→digit (`sha` from `256`) —
///   unlike the digit→uppercase case, nothing in the audit's findings shows
///   that direction actually hurts a real query, and gluing a digit onto its
///   preceding letters (`sha256`, `utf8`) is usually the more natural unit.
fn split_words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev: Option<char> = None;

    for i in 0..chars.len() {
        let ch = chars[i];
        if !ch.is_alphanumeric() {
            if current.len() >= 3 {
                tokens.push(current.to_lowercase());
            }
            current.clear();
            prev = None;
            continue;
        }

        let next_is_lowercase = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
        let boundary = match prev {
            None => false,
            Some(p) => {
                (p.is_lowercase() && ch.is_uppercase())
                    || (p.is_uppercase() && ch.is_uppercase() && next_is_lowercase)
                    || (p.is_ascii_digit() && ch.is_uppercase())
            }
        };

        if boundary {
            if current.len() >= 3 {
                tokens.push(current.to_lowercase());
            }
            current.clear();
        }
        current.push(ch);
        prev = Some(ch);
    }
    if current.len() >= 3 {
        tokens.push(current.to_lowercase());
    }
    tokens
}

/// Compute BM25 score for pre-tokenized query terms against a document.
/// `query_terms` should be lowercased and filtered — hoist the tokenization
/// out of any per-candidate loop to avoid redundant allocation.
///
/// Plan 21 item 12: `doc` is tokenized with `tokenize_name` (the same
/// camelCase/snake_case-splitting, stemming pipeline `query_terms` was
/// already built with via `tokenize_query`) instead of a plain
/// non-alphanumeric split — previously `doc_words` were split but never
/// stemmed or camelCase-decomposed, so a stemmed query term like `"creat"`
/// (from "creates"/"created"/"creating") could never match a document's
/// literal `"created"`, and an identifier like `resetEditor` embedded in a
/// snippet stayed one opaque token instead of splitting into `reset`+`editor`.
fn bm25_score_terms(query_terms: &[String], doc_words: &[String], avg_doc_len: f32, k1: f32, b: f32) -> f32 {
    if query_terms.is_empty() { return 0.0; }

    let doc_len = doc_words.len() as f32;

    let mut score = 0.0f32;
    for term in query_terms {
        let tf = doc_words.iter().filter(|w| w.as_str() == term.as_str()).count() as f32;
        if tf == 0.0 { continue; }
        // BM25 TF component (IDF approximated as 1.0 — single-document scoring)
        let tf_norm = (tf * (k1 + 1.0))
            / (tf + k1 * (1.0 - b + b * doc_len / avg_doc_len.max(1.0)));
        score += tf_norm;
    }
    score
}

// ── Personalized PageRank (model-free "semantic reach" via graph structure) ─

/// Restart probability for `personalized_pagerank` — matches Tool A's
/// documented value for the same technique ("Random-Walk-with-Restart,
/// personalized PageRank, α=0.25"), giving this a validated reference point
/// rather than a guess.
const PPR_RESTART_PROB: f32 = 0.25;
/// Caps how many nodes carry mass forward between iterations (truncating to
/// the top-mass nodes), bounding cost independently of how large or densely
/// connected the reachable subgraph is. Matches `expand_graph`'s existing
/// 100-candidate cap.
const PPR_MAX_NODES: usize = 100;

/// Personalized PageRank (random-walk-with-restart) over the call graph,
/// seeded from `seeds` (SymbolId → restart weight — typically a query's
/// direct hits, weighted by their own vec/lexical strength).
///
/// Model-free "semantic reach" via graph structure rather than vocabulary or
/// embeddings: a function structurally adjacent to a matched one can surface
/// even with zero shared vocabulary — the same technique Tool A uses
/// *instead of* vector search. This doesn't replace embeddings; it amplifies
/// whatever seeds the lexical/vector stages already found, and returns zero
/// mass on a query with no seeds to start from.
///
/// Replaces raw in-degree as `rerank`'s graph-centrality signal for expanded
/// neighbors: in-degree is a global, query-independent popularity measure, so
/// a function called from everywhere scored the same regardless of the query
/// — letting hub functions dominate once lexical signal was diluted. PPR mass
/// only exists because it propagated from *this* query's own seeds.
///
/// Follows only `callees_of` at each step (directed — mass flows to what a
/// function calls, never back to its callers), NOT the undirected
/// `callers_of + callees_of` combination `expand_graph` uses for candidate
/// *discovery*. Deliberate, not an oversight: an undirected version of this
/// walk made hub-dominance *worse* — a hub reached by many irrelevant
/// degree-1 callers just bounces the mass it receives straight back next
/// iteration (nowhere else for a degree-1 node to send it), so it retains
/// disproportionate mass precisely because of its irrelevant callers, a real
/// property of unrestricted random walks (stationary mass ∝ degree). A
/// callees-only walk never traverses into an irrelevant caller at all, so it
/// can't reflect anything back. See `checklist.md` Plan 16.
fn personalized_pagerank<G: GraphStore>(
    graph: &G,
    seeds: &HashMap<SymbolId, f32>,
    restart_prob: f32,
    iterations: usize,
    max_nodes: usize,
    early_stop_epsilon: Option<f32>,
) -> HashMap<SymbolId, f32> {
    ppr_core(
        |id| graph.callee_ids_of(id).unwrap_or_default(),
        seeds,
        restart_prob,
        iterations,
        max_nodes,
        early_stop_epsilon,
    )
}

/// `personalized_pagerank`'s numeric core, parameterized on the neighbor lookup so
/// its accumulation order can be tested without a full `GraphStore`.
fn ppr_core(
    callees: impl Fn(SymbolId) -> Vec<SymbolId>,
    seeds: &HashMap<SymbolId, f32>,
    restart_prob: f32,
    iterations: usize,
    max_nodes: usize,
    early_stop_epsilon: Option<f32>,
) -> HashMap<SymbolId, f32> {
    if seeds.is_empty() {
        return HashMap::new();
    }
    // Every float accumulation below walks nodes in ascending-id order, never
    // `HashMap` order: f32 addition is not associative and a HashMap's iteration
    // order is seeded per process, so summing in map order flipped near-tied
    // results between two runs over the *same* database (found by the Plan 24
    // upgrade-vs-fresh comparison; invisible to same-process repeat tests).
    let mut seed_list: Vec<(SymbolId, f32)> = seeds.iter().map(|(&id, &w)| (id, w)).collect();
    seed_list.sort_unstable_by_key(|p| p.0);
    let seed_sum: f32 = seed_list.iter().map(|p| p.1).sum::<f32>().max(1e-6);
    let personalization_list: Vec<(SymbolId, f32)> =
        seed_list.iter().map(|&(id, w)| (id, w / seed_sum)).collect();

    let mut scores: HashMap<SymbolId, f32> = personalization_list.iter().copied().collect();
    let mut neighbor_cache: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();

    for _ in 0..iterations {
        let mut next: HashMap<SymbolId, f32> = HashMap::new();
        for &(id, w) in &personalization_list {
            *next.entry(id).or_insert(0.0) += restart_prob * w;
        }
        let mut current: Vec<(SymbolId, f32)> = scores.iter().map(|(&id, &m)| (id, m)).collect();
        current.sort_unstable_by_key(|p| p.0);
        for &(id, mass) in &current {
            if mass <= 1e-6 {
                continue;
            }
            let neighbors = neighbor_cache.entry(id).or_insert_with(|| {
                let mut ns: Vec<SymbolId> = callees(id);
                ns.sort_unstable();
                ns.dedup();
                ns
            });
            if neighbors.is_empty() {
                // Dangling node: return its mass to the restart distribution
                // rather than letting it vanish — standard PPR handling.
                for &(sid, w) in &personalization_list {
                    *next.entry(sid).or_insert(0.0) += (1.0 - restart_prob) * mass * w;
                }
                continue;
            }
            let share = (1.0 - restart_prob) * mass / neighbors.len() as f32;
            for &nid in neighbors.iter() {
                *next.entry(nid).or_insert(0.0) += share;
            }
        }
        if next.len() > max_nodes {
            let mut ranked: Vec<(SymbolId, f32)> = next.into_iter().collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            ranked.truncate(max_nodes);
            next = ranked.into_iter().collect();
        }
        // Plan 22 item 4: optional early stop once the L1 delta between
        // consecutive iterations drops below `early_stop_epsilon` — computed
        // over the union of both iterations' keys (truncation to
        // `max_nodes` means the keyset itself can change between rounds,
        // not just the masses). `None` (the default) always runs the full
        // `iterations` count, matching pre-item-4 behavior exactly.
        if let Some(eps) = early_stop_epsilon {
            let mut keys: Vec<SymbolId> = scores.keys().chain(next.keys()).copied().collect();
            keys.sort_unstable();
            keys.dedup();
            let delta: f32 = keys
                .iter()
                .map(|k| (next.get(k).copied().unwrap_or(0.0) - scores.get(k).copied().unwrap_or(0.0)).abs())
                .sum();
            scores = next;
            if delta < eps {
                break;
            }
        } else {
            scores = next;
        }
    }
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    /// f32 addition is not associative and `HashMap` iteration order is seeded per
    /// map, so accumulating PageRank mass in map order made the last bits — and any
    /// near-tie ranked on them — vary from run to run. The accumulation must walk
    /// nodes in id order, so identical inputs give bit-identical scores.
    #[test]
    fn pagerank_scores_do_not_depend_on_hash_map_iteration_order() {
        // A dense little graph whose shares (mass / degree) are not exactly representable.
        let mut graph: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        for id in 1..=60u64 {
            let out: Vec<SymbolId> = (1..=60u64).filter(|n| *n != id && (n * 7 + id * 3) % 5 < 3).collect();
            graph.insert(id, out);
        }
        let seeds: HashMap<SymbolId, f32> =
            (1..=25u64).map(|i| (i * 2, 0.013 + (i as f32) * 0.0071)).collect();
        let run = || {
            let s = ppr_core(|id| graph.get(&id).cloned().unwrap_or_default(), &seeds, 0.25, 12, 10_000, None);
            let mut v: Vec<(SymbolId, u32)> = s.into_iter().map(|(k, x)| (k, x.to_bits())).collect();
            v.sort_unstable();
            v
        };
        let first = run();
        for _ in 0..200 {
            assert_eq!(run(), first, "PageRank scores changed between identical runs");
        }
    }

    #[test]
    fn identifier_terms_emits_whole_camel_case_identifiers_only() {
        assert_eq!(identifier_terms("Which getId method returns the id?"), vec!["getid"]);
        assert_eq!(identifier_terms("call toUI and getOS"), vec!["toui", "getos"]);
        assert_eq!(identifier_terms("$applyTransforms and readEditorState"), vec!["applytransforms", "readeditorstate"]);
        assert_eq!(identifier_terms("IOError happens"), vec!["ioerror"]);
        assert!(identifier_terms("plain words HTTP GET only").is_empty(), "no case boundary, no term");
        assert!(identifier_terms("snake_case_name and kebab-case").is_empty(), "split by the index itself");
        assert_eq!(identifier_terms("getId getId").len(), 1, "deduplicated");
    }

    #[test]
    fn tokenize_name_splits_qualified_names_on_module_delimiters_too() {
        // Expected values are the real stemmed output (`rust_stemmers`'s
        // English/Porter algorithm), not the raw words — e.g. "routes" stems
        // to "rout", "create" to "creat". Deliberately pinned to actual
        // output rather than guessed, since stemmer quirks are exactly the
        // kind of thing worth having a regression test catch.
        assert_eq!(
            tokenize_name("routes::createResumeRouter"),
            vec!["rout", "creat", "resum", "router"],
        );
        assert_eq!(
            tokenize_name("pkg/file.rs::internalMarkSiblingsAsDirty"),
            vec!["pkg", "file", "intern", "mark", "sibl", "dirti"],
            "'rs' and 'as' are individually < 3 chars and get dropped, everything else survives",
        );
    }

    #[test]
    fn tokenize_name_splits_an_acronym_run_from_the_word_that_follows_it() {
        // Plan 21 item 13, real audit finding: the old rule only fired on a
        // single lowercase-to-uppercase transition, so a run of capitals
        // (an acronym) never got an internal boundary — `isSSOAuthentication`
        // stayed glued as one `SSOAuthentication` token that only ever
        // stemmed down to an unrecognizable `ssoauthent`. Splitting before
        // the acronym's last capital (when followed by a lowercase letter)
        // fixes it: "sso" ends up short enough to survive un-stemmed (3
        // chars, at the length filter's boundary), "authentication" stems
        // normally. "is" (2 chars) is dropped by the length filter either way.
        assert_eq!(
            tokenize_name("isSSOAuthentication"),
            vec!["sso", "authent"],
        );
    }

    #[test]
    fn tokenize_name_splits_a_digit_run_from_the_capitalized_word_after_it() {
        // Plan 21 item 13, real audit finding: digits aren't
        // `char::is_lowercase()`, so the original camelCase rule never fired
        // at a digit-to-uppercase-letter boundary — `sha256UrlEncodedHash`
        // stayed glued as `sha256url…` instead of `sha256`+`url`+`encoded`+
        // `hash`. Deliberately NOT splitting the leading letters from the
        // digit run itself (`sha` from `256`) — gluing a digit onto its
        // preceding letters is usually the more natural identifier unit.
        assert_eq!(
            tokenize_name("sha256UrlEncodedHash"),
            vec!["sha256", "url", "encod", "hash"],
        );
        assert_eq!(
            tokenize_name("OAuth2DeviceCodeModel"),
            vec!["auth2", "devic", "code", "model"],
            "the acronym rule splits the leading 'O' off first ('OA' is an uppercase run \
             followed by lowercase 'u', same as the SSO case above — 'O' alone is under the \
             3-char floor and dropped); the digit-to-uppercase rule then splits 'Auth2' from \
             'Device' — this is a real improvement over the pre-item-13 behavior, which glued \
             the digit run onto 'Device' as one 'oauth2device' token",
        );
    }

    #[test]
    fn tokenize_query_lowercases_and_drops_short_and_punct_tokens() {
        // "reset" is itself a member of `SYNONYM_GROUPS`' delete/clear group,
        // so its synonyms are expected in the output too — see
        // `tokenize_query_drops_common_stopwords` for the same effect on
        // "clear". "please" stems to "pleas".
        assert_eq!(
            tokenize_query("Reset the Editor-State, please!"),
            vec!["reset", "editor", "state", "pleas", "delet", "remov", "destroy", "clear", "wipe", "eras"],
            "'the' is a stopword and should be dropped; 'reset' expands via its synonym group",
        );
    }

    #[test]
    fn tokenize_query_drops_common_stopwords() {
        // "clears" stems to "clear", which is itself a member of
        // `SYNONYM_GROUPS`' delete/remove/destroy/clear/wipe/erase/reset
        // group — so its synonyms (stemmed, minus "clear" itself) are
        // expected in the output too.
        assert_eq!(
            tokenize_query("I want to find the function that clears the editor state"),
            vec!["function", "clear", "editor", "state", "delet", "remov", "destroy", "wipe", "eras", "reset"],
        );
    }

    #[test]
    fn tokenize_query_keeps_uppercase_get_as_an_http_verb_but_drops_lowercase_get() {
        // "GET" is written in caps because it names the HTTP method, not the
        // generic verb — it must survive the stopword filter that drops
        // "get" everywhere else. "restrict"/"http" aren't stopwords or
        // synonym-group members, so they pass through as-is; "get" stems to
        // itself.
        let t = tokenize_query("restrict via HTTP GET requests");
        assert!(t.contains(&"get".to_string()), "uppercase GET should survive as an HTTP verb: {t:?}");

        // Same word, lowercase, ordinary verb usage — still dropped.
        let t = tokenize_query("get the current status");
        assert!(!t.contains(&"get".to_string()), "lowercase get should still be filtered as a stopword: {t:?}");
    }

    #[test]
    fn tokenize_query_splits_underscored_identifiers_same_as_tokenize_name() {
        // Reversed from an earlier version of this test, which asserted the
        // OPPOSITE (underscore preserved, one token) — that was itself a bug
        // of the same class the camelCase-splitting fix addresses: the index
        // (`tokenize_name`) always decomposes on underscore too
        // (`internal_mark_siblings` → three separate stemmed tokens), so a
        // query keeping it as one blob could never match that decomposition
        // regardless of how it was stemmed. `tokenize_query` and
        // `tokenize_name` must use the same boundary rules (`split_words`)
        // for either one to be able to match the other.
        assert_eq!(
            tokenize_query("internal_mark_siblings"),
            tokenize_name("internal_mark_siblings"),
        );
    }

    #[test]
    fn tokenize_query_filters_terms_of_length_two_or_less() {
        assert_eq!(tokenize_query("a an if cat fn"), vec!["cat"]);
    }

    #[test]
    fn tokenize_query_empty_input_yields_empty_terms() {
        assert!(tokenize_query("").is_empty());
        assert!(tokenize_query("   ").is_empty());
    }

    #[test]
    fn stem_term_collapses_common_inflections_to_one_root() {
        // The exact motivating case: `lex-feat-02`'s benchmark query says
        // "resets" where the target identifier is `resetEditor` — a mismatch
        // camelCase-splitting alone can't fix (`resets` ≠ `reset` as plain
        // strings). Stemming both sides the same way closes it.
        assert_eq!(stem_term("resets"), stem_term("reset"));
        assert_eq!(stem_term("resetting"), stem_term("reset"));
        assert_eq!(stem_term("created"), stem_term("creates"));
    }

    #[test]
    fn tokenize_query_expands_query_terms_via_synonym_groups() {
        // No embeddings, no LLM call: this is the model-free technique for
        // bridging "clears the entire editor content" → `resetEditor` sharing
        // zero literal vocabulary, the exact paraphrase gap that motivated
        // this whole fix chain.
        let terms = tokenize_query("please clear the cache");
        for expected in ["remov", "delet", "wipe", "reset"] {
            assert!(terms.contains(&expected.to_string()), "missing synonym expansion: {expected}");
        }
    }

    #[test]
    fn tokenize_query_does_not_expand_terms_outside_any_synonym_group() {
        let terms = tokenize_query("compute the checksum");
        assert_eq!(terms, vec!["comput", "checksum"], "neither term is in SYNONYM_GROUPS — no expansion expected");
    }

    #[test]
    fn tokenize_query_splits_a_literally_named_camelcase_identifier() {
        // The exact real bug, found by directly reconstructing and running
        // the FTS5 query against a live index rather than trusting unit
        // tests alone: many of this benchmark's questions literally name the
        // target identifier ("Which $applyTransforms function...", "Which
        // readEditorState function..."). Before `tokenize_query` shared
        // `split_words` with `tokenize_name`, `$applyTransforms` tokenized
        // as one opaque blob ("applytransforms") that could never match the
        // index's separately camelCase-decomposed "appli"+"transform"
        // tokens — the identifier the query named verbatim was invisible in
        // its own results.
        let query_terms = tokenize_query("Which $applyTransforms function runs registered transforms?");
        let name_terms = tokenize_name("applyTransforms");
        for t in &name_terms {
            assert!(
                query_terms.contains(t),
                "query naming `applyTransforms` verbatim must decompose to include {t:?} \
                 (name tokens: {name_terms:?}, query tokens: {query_terms:?})"
            );
        }
    }

    // ── Plan 22 item 1: generated-file penalty query-gate ────────────────

    #[test]
    fn query_mentions_generated_code_matches_gate_words() {
        assert!(query_mentions_generated_code("regenerate the protobuf stub"));
        assert!(query_mentions_generated_code("what does this generated file do"));
        assert!(query_mentions_generated_code("find the codegen entry point"));
        assert!(query_mentions_generated_code("update the schema"));
    }

    #[test]
    fn query_mentions_generated_code_does_not_false_positive_on_unrelated_queries() {
        assert!(!query_mentions_generated_code("find the function that clears editor state"));
        assert!(!query_mentions_generated_code("how does authentication work"));
    }

    #[test]
    fn is_generated_filename_matches_known_conventions() {
        use std::path::Path;
        assert!(is_generated_filename(Path::new("api_grpc.pb.go")));
        assert!(is_generated_filename(Path::new("messages_pb2.py")));
        assert!(!is_generated_filename(Path::new("main.go")));
    }

    // ── Plan 22 item 3: file-path pinning detection ──────────────────────

    #[test]
    fn detects_a_repo_relative_path() {
        assert_eq!(
            detect_path_mentions("what does accounts/doctype/account/account.py do"),
            vec!["accounts/doctype/account/account.py"],
        );
    }

    #[test]
    fn detects_a_bare_filename() {
        assert_eq!(detect_path_mentions("explain LexicalEditor.ts"), vec!["LexicalEditor.ts"]);
    }

    #[test]
    fn detects_a_windows_style_path() {
        assert_eq!(
            detect_path_mentions(r"look at src\billing\status.py please"),
            vec![r"src\billing\status.py"],
        );
    }

    #[test]
    fn strips_surrounding_backticks() {
        assert_eq!(detect_path_mentions("see `foo/bar.py` for details"), vec!["foo/bar.py"]);
    }

    #[test]
    fn strips_surrounding_quotes() {
        assert_eq!(detect_path_mentions(r#"see "foo/bar.py" for details"#), vec!["foo/bar.py"]);
    }

    #[test]
    fn strips_trailing_sentence_punctuation_not_leading() {
        // Trailing comma/period stripped; a leading dot (dotfile-shaped) is
        // preserved since it's not "trailing".
        assert_eq!(detect_path_mentions("check config.toml, then run it."), vec!["config.toml"]);
    }

    #[test]
    fn a_url_like_string_has_path_shape_but_is_left_to_the_store_lookup_to_reject() {
        // Detection is deliberately loose (contains '/') — resolving this to
        // zero indexed files is `resolve_pinned_files`'s job (see the
        // `path_pin_unresolved_mention_pins_nothing_and_does_not_error`
        // pipeline test), not detection's. This test just pins that detection
        // itself doesn't special-case URLs away.
        assert_eq!(
            detect_path_mentions("see https://example.com/foo.py for details"),
            vec!["https://example.com/foo.py"],
        );
    }

    #[test]
    fn plain_words_and_short_extensions_are_not_falsely_shaped_as_paths() {
        assert_eq!(detect_path_mentions("what is the meaning of life"), Vec::<String>::new());
        // "3.14" — extension is all-digit, deliberately excluded.
        assert_eq!(detect_path_mentions("pi is roughly 3.14"), Vec::<String>::new());
    }

    #[test]
    fn duplicate_mentions_are_deduplicated_in_first_seen_order() {
        assert_eq!(
            detect_path_mentions("compare foo.py against foo.py again"),
            vec!["foo.py"],
        );
    }

    #[test]
    fn strip_path_mentions_removes_only_the_named_tokens() {
        let mut remove = HashSet::new();
        remove.insert("foo/bar.py".to_string());
        assert_eq!(
            strip_path_mentions("what does foo/bar.py actually do", &remove),
            "what does actually do",
        );
    }
}
