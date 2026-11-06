//! REST handlers used to return HTTP 200
//! unconditionally, with only the (easy-to-miss) `meta.index_state` response
//! field hinting that the index was `Stale`. These tests assert the HTTP
//! status code itself now reflects that.

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use graphrelay_api::{api_router, ApiState};
use graphrelay_core::{
    retrieval::{QueryPipeline, RetrievalQuery, RetrievalResult, SourceLoader},
    types::{IndexState, IndexStatus},
};
use graphrelay_store::GraphDb;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::util::ServiceExt; // oneshot

struct StubPipeline;

#[async_trait]
impl QueryPipeline for StubPipeline {
    async fn query(&self, _req: RetrievalQuery) -> anyhow::Result<Vec<RetrievalResult>> {
        Ok(vec![])
    }
}

fn state_with(index_state: IndexState) -> ApiState {
    ApiState {
        graph_db:      Arc::new(GraphDb::open_in_memory().unwrap()),
        source_loader: Arc::new(SourceLoader::new()),
        watch_status:  Arc::new(RwLock::new(IndexStatus {
            state: index_state, last_indexed_at: None, pending_count: 0,
            total_symbols: 0, total_files: 0, last_error: None, semantic_pending: 0,
        })),
        pipeline: Arc::new(StubPipeline),
    }
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

#[tokio::test]
async fn search_returns_200_when_fresh() {
    let app = api_router(state_with(IndexState::Fresh));
    let (status, body) = get(app, "/api/search?q=test").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["meta"]["index_state"], "fresh");
}

#[tokio::test]
async fn search_returns_503_when_stale() {
    let app = api_router(state_with(IndexState::Stale));
    let (status, body) = get(app, "/api/search?q=test").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["meta"]["index_state"], "stale");
}

#[tokio::test]
async fn graph_returns_503_when_stale() {
    let app = api_router(state_with(IndexState::Stale));
    let (status, body) = get(app, "/api/graph").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["meta"]["index_state"], "stale");
}

#[tokio::test]
async fn languages_returns_503_when_stale() {
    let app = api_router(state_with(IndexState::Stale));
    let (status, body) = get(app, "/api/languages").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["meta"]["index_state"], "stale");
}

#[tokio::test]
async fn languages_returns_200_when_indexing() {
    // Deliberate: `Indexing` stays 200 for these general handlers (unlike
    // `search_handler`'s own pre-existing hard-error for Indexing/Error) —
    // see `respond()`'s doc comment for why this is scoped to `Stale` only.
    let app = api_router(state_with(IndexState::Indexing));
    let (status, body) = get(app, "/api/languages").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["meta"]["index_state"], "indexing");
}

#[tokio::test]
async fn graph_returns_503_when_error() {
    let app = api_router(state_with(IndexState::Error));
    let (status, body) = get(app, "/api/graph").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["meta"]["index_state"], "error");
}

#[tokio::test]
async fn status_endpoint_always_returns_200_even_when_stale() {
    // /api/status is how a client is meant to *discover* staleness in the
    // first place — it must stay 200 regardless of index_state, or checking
    // becomes self-defeating.
    let app = api_router(state_with(IndexState::Stale));
    let (status, body) = get(app, "/api/status").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "stale");
}
