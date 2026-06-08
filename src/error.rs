use std::fmt;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub enum AppError {
    Internal(String),
    NotFound(String),
    BadRequest(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Internal(msg) => write!(f, "internal error: {msg}"),
            AppError::NotFound(msg) => write!(f, "not found: {msg}"),
            AppError::BadRequest(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AppError {}

/// Body returned to clients for any `AppError::Internal`. The real error
/// detail is logged server-side and intentionally not exposed: an
/// internal-error message can contain filesystem paths, library version
/// strings, or other state that should not leak across a trust boundary.
const INTERNAL_ERROR_BODY: &str = "internal server error";

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Internal(detail) => {
                // Full detail goes to the operator via tracing; the client
                // only sees the generic body.
                tracing::error!(error = %detail, "internal error returned to client");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": INTERNAL_ERROR_BODY })),
                )
                    .into_response()
            }
            AppError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            AppError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        }
    }
}

impl From<heed::Error> for AppError {
    fn from(e: heed::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        // serde_json errors reach this `From` only from internal call sites
        // (decoding stored documents, encoding internal payloads). HTTP
        // request bodies are decoded by axum's `Json<T>` extractor, which
        // returns its own 400 without going through `AppError`.
        AppError::Internal(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[test]
    fn internal_returns_generic_body_and_500() -> Result<(), Box<dyn std::error::Error>> {
        // Regression: previously the internal-error detail was echoed back
        // to the client, leaking LMDB/serde messages (filesystem paths,
        // internal state). Now the body is a fixed generic message and the
        // real detail is logged server-side only.
        let resp = AppError::Internal("secret detail: /var/data/aperio".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_to_json(resp)?;
        assert_eq!(body["error"], INTERNAL_ERROR_BODY);
        // The Display impl still carries the detail (used by the tracing
        // log) — pin that contract so it doesn't regress.
        let display = AppError::Internal("secret detail: /var/data/aperio".into()).to_string();
        assert!(display.contains("secret detail"));
        Ok(())
    }

    #[test]
    fn not_found_converts_to_404() -> Result<(), Box<dyn std::error::Error>> {
        let resp = AppError::NotFound("missing".into()).into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_to_json(resp)?;
        assert_eq!(body["error"], "missing");
        Ok(())
    }

    #[test]
    fn bad_request_converts_to_400() -> Result<(), Box<dyn std::error::Error>> {
        let resp = AppError::BadRequest("invalid".into()).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_to_json(resp)?;
        assert_eq!(body["error"], "invalid");
        Ok(())
    }

    #[test]
    fn from_serde_json_error() -> Result<(), Box<dyn std::error::Error>> {
        let serde_err: serde_json::Error =
            serde_json::from_str::<()>("invalid").expect_err("parse should fail");
        let err: AppError = serde_err.into();
        assert!(matches!(err, AppError::Internal(_)));
        Ok(())
    }

    fn body_to_json(resp: Response) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let body = resp.into_body();
        let rt = tokio::runtime::Runtime::new()?;
        let bytes = rt.block_on(async {
            BodyExt::collect(body)
                .await
                .map(|collected| collected.to_bytes())
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
