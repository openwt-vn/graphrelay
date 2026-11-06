use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub index: IndexConfig,

    #[serde(default)]
    pub retrieval: RetrievalConfig,

    #[serde(default)]
    pub watcher: WatcherConfig,

    /// Plan 12 C2: optional SCIP semantic layer configuration.
    #[serde(default)]
    pub scip: ScipConfig,
}

/// Plan 12 C2: SCIP semantic indexing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScipConfig {
    /// Path to the pre-computed `.scip` index file (relative to project root).
    /// Default: `.graphrelay/index.scip`
    pub scip_path: PathBuf,
    /// Automatically run the appropriate SCIP indexer after full tree-sitter indexing.
    /// **Off by default** (Plan 24 item 1.8 A/B gate, 2026-09-29): with the SCIP
    /// layer fixed, SCIP-on vs SCIP-off measured +0.044 normalized MRR on lexical,
    /// +0.004 on erpnext and +0.009 on keycloak — real but small and corpus-
    /// dependent, and not worth launching an external compiler-backed indexer
    /// (tens of minutes on a large Maven repo) unasked. A `.scip` file that already
    /// exists (e.g. from `gr scip index`) is still ingested regardless.
    /// Supported indexers: rust-analyzer (Rust), scip-python (Python),
    /// scip-typescript (TS), scip-java (Java), scip-go (Go); gracefully skipped
    /// when the binary is not in PATH.
    #[serde(default)]
    pub auto_run: bool,
}

fn default_true() -> bool { true }

// ── Per-field serde defaults (allow partial TOML sections) ──────────────────
fn default_port()         -> u16          { 3030 }
fn default_bind()         -> String       { "127.0.0.1".to_string() }
fn default_root()         -> PathBuf      { PathBuf::from(".") }
fn default_ignore_index() -> Vec<String>  {
    vec![".git/**","node_modules/**","target/**","dist/**","*.min.js","*.lock",
         ".venv/**","venv/**","__pycache__/**","vendor/**"]
        .into_iter().map(String::from).collect()
}
// Every language `graphrelay_parser::LanguageRegistry::with_defaults()`
// registers — keep in sync with that list (checked by
// `graphrelay-watcher`'s `default_languages_cover_every_registered_language`
// integration test, which will fail loudly if the two drift apart).
fn default_languages()    -> Vec<String>  {
    vec![
        "rust", "typescript", "javascript", "python", "go", "java", "c", "cpp",
        "csharp", "ruby", "kotlin", "swift", "php", "scala", "lua", "vue", "svelte",
        "bash", "zig", "elixir", "julia", "fortran", "ocaml", "commonlisp",
        "verilog", "pascal", "dart", "objectivec", "groovy", "powershell", "json",
    ]
        .into_iter().map(String::from).collect()
}
fn default_candidate_top_k() -> usize        { 20 }
fn default_expand_hops()  -> u8           { 1 }
fn default_final_top_n()  -> usize        { 5 }
// Plan 20 (remove vector search): carried over from the pre-removal
// `embedding_degraded` renormalization — `beta/(beta+gamma)`, `gamma/(beta+gamma)`
// off the old defaults `beta=0.2, gamma=0.1` — since that's the only
// empirically-validated point (`bench-reports/keycloak-cross-repo-generalization-2026-08-20.md`
// et al.), not a fresh guess. Re-tune only against a real benchmark run, not
// in the abstract.
fn default_beta()         -> f32          { 0.667 }
fn default_gamma()        -> f32          { 0.333 }
fn default_rerank_k1()    -> f32          { 1.5 }
fn default_rerank_b()     -> f32          { 0.75 }
// Plan 20 Phase 8 — optional cloud-judge rerank signal (TypeSafe AI's Jev).
// Off by default: this is a supplemental, opt-in signal on top of the
// always-on lexical+graph pipeline, not a replacement for it — see
// `plans/20-remove-vector-search.md` Phase 8.
fn default_jev_base_url() -> String       { "https://api.typesafe.ai/v1/systemone".to_string() }
fn default_jev_model()    -> String       { "jev-latest".to_string() }
fn default_jev_top_n()    -> usize        { 50 }
fn default_jev_weight()   -> f32          { 0.15 }
fn default_jev_timeout_ms() -> u64        { 3000 }
fn default_laya_base_url() -> String      { "http://127.0.0.1:8000/v1/systemone".to_string() }
fn default_laya_top_n()    -> usize       { 20 }
fn default_laya_weight()   -> f32         { 0.05 }
fn default_laya_timeout_ms() -> u64       { 15_000 }
fn default_laya_concurrency() -> usize    { 4 }
fn default_debounce_ms()  -> u64          { 50 }
fn default_max_queue()    -> usize        { 1000 }
fn default_ignore_watcher() -> Vec<String> {
    vec![".git/**","node_modules/**","target/**",".venv/**","venv/**","__pycache__/**","vendor/**"]
        .into_iter().map(String::from).collect()
}

impl Default for ScipConfig {
    fn default() -> Self {
        Self {
            scip_path: PathBuf::from(".graphrelay/index.scip"),
            auto_run:  false,  // opt-in: see the field's doc comment (Plan 24 item 1.8)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
    pub bind: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 3030,
            bind: "127.0.0.1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    #[serde(default = "default_root")]
    pub root:      PathBuf,
    #[serde(default = "default_ignore_index")]
    pub ignore:    Vec<String>,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default)]
    pub data_dir:  Option<PathBuf>,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            root:      PathBuf::from("."),
            ignore:    default_ignore_index(),
            languages: default_languages(),
            data_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    #[serde(default = "default_candidate_top_k")]
    pub candidate_top_k:  usize,
    #[serde(default = "default_expand_hops")]
    pub expand_hops:   u8,
    #[serde(default = "default_final_top_n")]
    pub final_top_n:   usize,
    #[serde(default = "default_beta")]
    pub beta:          f32,
    #[serde(default = "default_gamma")]
    pub gamma:         f32,
    /// Improvement #3: BM25 reranking of lexical/graph candidates.
    /// Scores retrieved code snippets with BM25 against the query terms, then blends
    /// the score with the FTS5 lexical score and graph-centrality signal.
    #[serde(default)]
    pub rerank: RerankConfig,
    /// Plan 20 Phase 8: optional cloud-judge rerank signal (TypeSafe AI's Jev).
    #[serde(default)]
    pub jev: JevConfig,
    /// Alternative self-hosted judge; see `LayaConfig`.
    #[serde(default)]
    pub laya: LayaConfig,
    /// Plan 22 item 3: file-path pinning. **Shipped as arm A ("strip") after
    /// a three-corpus strict-scorer measurement** (see
    /// `bench-reports/plan22-item3-*.md` — an earlier same-day measurement
    /// wrongly concluded "not shipped" because a pre-existing per-project
    /// `.graphrelay/config.toml` was silently shadowing the top-level
    /// `graphrelay.toml` used for the sweep, meaning path-pin mode was `Off`
    /// in every "arm" tested; re-measured correctly, arm A shows a real,
    /// reproduced, targeted improvement on erpnext and lexical with no
    /// off-target regressions). Config-driven (not an env var) so a future
    /// arm/constant re-sweep is just a config edit + daemon restart with no
    /// rebuild.
    #[serde(default)]
    pub path_pin: PathPinConfig,
    /// Plan 22 item 4: personalized PageRank iteration count and early-stop
    /// epsilon, config-driven for the same reason `path_pin` is — a sweep
    /// value change is a config edit + daemon restart, no rebuild. Defaults
    /// reproduce the pre-item-4 hardcoded constants exactly.
    #[serde(default)]
    pub ppr: PprConfig,
    /// Damping applied to a direct hit's own graph-centrality credit in
    /// `rerank` — coupled with `ppr.iterations` (both were tuned together;
    /// see `DIRECT_HIT_GRAPH_CREDIT_DAMPING`'s doc comment in pipeline.rs),
    /// hence config-driven alongside it for Plan 22 item 4's joint sweep.
    #[serde(default = "default_direct_hit_graph_credit_damping")]
    pub direct_hit_graph_credit_damping: f32,
}

/// Damping applied to a direct hit's own graph-centrality credit (see the
/// `graph_credit` computation in `rerank`'s scoring closure, `pipeline.rs`).
/// Originally shipped gated to `embedding_degraded` mode only (Plan 16,
/// 2026-08-18, swept there against 0.25/0.75 on that gated baseline, 0.5
/// kept); Plan 20 (2026-09-25) made that mode permanent, so the gate
/// disappeared but the value was never re-swept against the now-always-on
/// reality.
///
/// Re-swept 2026-09-25 (`bench-reports/plan20-post-removal-regression-check-2026-09-25.md`'s
/// "Item 2" section): 0/0.25/0.5/0.75/1.0, dual-corpus (lexical + keycloak,
/// Jev off), pass@5 + summed-MRR-over-top-30 (not pass count alone — this
/// project's own lexical suite is already 40/40-adjacent and can't show
/// movement on pass count alone), 2 reproduced runs per value on a verified
/// fully-settled index (`semantic_pending: 0`, not just `state: "fresh"` —
/// a real methodology gap found mid-sweep: `state` can flip to `fresh`
/// while the FTS backlog is still draining on a large corpus like keycloak,
/// which produced one spurious, non-reproducing outlier reading before this
/// was caught and fixed). Result was monotonically increasing on both
/// corpora, not a plateau centered on 0.5 as the old gated sweep's 0.5 pick
/// might have suggested:
///
/// | value | keycloak pass@5 | keycloak MRR | lexical pass@5 | lexical MRR |
/// |-------|------------------|---------------|-----------------|--------------|
/// | 0.0   | 18/40            | 7.65          | 33/40           | 15.94        |
/// | 0.25  | 22/40            | 8.90          | 36/40           | 25.40        |
/// | 0.5   | 22/40            | 10.54         | 36/40           | 28.03        |
/// | 0.75  | 24/40            | 14.28         | 36/40           | 29.60        |
/// | 1.0   | 24/40            | 14.82         | 36/40           | 29.60        |
///
/// 0.75 and 1.0 form a real plateau (nearly identical on both corpora, not
/// a knife-edge single point) and both beat 0.5 outright — at least as good
/// on pass count, strictly better summed MRR, on both corpora. Kept at the
/// smaller value in that plateau (0.75, not 1.0), the same precedent
/// `TEST_FILE_PENALTY` set when it landed inside a 0.15-0.25 plateau at
/// 0.15. Values beyond 1.0 were not tested (out of the sweep's scoped
/// range) — if `graph_score` ever gets renormalized or a future signal
/// changes this term's effective range, this is worth re-sweeping rather
/// than assumed to hold indefinitely. Plan 22 item 4 made this config-driven
/// (was a `pipeline.rs` constant) so a future re-sweep is a config edit, not
/// a rebuild — the default value itself is unchanged.
fn default_direct_hit_graph_credit_damping() -> f32 {
    0.75
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            candidate_top_k: 20,
            expand_hops:  1,
            final_top_n:  5,
            beta:         0.667,
            gamma:        0.333,
            rerank: RerankConfig::default(),
            jev:    JevConfig::default(),
            laya:   LayaConfig::default(),
            path_pin: PathPinConfig::default(),
            ppr: PprConfig::default(),
            direct_hit_graph_credit_damping: default_direct_hit_graph_credit_damping(),
        }
    }
}

impl RetrievalConfig {
    /// The judge the pipeline should use: Jev if enabled, else Laya if enabled.
    pub fn judge_provider(&self) -> Option<JudgeProvider> {
        if self.jev.enabled {
            Some(JudgeProvider::Jev)
        } else if self.laya.enabled {
            Some(JudgeProvider::Laya)
        } else {
            None
        }
    }

    /// Pool width for the active judge (`jev.top_n` when none is active — unused then).
    pub fn judge_top_n(&self) -> usize {
        match self.judge_provider() {
            Some(JudgeProvider::Laya) => self.laya.top_n,
            _ => self.jev.top_n,
        }
    }

    /// Blend weight for the active judge.
    pub fn judge_weight(&self) -> f32 {
        match self.judge_provider() {
            Some(JudgeProvider::Laya) => self.laya.weight,
            _ => self.jev.weight,
        }
    }
}

/// Plan 22 item 3 — file-path pinning. See
/// `plans/22-retrieval-hygiene-and-agent-signals.md` for the full design:
/// a query naming a path or bare filename (`accounts/…/billing_status.py`,
/// `LexicalEditor.ts`) gets that file's symbols guaranteed admission into the
/// candidate pool, since `tokenize_query_split` shredding a path into generic
/// stems never actually matches `chunk_fts.file_path` (untokenized — a
/// separate, later-measured change, arm C).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathPinMode {
    /// Detection doesn't even run — zero behavior/perf change vs. pre-item-3.
    /// No longer the default (see `PathPinConfig`'s doc comment) — kept as
    /// an explicit opt-out.
    Off,
    /// Pin, and remove the matched path mention from the text fed to Stage
    /// 1's `lexical_search` (arm A) — **the shipped default**, measured
    /// better than or equal to arm B on both corpora with file-naming
    /// tasks (lexical: no regression at all vs. a small one under Keep;
    /// erpnext: statistically equivalent).
    #[default]
    Strip,
    /// Pin, and leave the path text in the FTS query unchanged (arm B).
    Keep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathPinConfig {
    #[serde(default)]
    pub mode: PathPinMode,
    /// Flat additive bonus (outside the beta+gamma≈1.0 budget, same pattern
    /// as `RerankConfig::name_match_bonus`) for a pinned candidate — Plan 22:
    /// "guaranteed admission," not a hard override of ranking, since the
    /// named file isn't always where the answer lives. Provisional starting
    /// value, unswept.
    #[serde(default = "default_path_pin_bonus")]
    pub bonus: f32,
    /// Cap on how many symbols from one pinned file are admitted — a large
    /// file can hold hundreds of symbols, and each admitted candidate costs
    /// `enrich()` a callers/callees/snippet load. Provisional starting value.
    #[serde(default = "default_path_pin_max_symbols")]
    pub max_symbols_per_file: usize,
}

fn default_path_pin_bonus() -> f32 {
    0.15
}

fn default_path_pin_max_symbols() -> usize {
    20
}

impl Default for PathPinConfig {
    fn default() -> Self {
        Self {
            // `PathPinMode::default()`, not a hardcoded variant — a config
            // TOML section that specifies e.g. only `bonus` and omits `mode`
            // must fall back to the *same* default as an entirely absent
            // `[retrieval.path_pin]` table. Two different "off"/"on"
            // defaults for those two cases is exactly the silent-shadowing
            // class of bug this item's own measurement got bitten by.
            mode: PathPinMode::default(),
            bonus: default_path_pin_bonus(),
            max_symbols_per_file: default_path_pin_max_symbols(),
        }
    }
}

/// Plan 22 item 4: personalized PageRank sweep knobs. Defaults reproduce the
/// pre-item-4 hardcoded constants (`PPR_ITERATIONS = 4`, no early stop)
/// exactly — this struct's job is letting a sweep change these via config +
/// restart, not changing default behavior on its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PprConfig {
    /// A fixed, small iteration count is a bounded approximation for latency
    /// (ship gate: p95 ≤ 100ms — not yet met on real corpora regardless of
    /// this value, see `bench-reports/plan22-item3-CORRECTED-shipped-
    /// 2026-09-27.md`'s latency finding), not a converged-to-precision
    /// PageRank.
    #[serde(default = "default_ppr_iterations")]
    pub iterations: usize,
    /// Stop early once an iteration's L1 score-delta drops below this
    /// epsilon, capped at `iterations` either way. `None` (the default)
    /// disables early stopping — always runs exactly `iterations` rounds,
    /// matching pre-item-4 behavior.
    #[serde(default)]
    pub early_stop_epsilon: Option<f32>,
}

fn default_ppr_iterations() -> usize {
    4
}

impl Default for PprConfig {
    fn default() -> Self {
        Self {
            iterations: default_ppr_iterations(),
            early_stop_epsilon: None,
        }
    }
}

/// Plan 20 Phase 8: optional cloud-judge rerank signal. A per-candidate
/// relevance judgment (TypeSafe AI's Jev — see `plans/20-remove-vector-search.md`
/// Phase 8) layered onto the always-on lexical+graph pipeline's already-ranked
/// pool, targeting the class of miss that has no textual bridge for BM25/PPR
/// to find at all (`bench-reports/keycloak-cross-repo-generalization-2026-08-20.md`).
/// `weight` is an additive bonus outside the `beta+gamma≈1.0` budget — the
/// same pattern `RerankConfig::name_match_bonus` already uses — not a new
/// term in that invariant, since Jev is a categorically different kind of
/// signal (a judgment call, not a keyword/graph score) and off by default
/// means it costs nothing when disabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevConfig {
    /// Off by default — requires `api_key` to actually activate even when true.
    #[serde(default)]
    pub enabled: bool,
    /// Overridden by the `GRAPHRELAY_JEV_API_KEY` env var if set.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_jev_base_url")]
    pub base_url: String,
    #[serde(default = "default_jev_model")]
    pub model: String,
    /// Dual-purpose pool width, active only when Jev is: (1) how many
    /// candidates `lexical_search` recalls in the first place (widened
    /// beyond the caller's own `top_k`, which stays tight — 20 by default —
    /// for everyone not using Jev), and (2) how many of `rerank()`'s ranked
    /// candidates get kept for Jev to judge. Both need to move together —
    /// widening recall alone is pointless if `rerank()` just truncates the
    /// newly-recalled candidates back out before Jev ever sees them.
    ///
    /// **Default 50, validated, not a guess.** First attempt at this (50,
    /// with each candidate carrying its full untruncated snippet) measurably
    /// regressed keycloak (36/40 → 34/40, reproduced twice) — real-world
    /// guidance on Jev's own limits (32K-token context, pay-per-input-token,
    /// "keep chunks tight" when scoring many candidates at once) pointed at
    /// payload size as the likely cause, which `apply_jev`'s
    /// `JEV_CANDIDATE_SNIPPET_CHARS` truncation (added alongside this) was
    /// built to fix. With truncation in place, 50 was re-tested and measured
    /// a real improvement instead: keycloak **36/40 → 38/40**, and the
    /// lexical suite (already 40/40 without Jev) stayed at **40/40** with
    /// Jev on — both reproduced 2-3× identically. Full before/after account:
    /// `bench-reports/plan20-phase8-jev-keycloak-benchmark-2026-09-25.md`.
    #[serde(default = "default_jev_top_n")]
    pub top_n: usize,
    #[serde(default = "default_jev_weight")]
    pub weight: f32,
    #[serde(default = "default_jev_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for JevConfig {
    fn default() -> Self {
        Self {
            enabled:     false,
            api_key:     None,
            base_url:    default_jev_base_url(),
            model:       default_jev_model(),
            top_n:       default_jev_top_n(),
            weight:      default_jev_weight(),
            timeout_ms:  default_jev_timeout_ms(),
        }
    }
}

/// Optional relevance judge backed by Laya (`github.com/NandhaKishorM/laya`, Apache-2.0):
/// a non-autoregressive encoder that answers typed yes/no/choice/score questions in one
/// forward pass and speaks the same `/v1/systemone` wire protocol as Jev. Self-hosted
/// (`pip install "laya[serve]"`, then `laya-serve`), so no API key is needed unless the
/// server sets `LAYA_API_KEY`.
///
/// Unlike Jev, Laya reads a small window (512 tokens on the English checkpoint, 1024 on
/// the others, 8192 with `max_len`), so it cannot take the whole candidate pool as one
/// state: the adapter sends **one request per candidate**, each with the query and that
/// candidate's (already truncated) text, and runs `concurrency` of them at a time. That
/// makes `top_n` a latency knob — every extra candidate is one more forward pass — and is
/// why its default is smaller than Jev's. Independent of `[retrieval.jev]`: if both are
/// enabled Jev wins (a warning is logged at startup).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayaConfig {
    /// Off by default.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_laya_base_url")]
    pub base_url: String,
    /// Only needed when the server was started with `LAYA_API_KEY`. Overridden by the
    /// `GRAPHRELAY_LAYA_API_KEY` env var if set.
    #[serde(default)]
    pub api_key: Option<String>,
    /// `""` lets Laya's router choose a checkpoint; otherwise `english`, `multilingual`
    /// or `typed-decisions` (or their Hugging Face ids).
    #[serde(default)]
    pub model: String,
    /// Same dual purpose as `jev.top_n` (recall widening and how many ranked candidates
    /// are judged), but each candidate costs one Laya forward pass (~135 ms on an Apple GPU
    /// via MPS; expect several times that on CPU), so the default is 20.
    #[serde(default = "default_laya_top_n")]
    pub top_n: usize,
    /// Default 0.05, chosen by the Plan 25 decision rule, not by preference: no swept
    /// weight met "at least +0.01 normalized MRR on every corpus with none below -0.01".
    /// Measured end to end (English checkpoint, `top_n = 20`), normalized-MRR change vs
    /// no judge — lexical / keycloak / erpnext: 0.05 → +0.020 / 0.000 / −0.007;
    /// 0.30 → +0.047 / +0.002 / −0.001; 0.60 → +0.069 / +0.044 / −0.010; 1.0 →
    /// +0.078 / +0.119 / −0.016. Laya's P(true) is compressed (most lexically related
    /// candidates score 0.7-0.9), so it acts as a near-tie-breaker among the top few:
    /// raise the weight (0.6 is a good start) on a corpus whose base ranking is strong
    /// and tie-heavy, and validate on your own queries first. See
    /// `bench-reports/plan25-laya-adapter-2026-09-29.md`.
    #[serde(default = "default_laya_weight")]
    pub weight: f32,
    /// Deadline for judging the whole pool. On expiry the candidates scored so far are
    /// used and the rest are treated as "no judgment", never as irrelevant.
    #[serde(default = "default_laya_timeout_ms")]
    pub timeout_ms: u64,
    /// In-flight requests. `laya-serve` runs one forward pass at a time, so concurrency only
    /// hides HTTP round-trips: measured p50 per 20-candidate query 2.9 s at 1, 2.7 s at 4
    /// and at 16 (Apple GPU, English checkpoint).
    #[serde(default = "default_laya_concurrency")]
    pub concurrency: usize,
    /// Optional per-request `max_len` (token budget) sent to Laya; `None` keeps the
    /// checkpoint default.
    #[serde(default)]
    pub max_len: Option<u32>,
}

impl Default for LayaConfig {
    fn default() -> Self {
        Self {
            enabled:     false,
            base_url:    default_laya_base_url(),
            api_key:     None,
            model:       String::new(),
            top_n:       default_laya_top_n(),
            weight:      default_laya_weight(),
            timeout_ms:  default_laya_timeout_ms(),
            concurrency: default_laya_concurrency(),
            max_len:     None,
        }
    }
}

/// Which concrete relevance judge (if any) the pipeline should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeProvider {
    Jev,
    Laya,
}

impl JudgeProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            JudgeProvider::Jev  => "jev",
            JudgeProvider::Laya => "laya",
        }
    }
}

/// Improvement #3: BM25 reranking configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankConfig {
    /// Enable BM25 reranking of vector candidates.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// BM25 k1 parameter (term saturation, typical: 1.2–2.0).
    #[serde(default = "default_rerank_k1")]
    pub k1:      f32,
    /// BM25 b parameter (length normalization, typical: 0.75).
    #[serde(default = "default_rerank_b")]
    pub b:       f32,
    /// Bonus added per unit of fractional name-token match.
    /// Splits the symbol's camelCase/snake_case name into tokens and checks how many
    /// appear in the query. A perfect match adds this bonus; a 50% match adds half.
    /// Helps navigation tasks where the query contains identifier tokens ("resume router").
    ///
    /// Bumped 0.15→0.22 2026-08-18 (`bench-reports/lexical-readeditorstate-tiebreak-investigation-2026-08-17.md`):
    /// a saturated (1.0) name-token match wasn't enough to rescue a candidate
    /// with zero graph-score credit under a stubbed embedder (a caller-side-only
    /// graph neighbor gets nothing from `personalized_pagerank`'s callees_of-only
    /// walk — see its own doc comment). Verified across 3 repeated full-benchmark
    /// runs at 0.22 (40/40 every time, vs. 38-39/40 at 0.15) and confirmed stable
    /// at neighboring 0.20 too (not a knife-edge fit to one lucky run) — the
    /// validation this project's history requires before trusting a rerank-weight
    /// change (see the reverted "specificity re-scoring" and `graph_score`
    /// caller-credit attempts in the same investigation, both of which fixed
    /// their one targeted case while regressing several others).
    #[serde(default = "default_name_match_bonus")]
    pub name_match_bonus: f32,
}

fn default_name_match_bonus() -> f32 { 0.22 }

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            enabled:          true,
            k1:               1.5,
            b:                0.75,
            name_match_bonus: 0.22,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherConfig {
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default = "default_max_queue")]
    pub max_queue:   usize,
    #[serde(default = "default_ignore_watcher")]
    pub ignore:      Vec<String>,
    /// Run the heuristic dynamic-dispatch synthesis pass (currently: C/C++
    /// function-pointer dispatch tables) after `resolve_edges()` on every
    /// full index. Purely additive — produces extra `EdgeSource::Synthesized`
    /// edges on top of the existing 3-stage resolver, never removes any.
    #[serde(default = "default_true")]
    pub synthesis_enabled: bool,
    /// Explicit override for the full-index parallel parse pool's memory
    /// budget, in MiB. When unset, the pool size is derived from a live
    /// `sysinfo` read of available host memory (see
    /// `graphrelay_watcher::memory_budget`), which does not see a
    /// cgroup/container memory limit — set this explicitly when running in
    /// a memory-limited container so the pool is sized to the real
    /// constraint rather than the host's full memory.
    #[serde(default)]
    pub memory_budget_mb: Option<u64>,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 50,
            max_queue:   1000,
            ignore:      default_ignore_watcher(),
            synthesis_enabled: true,
            memory_budget_mb: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            tracing::info!(
                "Config file {:?} not found, using defaults",
                path
            );
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {:?}", path))?;
        let cfg: Self = toml::from_str(&content)
            .with_context(|| format!("parsing config {:?}", path))?;
        cfg.validate();
        Ok(cfg)
    }

    fn validate(&self) {
        // Retrieval weight sum — Plan 20 removed the vector leg (`alpha`),
        // so this is now just beta+gamma.
        let sum = self.retrieval.beta + self.retrieval.gamma;
        if (sum - 1.0_f32).abs() > 0.05 {
            tracing::warn!(
                "retrieval weights beta+gamma = {:.3} (expected ~1.0)",
                sum
            );
        }

        // Graph expansion is always one hop (`GraphDb::neighbors` never reads
        // `hops` beyond > 0), so this knob cannot do what its name says.
        if self.retrieval.expand_hops != default_expand_hops() {
            tracing::warn!(
                "retrieval.expand_hops = {} has no effect: graph expansion is always 1 hop",
                self.retrieval.expand_hops
            );
        }

        // Port range
        if self.server.port < 1024 {
            tracing::warn!(
                "server.port {} is below 1024 — this typically requires elevated privileges",
                self.server.port
            );
        }

        // Memory budget override
        if let Some(mb) = self.watcher.memory_budget_mb {
            if mb == 0 {
                tracing::warn!(
                    "watcher.memory_budget_mb is 0 — the full-index parse pool will be forced \
                     to run sequentially (pool size 1); set it unset or to a positive value \
                     unless that's intentional",
                );
            }
        }

        if self.retrieval.jev.enabled && self.retrieval.laya.enabled {
            tracing::warn!(
                "retrieval.jev.enabled and retrieval.laya.enabled are both true — only one \
                 relevance judge runs; Jev takes precedence, Laya is ignored",
            );
        }
        if self.retrieval.laya.enabled && self.retrieval.laya.concurrency == 0 {
            tracing::warn!("retrieval.laya.concurrency is 0 — it will be treated as 1");
        }

        // Jev (Plan 20 Phase 8) — enabled with no way to authenticate means every
        // call will fail at request time; warn at startup instead of only on the
        // first query.
        if self.retrieval.jev.enabled {
            let has_key = self.retrieval.jev.api_key.is_some()
                || std::env::var("GRAPHRELAY_JEV_API_KEY").is_ok();
            if !has_key {
                tracing::warn!(
                    "retrieval.jev.enabled is true but no api_key is set (config or \
                     GRAPHRELAY_JEV_API_KEY) — Jev reranking will be skipped, falling back \
                     to lexical+graph ranking only",
                );
            }
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        if let Some(ref d) = self.index.data_dir {
            return d.clone();
        }
        if let Ok(env_dir) = std::env::var("GRAPHRELAY_DATA_DIR") {
            return PathBuf::from(env_dir);
        }
        // Default: ~/.graphrelay/<project-hash>/
        let home = dirs_home();
        let root_str = self.index.root.to_string_lossy();
        let hash = format!("{:x}", simple_hash(root_str.as_bytes()));
        home.join(".graphrelay").join(&hash[..8])
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn simple_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
