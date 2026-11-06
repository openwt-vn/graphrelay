//! Plan 20 Phase 8: concrete `RelevanceJudge` implementations (the optional
//! cloud-judge rerank signal — see `plans/20-remove-vector-search.md` Phase
//! 8 and `graphrelay_core::relevance_judge`). Named `graphrelay-decision-engine`,
//! not `graphrelay-jev`, deliberately: `RelevanceJudge` is provider-agnostic
//! by design, and this crate is meant to be the one place that holds every
//! concrete provider adapter as a sibling module, the same way the old
//! `graphrelay-embed` crate held both `BgeSmallModel` (local ONNX) and
//! `VoyageModel` (cloud) side by side under one `EmbeddingAdapter` trait
//! before Plan 20 removed embeddings entirely. Jev is the first adapter, not
//! the only one this crate is meant to ever hold — adding a provider means a new
//! sibling module here (`src/some_other_provider.rs`) and an impl of the same
//! trait, not a new crate and a new workspace member. `laya` (Plan 25) is the
//! second: a self-hosted encoder speaking Jev's wire protocol but scored one
//! candidate per request, see its module doc.
//!
//! Kept out of `graphrelay-core` (which must stay dependency-free — see
//! `CLAUDE.md`'s coding conventions) and out of `graphrelay-store` (which
//! owns SQLite and has nothing to do with an HTTP judging API) — this crate
//! is purely a boundary adapter, same discipline `graphrelay-embed` used.

pub mod jev;
pub mod laya;

pub use jev::JevJudge;
pub use laya::LayaJudge;
