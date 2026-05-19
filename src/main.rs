use axum::{routing::get, Json, Router};
use serde::Serialize;

#[derive(Serialize)]
struct Status {
    ok: bool,
}

async fn status() -> Json<Status> {
    Json(Status { ok: true })
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/status", get(status));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
