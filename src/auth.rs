use axum::Json;
use axum::extract::{Request, State};
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
    let path = req.uri().path();

    if path == "/status" {
        return next.run(req).await;
    }

    let token = match req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
    {
        Some(t) => t,
        None => {
            return unauthorized();
        }
    };

    let is_search = path.ends_with("/search");

    if is_search {
        if token == auth.main_api_key || token == auth.search_api_key {
            return next.run(req).await;
        }
    } else if token == auth.main_api_key {
        return next.run(req).await;
    }

    unauthorized()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized"})),
    )
        .into_response()
}
