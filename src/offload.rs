use axum::{extract::Request, middleware::Next, response::Response};
use std::future::Future;

// MCP tools and REST handlers run synchronous SQLite queries (one connection
// mutex shared with the indexer) and file reads directly in their futures. On
// the multi-thread runtime, `block_in_place` hands this worker's other tasks
// to another thread while the request runs, so a slow query, or one waiting on
// an indexing write transaction, cannot stall `/health`, the UI or other calls.
//
// Only `/mcp` and `/api` do that work, so only they pay for the hand-off, and at
// most `MAX_BLOCKING_REQUESTS` run at once. Waiting happens on a semaphore, which
// a dropped request (client disconnect) leaves for free, instead of every request
// pinning a runtime worker and forcing a replacement thread under bursts.
const MAX_BLOCKING_REQUESTS: usize = 16;

fn gate() -> &'static tokio::sync::Semaphore {
    static GATE: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Semaphore::new(MAX_BLOCKING_REQUESTS))
}

fn is_blocking_route(path: &str) -> bool {
    path == "/mcp" || path.starts_with("/mcp/") || path == "/api" || path.starts_with("/api/")
}

pub async fn blocking_handler(req: Request, next: Next) -> Response {
    if !is_blocking_route(req.uri().path()) {
        return next.run(req).await;
    }
    let _permit = gate().acquire().await.expect("offload gate is never closed");
    run_blocking(next.run(req)).await
}

async fn run_blocking<F: Future>(fut: F) -> F::Output {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| h.block_on(fut))
        }
        _ => fut.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runs_the_future_on_the_multi_thread_runtime() {
        assert_eq!(run_blocking(async { 7 }).await, 7);
    }

    #[test]
    fn only_mcp_and_api_routes_are_gated() {
        for p in ["/mcp", "/api/search", "/api", "/mcp/x"] {
            assert!(is_blocking_route(p), "{p}");
        }
        for p in ["/health", "/", "/assets/app.js", "/mcpx", "/apis"] {
            assert!(!is_blocking_route(p), "{p}");
        }
    }

    #[tokio::test]
    async fn falls_back_to_awaiting_on_a_current_thread_runtime() {
        assert_eq!(run_blocking(async { 7 }).await, 7);
    }
}
