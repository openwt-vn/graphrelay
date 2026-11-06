//! `LayaJudge`: the Laya adapter (`https://github.com/NandhaKishorM/laya`, Apache-2.0),
//! the second `RelevanceJudge` in this crate next to `JevJudge`.
//!
//! Wire protocol: Laya's `laya-serve` exposes Jev's `POST /v1/systemone` protocol
//! (`{state, questions, model?}` → `{answers: {<id>: {type: "noul", noul: <P(true)>}}}`),
//! verified against `docs/http-api.md` and `laya/serve.py` of the repository (fetched
//! directly, not guessed). It also accepts `max_len` / `head_max_len` token-budget
//! overrides, and an optional `Authorization: Bearer` check when the server sets
//! `LAYA_API_KEY`.
//!
//! **Why this is not `JevJudge` with a different URL.** Jev takes the whole candidate pool
//! as one structured state and one yes/no question per candidate id. Laya is a 421M-parameter
//! encoder with a small window (512 tokens on the English checkpoint, 1024 on the others), so
//! 20-50 candidates cannot share a state. The adapter therefore sends **one request per
//! candidate** — state = the query plus that candidate's text, one `noul` question — with a
//! bounded number in flight, and treats each answer's `noul` (a calibrated P(true)) as the
//! relevance probability. Consequences that shape the defaults: every candidate costs one
//! forward pass (so `laya.top_n` defaults to 20, not 50), a slow or failed candidate is
//! simply left unscored (the trait's "missing means no judgment" rule), and the whole call
//! has its own deadline that returns whatever finished.
//!
//! **`noul` needs explicit criteria.** Laya's README documents that a `noul` without
//! criteria answers "no" whatever the state on the English checkpoint, and that the option
//! labels can dominate the answer (issue #156). The question therefore always carries
//! criteria describing the property of the *state*; validate scores on your own corpus
//! (`bench-reports/plan25-laya-adapter-*.md` has this project's measurement).

use async_trait::async_trait;
use graphrelay_core::relevance_judge::RelevanceJudge;
use graphrelay_core::types::SymbolId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8000/v1/systemone";
/// The id of the single question sent per candidate.
const QUESTION_ID: &str = "relevant";
/// A server at `LAYA_MAX_CONCURRENT` refuses (503) instead of queueing; retry briefly.
const BUSY_RETRIES: u32 = 3;
/// The query is truncated (in chars) so it can never crowd the candidate out of the
/// checkpoint's window. Benchmark questions run ~300-500 chars.
const MAX_QUERY_CHARS: usize = 600;

/// Instructions are deliberately short: the question's instructions and its two criteria
/// share the checkpoint's option budget (`head_max_len`, 192-256 tokens); the query and
/// candidate go in the state, which has the rest of the window.
const INSTRUCTIONS: &str = "Is the candidate code relevant to the search query?";
const CRITERIA_TRUE: &str = "The candidate code is relevant to the search query: the answer to \
     the query plausibly lives in or near this code.";
const CRITERIA_FALSE: &str = "The candidate code is not relevant to the search query.";

pub struct LayaJudge {
    client:      reqwest::Client,
    api_key:     Option<String>,
    base_url:    String,
    model:       Option<String>,
    max_len:     Option<u32>,
    concurrency: usize,
    deadline:    Duration,
}

impl LayaJudge {
    /// `timeout` is both each request's timeout and the deadline for the whole call
    /// (`RelevanceJudge` implementations own their timeouts — `graphrelay-core` has no
    /// runtime to enforce one).
    pub fn new(
        api_key: Option<String>,
        base_url: String,
        model: String,
        max_len: Option<u32>,
        concurrency: usize,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build Laya HTTP client: {e}"))?;
        Ok(Self {
            client,
            api_key: api_key.filter(|k| !k.is_empty()),
            base_url: if base_url.is_empty() { DEFAULT_BASE_URL.to_string() } else { base_url },
            model: Some(model).filter(|m| !m.is_empty()),
            max_len,
            concurrency: concurrency.max(1),
            deadline: timeout,
        })
    }
}

// ── Wire format (docs/http-api.md, laya/serve.py) ───────────────────────────

#[derive(Serialize)]
struct SystemOneRequest<'a> {
    state:     String,
    questions: HashMap<&'static str, Question>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model:     Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_len:   Option<u32>,
}

#[derive(Serialize)]
struct Question {
    #[serde(rename = "type")]
    kind:         &'static str,
    instructions: &'static str,
    criteria:     Criteria,
}

#[derive(Serialize)]
struct Criteria {
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
    /// P(true) on a `noul` answer. `Option` so an unexpectedly-shaped answer degrades
    /// to "no judgment" for that candidate instead of failing the whole response.
    noul: Option<f32>,
}

fn build_request<'a>(
    query: &str,
    candidate_text: &str,
    model: Option<&'a str>,
    max_len: Option<u32>,
) -> SystemOneRequest<'a> {
    let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
    let mut questions = HashMap::with_capacity(1);
    questions.insert(
        QUESTION_ID,
        Question {
            kind: "noul",
            instructions: INSTRUCTIONS,
            criteria: Criteria { yes: CRITERIA_TRUE, no: CRITERIA_FALSE },
        },
    );
    SystemOneRequest {
        state: format!("Search query: {query}\n\nCandidate code:\n{candidate_text}"),
        questions,
        model,
        max_len,
    }
}

#[async_trait]
impl RelevanceJudge for LayaJudge {
    async fn judge_relevance(
        &self,
        query: &str,
        candidates: &[(SymbolId, String)],
    ) -> anyhow::Result<HashMap<SymbolId, f32>> {
        if candidates.is_empty() {
            return Ok(HashMap::new());
        }

        let deadline = Instant::now() + self.deadline;
        let permits = Arc::new(Semaphore::new(self.concurrency));
        let mut tasks: JoinSet<(SymbolId, anyhow::Result<f32>)> = JoinSet::new();
        for (id, text) in candidates {
            let (id, text, query) = (*id, text.clone(), query.to_string());
            let (client, url, key) = (self.client.clone(), self.base_url.clone(), self.api_key.clone());
            let (model, max_len, permits) = (self.model.clone(), self.max_len, permits.clone());
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let body = build_request(&query, &text, model.as_deref(), max_len);
                (id, score_one(&client, &url, key.as_deref(), &body).await)
            });
        }

        let mut out = HashMap::with_capacity(candidates.len());
        let mut failed = 0usize;
        let mut first_error: Option<anyhow::Error> = None;
        let mut timed_out = false;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, tasks.join_next()).await {
                Ok(Some(Ok((id, Ok(score))))) => {
                    out.insert(id, score.clamp(0.0, 1.0));
                }
                Ok(Some(Ok((_, Err(e))))) => {
                    failed += 1;
                    first_error.get_or_insert(e);
                }
                Ok(Some(Err(join_error))) => {
                    failed += 1;
                    first_error.get_or_insert(anyhow::anyhow!("Laya task failed: {join_error}"));
                }
                Ok(None) => break,
                Err(_elapsed) => {
                    timed_out = true;
                    tasks.abort_all();
                    break;
                }
            }
        }

        if out.is_empty() {
            return Err(first_error.unwrap_or_else(|| {
                anyhow::anyhow!("Laya judged 0 of {} candidates before the deadline", candidates.len())
            }));
        }
        if failed > 0 || timed_out {
            tracing::warn!(
                "Laya judged {}/{} candidates ({failed} failed, deadline hit: {timed_out}) — the rest are unscored",
                out.len(),
                candidates.len(),
            );
        }
        Ok(out)
    }
}

/// One candidate, with a short retry when the server is at capacity (it refuses with 503
/// rather than queueing).
async fn score_one(
    client: &reqwest::Client,
    url: &str,
    api_key: Option<&str>,
    body: &SystemOneRequest<'_>,
) -> anyhow::Result<f32> {
    let mut delay = Duration::from_millis(150);
    for attempt in 0..=BUSY_RETRIES {
        let mut req = client.post(url).json(body);
        if let Some(key) = api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(|e| anyhow::anyhow!("Laya request failed: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE && attempt < BUSY_RETRIES {
            tokio::time::sleep(delay).await;
            delay *= 2;
            continue;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!(
                "Laya rejected the bearer token (401) — set api_key in [retrieval.laya] or \
                 GRAPHRELAY_LAYA_API_KEY to match the server's LAYA_API_KEY"
            );
        }
        let parsed: SystemOneResponse = resp
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("Laya API returned an error: {e}"))?
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("failed to parse Laya response: {e}"))?;
        return parsed
            .answers
            .get(QUESTION_ID)
            .and_then(|a| a.noul)
            .ok_or_else(|| anyhow::anyhow!("Laya response has no `noul` answer for `{QUESTION_ID}`"));
    }
    unreachable!("the loop returns or continues on every path")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, http::StatusCode, routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Pins the exact JSON sent, field for field against `docs/http-api.md`.
    #[test]
    fn request_serializes_to_the_documented_wire_shape() {
        let req = build_request("find reset", "a.rs::resetEditor\nfn resetEditor() {}", Some("multilingual"), Some(1024));
        let v: Value = serde_json::to_value(&req).unwrap();
        assert_eq!(v["model"], "multilingual");
        assert_eq!(v["max_len"], 1024);
        let state = v["state"].as_str().unwrap();
        assert!(state.contains("Search query: find reset"));
        assert!(state.contains("fn resetEditor() {}"));
        assert_eq!(v["questions"]["relevant"]["type"], "noul");
        assert!(!v["questions"]["relevant"]["instructions"].as_str().unwrap().is_empty());
        assert!(v["questions"]["relevant"]["criteria"]["true"].as_str().unwrap().contains("relevant"));
        assert!(v["questions"]["relevant"]["criteria"]["false"].as_str().unwrap().contains("not relevant"));
    }

    #[test]
    fn optional_fields_are_omitted_when_unset() {
        let v: Value = serde_json::to_value(build_request("q", "c", None, None)).unwrap();
        assert!(v.get("model").is_none(), "an empty model lets Laya's router choose");
        assert!(v.get("max_len").is_none());
    }

    #[test]
    fn a_very_long_query_is_truncated_in_chars_not_bytes() {
        let q = "é".repeat(5000);
        let v: Value = serde_json::to_value(build_request(&q, "c", None, None)).unwrap();
        let state = v["state"].as_str().unwrap();
        assert!(state.chars().filter(|c| *c == 'é').count() == MAX_QUERY_CHARS);
    }

    #[test]
    fn response_deserializes_the_documented_sample() {
        let sample = r#"{"model":"laya-rl-agent","answers":{"relevant":{"type":"noul","noul":0.91,
            "confidence":0.91,"answer_confidence":0.91,"action":{"act_probability":1.0}}},
            "usage":{"input_tokens":74,"output_tokens":0},"routing":{"model":"english"}}"#;
        let r: SystemOneResponse = serde_json::from_str(sample).unwrap();
        assert_eq!(r.answers["relevant"].noul, Some(0.91));
    }

    // ── an in-process stand-in for `laya-serve` ─────────────────────────────

    #[derive(Clone)]
    struct Mock {
        in_flight:     Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
        calls:         Arc<AtomicUsize>,
        seen_auth:     Arc<std::sync::Mutex<Vec<Option<String>>>>,
        busy_first_n:  Arc<AtomicUsize>,
        delay_ms:      u64,
    }

    impl Mock {
        fn new(delay_ms: u64) -> Self {
            Self {
                in_flight: Arc::default(),
                max_in_flight: Arc::default(),
                calls: Arc::default(),
                seen_auth: Arc::default(),
                busy_first_n: Arc::default(),
                delay_ms,
            }
        }
    }

    /// Scores by marker in the state, so a test can tell that each answer was mapped back
    /// to the candidate it was asked about: "HIGH" → 0.9, "BAD" → 422, "SLOW" → sleeps.
    async fn handler(
        State(m): State<Mock>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Result<Json<Value>, StatusCode> {
        m.calls.fetch_add(1, Ordering::SeqCst);
        m.seen_auth.lock().unwrap().push(
            headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string),
        );
        if m.busy_first_n.load(Ordering::SeqCst) > 0 {
            m.busy_first_n.fetch_sub(1, Ordering::SeqCst);
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let now = m.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        m.max_in_flight.fetch_max(now, Ordering::SeqCst);
        let state = body["state"].as_str().unwrap_or_default().to_string();
        let delay = if state.contains("SLOW") { 5_000 } else { m.delay_ms };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        m.in_flight.fetch_sub(1, Ordering::SeqCst);
        if state.contains("BAD") {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let p = if state.contains("HIGH") { 0.9 } else { 0.1 };
        Ok(Json(json!({"answers": {"relevant": {"type": "noul", "noul": p}}, "usage": {}})))
    }

    async fn serve(m: Mock) -> String {
        let app = Router::new().route("/v1/systemone", post(handler)).with_state(m);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1/systemone")
    }

    fn judge(url: String, key: Option<&str>, concurrency: usize, timeout_ms: u64) -> LayaJudge {
        LayaJudge::new(
            key.map(str::to_string),
            url,
            String::new(),
            None,
            concurrency,
            Duration::from_millis(timeout_ms),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn each_answer_is_mapped_back_to_its_own_candidate() {
        let url = serve(Mock::new(5)).await;
        let cands: Vec<(SymbolId, String)> = (1..=6u64)
            .map(|i| (i, if i % 2 == 0 { format!("HIGH {i}") } else { format!("low {i}") }))
            .collect();
        let out = judge(url, None, 3, 5_000).judge_relevance("q", &cands).await.unwrap();
        assert_eq!(out.len(), 6);
        for i in 1..=6u64 {
            let want = if i % 2 == 0 { 0.9 } else { 0.1 };
            assert!((out[&i] - want).abs() < 1e-6, "candidate {i} got {}", out[&i]);
        }
    }

    #[tokio::test]
    async fn concurrency_is_bounded_and_actually_used() {
        let m = Mock::new(60);
        let url = serve(m.clone()).await;
        let cands: Vec<(SymbolId, String)> = (1..=12u64).map(|i| (i, format!("c{i}"))).collect();
        judge(url, None, 4, 10_000).judge_relevance("q", &cands).await.unwrap();
        let peak = m.max_in_flight.load(Ordering::SeqCst);
        assert!(peak <= 4, "peak in flight {peak} exceeds concurrency 4");
        assert!(peak >= 2, "requests were not overlapped (peak {peak})");
        assert_eq!(m.calls.load(Ordering::SeqCst), 12);
    }

    #[tokio::test]
    async fn a_failed_candidate_is_omitted_not_an_error_and_not_zero() {
        let url = serve(Mock::new(1)).await;
        let cands = vec![(1u64, "HIGH ok".to_string()), (2u64, "BAD input".to_string()), (3u64, "fine".to_string())];
        let out = judge(url, None, 2, 5_000).judge_relevance("q", &cands).await.unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains_key(&1) && out.contains_key(&3));
        assert!(!out.contains_key(&2), "a rejected candidate must have no score, not 0.0");
    }

    #[tokio::test]
    async fn every_candidate_failing_is_an_error() {
        let url = serve(Mock::new(1)).await;
        let cands = vec![(1u64, "BAD one".to_string()), (2u64, "BAD two".to_string())];
        assert!(judge(url, None, 2, 5_000).judge_relevance("q", &cands).await.is_err());
    }

    #[tokio::test]
    async fn the_deadline_returns_what_finished() {
        let url = serve(Mock::new(1)).await;
        let cands = vec![(1u64, "HIGH fast".to_string()), (2u64, "SLOW never".to_string())];
        let started = Instant::now();
        let out = judge(url, None, 2, 700).judge_relevance("q", &cands).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(3), "must not wait for the slow candidate");
        assert_eq!(out.len(), 1);
        assert!(out.contains_key(&1));
    }

    #[tokio::test]
    async fn the_bearer_token_is_sent_only_when_configured() {
        let m = Mock::new(1);
        let url = serve(m.clone()).await;
        let cands = vec![(1u64, "c".to_string())];
        judge(url.clone(), Some("s3cret"), 1, 5_000).judge_relevance("q", &cands).await.unwrap();
        judge(url, None, 1, 5_000).judge_relevance("q", &cands).await.unwrap();
        let seen = m.seen_auth.lock().unwrap().clone();
        assert_eq!(seen[0].as_deref(), Some("Bearer s3cret"));
        assert_eq!(seen[1], None, "no Authorization header when no key is set");
    }

    #[tokio::test]
    async fn a_busy_server_is_retried() {
        let m = Mock::new(1);
        m.busy_first_n.store(2, Ordering::SeqCst);
        let url = serve(m.clone()).await;
        let out = judge(url, None, 1, 10_000).judge_relevance("q", &[(1u64, "HIGH".to_string())]).await.unwrap();
        assert!(out.contains_key(&1));
        assert_eq!(m.calls.load(Ordering::SeqCst), 3, "two 503s then success");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_an_error_not_a_hang() {
        let cands = vec![(1u64, "c".to_string())];
        let started = Instant::now();
        let r = judge("http://127.0.0.1:9/v1/systemone".to_string(), None, 1, 2_000)
            .judge_relevance("q", &cands)
            .await;
        assert!(r.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn no_candidates_makes_no_request() {
        let m = Mock::new(1);
        let url = serve(m.clone()).await;
        assert!(judge(url, None, 1, 1_000).judge_relevance("q", &[]).await.unwrap().is_empty());
        assert_eq!(m.calls.load(Ordering::SeqCst), 0);
    }
}
