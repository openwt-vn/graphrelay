use axum::{
    extract::Request,
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

// The daemon binds 127.0.0.1, but a browser page on any site can still reach it:
// a cross-origin fetch carries a foreign `Origin`, and a DNS-rebinding attack
// carries a foreign `Host`. The MCP Streamable HTTP spec requires rejecting both.
pub async fn local_only(req: Request, next: Next) -> Response {
    if !is_local_request(req.headers()) {
        return (StatusCode::FORBIDDEN, "graphrelay only accepts requests from localhost").into_response();
    }
    next.run(req).await
}

fn is_local_request(headers: &HeaderMap) -> bool {
    let host_ok = match headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        Some(host) => is_loopback_authority(host),
        None => true,
    };
    let origin_ok = match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(origin) => origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            .is_some_and(is_loopback_authority),
    };
    host_ok && origin_ok
}

fn is_loopback_authority(authority: &str) -> bool {
    let host = if let Some(rest) = authority.strip_prefix('[') {
        match rest.split_once(']') {
            Some((h, _)) => h,
            None => return false,
        }
    } else {
        authority.split(':').next().unwrap_or("")
    };
    matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(host: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = host { h.insert(header::HOST, HeaderValue::from_str(v).unwrap()); }
        if let Some(v) = origin { h.insert(header::ORIGIN, HeaderValue::from_str(v).unwrap()); }
        h
    }

    #[test]
    fn accepts_local_clients() {
        assert!(is_local_request(&headers(Some("127.0.0.1:3030"), None)));
        assert!(is_local_request(&headers(Some("localhost:3030"), Some("http://localhost:3030"))));
        assert!(is_local_request(&headers(Some("[::1]:3030"), Some("http://[::1]:3030"))));
        assert!(is_local_request(&headers(None, None)));
    }

    #[test]
    fn rejects_foreign_origin_and_rebound_host() {
        assert!(!is_local_request(&headers(Some("127.0.0.1:3030"), Some("https://evil.example"))));
        assert!(!is_local_request(&headers(Some("evil.example:3030"), None)));
        assert!(!is_local_request(&headers(Some("localhost.evil.example"), None)));
        assert!(!is_local_request(&headers(Some("127.0.0.1:3030"), Some("null"))));
    }
}
