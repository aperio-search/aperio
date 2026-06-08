use axum::Json;
use axum::extract::{Request, State};
use axum::extract::MatchedPath;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

#[derive(Clone)]
pub struct AuthConfig {
    pub main_api_key: String,
    pub search_api_key: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            main_api_key: "SecretApiKey".into(),
            search_api_key: "PublicApiKey".into(),
        }
    }
}

pub async fn check_auth(State(auth): State<AuthConfig>, req: Request, next: Next) -> Response {
    if req.uri().path() == "/status" {
        return next.run(req).await;
    }

    let token = match req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        Some(t) => t,
        None => return unauthorized(),
    };

    // Use the matched route pattern (e.g. "/collections/{collection}/search")
    // rather than the raw request path. A user-controlled segment cannot then
    // be crafted to look like a search-tier route (e.g. a collection literally
    // named "search" or an item id "suggest").
    let is_search = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| {
            let p = m.as_str();
            p == "/collections/{collection}/search" || p == "/collections/{collection}/suggest"
        })
        .unwrap_or(false);

    if is_search {
        if constant_time_eq(token.as_bytes(), auth.main_api_key.as_bytes())
            || constant_time_eq(token.as_bytes(), auth.search_api_key.as_bytes())
        {
            return next.run(req).await;
        }
    } else if constant_time_eq(token.as_bytes(), auth.main_api_key.as_bytes()) {
        return next.run(req).await;
    }

    unauthorized()
}

/// Constant-time byte-slice equality. Returns false when lengths differ
/// (an attacker already learns length via TCP framing, so this is fine).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized"})),
    )
        .into_response()
}
