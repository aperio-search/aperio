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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
            AppError::NotFound(e) => (StatusCode::NOT_FOUND, e),
            AppError::BadRequest(e) => (StatusCode::BAD_REQUEST, e),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<redb::Error> for AppError {
    fn from(e: redb::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<redb::DatabaseError> for AppError {
    fn from(e: redb::DatabaseError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<redb::TableError> for AppError {
    fn from(e: redb::TableError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<redb::StorageError> for AppError {
    fn from(e: redb::StorageError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<redb::CommitError> for AppError {
    fn from(e: redb::CommitError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<redb::TransactionError> for AppError {
    fn from(e: redb::TransactionError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[test]
    fn internal_converts_to_500() {
        let resp = AppError::Internal("boom".into()).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_to_json(resp);
        assert_eq!(body["error"], "boom");
    }

    #[test]
    fn not_found_converts_to_404() {
        let resp = AppError::NotFound("missing".into()).into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_to_json(resp);
        assert_eq!(body["error"], "missing");
    }

    #[test]
    fn bad_request_converts_to_400() {
        let resp = AppError::BadRequest("invalid".into()).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_to_json(resp);
        assert_eq!(body["error"], "invalid");
    }

    #[test]
    fn from_serde_json_error() {
        let serde_err: serde_json::Error = serde_json::from_str::<()>("invalid").unwrap_err();
        let err: AppError = serde_err.into();
        assert!(matches!(err, AppError::Internal(_)));
    }

    fn body_to_json(resp: Response) -> serde_json::Value {
        let body = resp.into_body();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let bytes = rt.block_on(async {
            let collected = BodyExt::collect(body).await.unwrap();
            collected.to_bytes()
        });
        serde_json::from_slice(&bytes).unwrap()
    }
}
