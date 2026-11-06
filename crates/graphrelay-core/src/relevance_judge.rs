use crate::types::SymbolId;
use async_trait::async_trait;
use std::collections::HashMap;

/// Plan 20 Phase 8: optional cloud-judge rerank signal. Implement this for any
/// judgment provider (TypeSafe AI's Jev, or a future alternative) — the
/// pipeline uses `Arc<dyn RelevanceJudge>` so adding a provider requires no
/// changes to `graphrelay-core`'s retrieval logic, the same boundary
/// discipline `EmbeddingAdapter` used before Plan 20 removed it.
///
/// Deliberately not named after Jev specifically — this trait describes what
/// the pipeline needs (a relevance judgment per candidate), not TypeSafe's
/// API shape. The one concrete implementation, `JevJudge`, lives outside
/// `graphrelay-core` (in a separate crate), which stays free of HTTP/reqwest
/// dependencies.
#[async_trait]
pub trait RelevanceJudge: Send + Sync {
    /// Judge how relevant each candidate is to `query`, in one batched call
    /// where possible — providers charging per-request benefit from judging
    /// the whole pool at once rather than one call per candidate.
    ///
    /// `candidates` is `(SymbolId, text)` pairs — `text` is whatever context
    /// (name + snippet, typically) the judge should reason about.
    ///
    /// Returns a map of `SymbolId` → relevance probability in `[0, 1]`.
    /// **Partial results are expected and must be handled gracefully by
    /// callers**: an id missing from the returned map means "no judgment,"
    /// not "zero relevance" — a provider that judged 18 of 20 candidates
    /// before hitting a timeout should still return those 18, not fail the
    /// whole call. Callers must not treat a missing id as a negative signal.
    ///
    /// **Implementations own their own timeout.** `graphrelay-core` stays free
    /// of `tokio`/I/O (see `CLAUDE.md`'s coding conventions), so the pipeline
    /// cannot wrap this call in `tokio::time::timeout` itself — an
    /// implementation must enforce `RetrievalConfig::jev.timeout_ms` (or its
    /// own equivalent) internally, e.g. via `reqwest::Client`'s own
    /// `.timeout(...)`, and return `Err` on expiry rather than hanging.
    async fn judge_relevance(
        &self,
        query: &str,
        candidates: &[(SymbolId, String)],
    ) -> anyhow::Result<HashMap<SymbolId, f32>>;
}
