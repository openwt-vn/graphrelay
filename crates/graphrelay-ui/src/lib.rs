use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

/// Returns a router with only the `/assets/*` route.
/// The caller mounts the index at `"/"` and passes the project base via
/// `serve_index_for(base)` to avoid Axum 0.7 nest + merge root-route quirks.
pub fn router() -> Router {
    Router::new()
        .route("/assets/*path", get(serve_asset_handler))
}

/// Serve index.html with `__BASE__` replaced by the actual project prefix
/// (e.g. "/graphrelay"). Must be called by the project router, not the UI router,
/// so the project name is available.
pub async fn serve_index_for(base: String) -> impl IntoResponse {
    match Assets::get("index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(&content.data)
                .replace("__BASE__", &base);
            (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                html,
            ).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

pub async fn serve_index_handler() -> impl IntoResponse {
    serve_file("index.html")
}

pub async fn serve_asset_handler(Path(path): Path<String>) -> impl IntoResponse {
    serve_file(&path)
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".js")        { "application/javascript; charset=utf-8" }
    else if path.ends_with(".css")  { "text/css; charset=utf-8" }
    else if path.ends_with(".html") { "text/html; charset=utf-8" }
    else                            { "application/octet-stream" }
}

fn serve_file(path: &str) -> Response {
    match Assets::get(path) {
        Some(content) => (
            [(header::CONTENT_TYPE, content_type(path))],
            content.data.to_vec(),
        ).into_response(),
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
