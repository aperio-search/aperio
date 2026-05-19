use axum::{routing::get, Json, Router, http::StatusCode};
use serde::Serialize;

#[derive(Serialize)]
struct Status {
    ok: bool,
}

async fn status() -> Json<Status> {
    Json(Status { ok: true })
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" })))
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/status", get(status))
        .fallback(not_found);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
