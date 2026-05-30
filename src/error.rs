use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

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

impl From<fjall::Error> for AppError {
    fn from(e: fjall::Error) -> Self {
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
    fn from_fjall_error() {
        let fjall_err: fjall::Error = std::io::Error::other("db error").into();
        let err: AppError = fjall_err.into();
        assert!(matches!(err, AppError::Internal(_)));
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
