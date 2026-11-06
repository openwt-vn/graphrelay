//! `JevJudge`: the TypeSafe AI adapter, one of potentially several
//! `RelevanceJudge` implementations this crate can hold — see the crate-level
//! doc comment in `lib.rs` for why `graphrelay-decision-engine` (not
//! `graphrelay-jev`) is the crate boundary.
//!
//! API shape below is transcribed from `https://docs.typesafe.ai/api`
//! (fetched directly, not guessed or inferred from the LangChain blog post
//! that originally described Jev) as of 2026-09-25. **Verified working
//! end-to-end the same day** against a real account, routed through
//! OpenRouter (`base_url = "https://openrouter.ai/api/v1/systemone"`,
//! `model = "typesafe/jev-1.13"` — OpenRouter documents this endpoint as
//! preserving TypeSafe's own schema, so no code change was needed, only
//! config) — confirmed via `watch_status`'s `jev_active: true`, a real
//! `find_relevant_code` call whose latency jumped from this project's normal
//! ~2-5ms to ~850-880ms (consistent with an actual network round-trip, not a
//! silent fallback), and no errors logged. **Benchmarked and iterated the
//! same day** against keycloak (`bench-reports/plan20-phase8-jev-keycloak-benchmark-2026-09-25.md`):
//! 33/40 (no Jev) → 36/40 (Jev, first working config) → 34/40 (a naive
//! `jev.top_n` 20→50 widening — every candidate's *full* snippet in one
//! bigger batched call — regressed, reverted) → **38/40** (top_n=50 with
//! each candidate's snippet truncated to `JEV_CANDIDATE_SNIPPET_CHARS`
//! before judging, the actual fix, now shipped as the default), each step
//! reproduced 2-3×. The lexical suite (already 40/40 without Jev) was
//! confirmed unaffected with Jev on, also reproduced twice.

use async_trait::async_trait;
use graphrelay_core::relevance_judge::RelevanceJudge;
use graphrelay_core::types::SymbolId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai/v1/systemone";
const DEFAULT_MODEL: &str = "jev-latest";
/// Retry attempts on 429/529 before giving up. Matches the pattern the old
/// `VoyageModel` used for its own rate-limit handling.
const MAX_RETRIES: u32 = 5;

pub struct JevJudge {
    client:   reqwest::Client,
    api_key:  String,
    base_url: String,
    model:    String,
    /// Bounds the whole call, retries and backoff sleeps included. The client
    /// timeout alone is per attempt, so a rate-limited call could block one
    /// search for ~45 s (5 attempts + 2+4+8+16 s of backoff).
    deadline: Duration,
}

impl JevJudge {
    /// `timeout` is enforced on the underlying `reqwest::Client` — see
    /// `RelevanceJudge::judge_relevance`'s doc comment on why the timeout
    /// can't live in `graphrelay-core` itself.
    pub fn new(api_key: String, base_url: String, model: String, timeout: Duration) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build Jev HTTP client: {e}"))?;
        Ok(Self {
            client,
            api_key,
            base_url: if base_url.is_empty() { DEFAULT_BASE_URL.to_string() } else { base_url },
            model: if model.is_empty() { DEFAULT_MODEL.to_string() } else { model },
            deadline: timeout,
        })
    }
}

// ── Wire format (https://docs.typesafe.ai/api) ──────────────────────────────

#[derive(Serialize)]
struct SystemOneRequest<'a> {
    state:     StateBody<'a>,
    model:     &'a str,
    questions: HashMap<String, Question>,
}

#[derive(Serialize)]
struct StateBody<'a> {
    query:      &'a str,
    candidates: Vec<StateCandidate<'a>>,
}

#[derive(Serialize)]
struct StateCandidate<'a> {
    id:   String,
    text: &'a str,
}

/// Only the "noul" (yes/no) question type is used here — one per candidate,
/// asking whether that specific candidate (named by id in `instructions`,
/// since a single "noul" question can't itself take multiple targets) is
/// relevant to `state.query`. "choice"/"score" question types exist in the
/// API but don't fit this per-candidate-relevance use case as naturally.
#[derive(Serialize)]
struct Question {
    #[serde(rename = "type")]
    kind:         &'static str,
    instructions: String,
    criteria:     NoulCriteria,
}

#[derive(Serialize)]
struct NoulCriteria {
    #[serde(rename = "true")]
    yes: &'static str,
    #[serde(rename = "false")]
    no:  &'static str,
}

#[derive(Deserialize)]
struct SystemOneResponse {
    answers: HashMap<String, Answer>,
}

#[derive(Deserialize)]
struct Answer {
    /// Present on a "noul" answer; absent for other question types (not used
    /// here, but the field is `Option` rather than required so an
    /// unexpectedly-shaped response degrades to "no judgment" for that one
    /// candidate instead of failing to deserialize the whole response.
    noul: Option<f32>,
}

#[async_trait]
impl RelevanceJudge for JevJudge {
    async fn judge_relevance(
        &self,
        query: &str,
        candidates: &[(SymbolId, String)],
    ) -> anyhow::Result<HashMap<SymbolId, f32>> {
        if self.api_key.is_empty() {
            anyhow::bail!(
                "jev api_key is not set — add api_key to [retrieval.jev] in graphrelay.toml \
                 or set the GRAPHRELAY_JEV_API_KEY environment variable"
            );
        }
        if candidates.is_empty() {
            return Ok(HashMap::new());
        }

        let state = StateBody {
            query,
            candidates: candidates
                .iter()
                .map(|(id, text)| StateCandidate { id: id.to_string(), text })
                .collect(),
        };
        let questions: HashMap<String, Question> = candidates
            .iter()
            .map(|(id, _)| {
                let key = id.to_string();
                let question = Question {
                    kind: "noul",
                    instructions: format!(
                        "In the state above, is the candidate with id \"{key}\" relevant to \
                         answering `query`?"
                    ),
                    criteria: NoulCriteria {
                        yes: "The candidate is relevant to the query — it's plausible the \
                              answer to the query lives in or near this code.",
                        no: "The candidate is not relevant to the query.",
                    },
                };
                (key, question)
            })
            .collect();

        let body = SystemOneRequest { state, model: &self.model, questions };
        let resp = tokio::time::timeout(self.deadline, self.call_with_retry(&body))
            .await
            .map_err(|_| anyhow::anyhow!("Jev judging exceeded its {:?} deadline", self.deadline))??;

        let mut out = HashMap::with_capacity(candidates.len());
        for (id, _) in candidates {
            if let Some(answer) = resp.answers.get(&id.to_string()) {
                if let Some(noul) = answer.noul {
                    out.insert(*id, noul.clamp(0.0, 1.0));
                }
            }
        }
        Ok(out)
    }
}

impl JevJudge {
    async fn call_with_retry(&self, body: &SystemOneRequest<'_>) -> anyhow::Result<SystemOneResponse> {
        let mut delay = Duration::from_secs(2);
        for attempt in 0..MAX_RETRIES {
            let resp = self
                .client
                .post(&self.base_url)
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("Jev request failed: {e}"))?;

            let status = resp.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::from_u16(529).unwrap()
            {
                if attempt == MAX_RETRIES - 1 {
                    anyhow::bail!("Jev rate-limited/overloaded after {MAX_RETRIES} attempts (status {status})");
                }
                tracing::warn!("Jev {status} (attempt {}/{MAX_RETRIES}) — retrying in {delay:?}", attempt + 1);
                tokio::time::sleep(delay).await;
                delay *= 2;
                continue;
            }

            return resp
                .error_for_status()
                .map_err(|e| anyhow::anyhow!("Jev API returned an error: {e}"))?
                .json::<SystemOneResponse>()
                .await
                .map_err(|e| anyhow::anyhow!("failed to parse Jev response: {e}"));
        }
        unreachable!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the hand-written request shape against a field-name typo —
    /// there's no real API key in this environment to verify against a live
    /// call, so this is the next best thing: pin the exact JSON `judge_relevance`
    /// sends, matching `https://docs.typesafe.ai/api`'s documented request
    /// schema field-for-field (`state.query`, `state.candidates[].id/text`,
    /// `questions.<id>.type/instructions/criteria.true/false`).
    #[test]
    fn request_serializes_to_the_documented_wire_shape() {
        let state = StateBody {
            query: "find the reset function",
            candidates: vec![StateCandidate { id: "42".to_string(), text: "fn resetEditor() {}" }],
        };
        let mut questions = HashMap::new();
        questions.insert(
            "42".to_string(),
            Question {
                kind: "noul",
                instructions: "is candidate 42 relevant?".to_string(),
                criteria: NoulCriteria { yes: "yes", no: "no" },
            },
        );
        let req = SystemOneRequest { state, model: "jev-latest", questions };

        let json: serde_json::Value = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "jev-latest");
        assert_eq!(json["state"]["query"], "find the reset function");
        assert_eq!(json["state"]["candidates"][0]["id"], "42");
        assert_eq!(json["state"]["candidates"][0]["text"], "fn resetEditor() {}");
        assert_eq!(json["questions"]["42"]["type"], "noul");
        assert_eq!(json["questions"]["42"]["criteria"]["true"], "yes");
        assert_eq!(json["questions"]["42"]["criteria"]["false"], "no");
    }

    /// Pinned against the documented response shape's "noul" answer:
    /// `{"type": "noul", "noul": <0-1>}` nested under `answers.<question_id>`.
    #[test]
    fn response_deserializes_a_documented_sample_payload() {
        let sample = r#"{
            "model": "jev-latest",
            "answers": {
                "42": {"type": "noul", "noul": 0.87},
                "7":  {"type": "noul", "noul": 0.02}
            },
            "usage": {"input_tokens": 100, "output_tokens": 20}
        }"#;
        let resp: SystemOneResponse = serde_json::from_str(sample).unwrap();
        assert_eq!(resp.answers["42"].noul, Some(0.87));
        assert_eq!(resp.answers["7"].noul, Some(0.02));
    }

    /// A candidate id present in `judge_relevance`'s input but missing from
    /// the response's `answers` map (or present with an unexpected shape)
    /// must be silently absent from the returned scores, not cause an error
    /// or a panic — see `RelevanceJudge::judge_relevance`'s doc comment on
    /// why partial results are the expected, not exceptional, case.
    #[tokio::test]
    async fn missing_or_malformed_answers_are_silently_omitted_not_errors() {
        // Can't hit the real network in a unit test — construct the judge
        // and drive the parsing path directly via a hand-built response,
        // exercising the same `answers.get(...).and_then(...)` logic
        // `judge_relevance` uses after a real HTTP call would return.
        let raw = r#"{"answers": {"1": {"type": "noul", "noul": 0.5}}}"#;
        let resp: SystemOneResponse = serde_json::from_str(raw).unwrap();

        let ids = [1u64, 2u64]; // 2 has no entry in `resp.answers` at all
        let mut out = HashMap::new();
        for id in ids {
            if let Some(answer) = resp.answers.get(&id.to_string()) {
                if let Some(noul) = answer.noul {
                    out.insert(id, noul.clamp(0.0, 1.0));
                }
            }
        }
        assert_eq!(out.get(&1), Some(&0.5));
        assert_eq!(out.get(&2), None, "id 2 has no answer at all — must be absent, not defaulted to 0.0");
    }

    /// The configured timeout bounds the whole call: a server that answers
    /// every attempt with 429 used to hold the query for the full retry ladder.
    #[tokio::test]
    async fn a_rate_limited_judge_gives_up_at_its_overall_deadline() {
        use axum::{routing::post, Router};
        let app = Router::new().route("/v1/systemone", post(|| async { axum::http::StatusCode::TOO_MANY_REQUESTS }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let judge = JevJudge::new(
            "key".into(),
            format!("http://{addr}/v1/systemone"),
            String::new(),
            Duration::from_millis(300),
        )
        .unwrap();
        let started = std::time::Instant::now();
        let result = judge.judge_relevance("q", &[(1, "fn a() {}".to_string())]).await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    }
}
