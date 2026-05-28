use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

#[derive(Debug)]
pub enum AppError {
    Internal(String),
    NotFound(String),
    BadRequest(String),
}

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

impl From<bincode::error::DecodeError> for AppError {
    fn from(e: bincode::error::DecodeError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<bincode::error::EncodeError> for AppError {
    fn from(e: bincode::error::EncodeError) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl From<fjall::Error> for AppError {
    fn from(e: fjall::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}
