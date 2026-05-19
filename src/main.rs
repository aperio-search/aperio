use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::sync::Arc;

struct AppState {
    db: Db,
}

#[derive(Deserialize)]
struct IngestRequest {
    id: String,
    content: String,
}

#[derive(Serialize)]
struct IngestResponse {
    indexed: bool,
}

#[derive(Deserialize)]
struct SearchParams {
    query: String,
}

#[derive(Serialize)]
struct SearchResponse {
    ids: Vec<String>,
}

fn normalize(word: &str) -> String {
    word.to_lowercase()
}

fn key(collection: &str, bucket: &str, word: &str) -> String {
    format!("{}:{}:{}", collection, bucket, word)
}

async fn ingest(
    State(state): State<Arc<AppState>>,
    Path((collection, bucket)): Path<(String, String)>,
    Json(body): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, (StatusCode, Json<serde_json::Value>)> {
    let words: std::collections::HashSet<String> = body
        .content
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(normalize)
        .collect();

    for word in words {
        let k = key(&collection, &bucket, &word);
        let existing = state.db.get(k.as_bytes()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;

        let mut ids: Vec<String> = match existing {
            Some(data) => serde_json::from_slice(&data).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
            })?,
            None => Vec::new(),
        };

        if !ids.contains(&body.id) {
            ids.push(body.id.clone());
            let value = serde_json::to_vec(&ids).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
            })?;
            state.db.insert(k.as_bytes(), value).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
            })?;
        }
    }

    Ok(Json(IngestResponse { indexed: true }))
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path((collection, bucket)): Path<(String, String)>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, (StatusCode, Json<serde_json::Value>)> {
    let word = normalize(&params.query);
    let k = key(&collection, &bucket, &word);

    let existing = state.db.get(k.as_bytes()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
    })?;

    let ids: Vec<String> = match existing {
        Some(data) => serde_json::from_slice(&data).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?,
        None => Vec::new(),
    };

    Ok(Json(SearchResponse { ids }))
}

async fn status() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true}))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "not found"})),
    )
}

#[tokio::main]
async fn main() {
    let db = sled::open("data/aster.db").expect("failed to open database");

    let app = Router::new()
        .route("/status", get(status))
        .route("/ingest/{collection}/{bucket}", post(ingest))
        .route("/search/{collection}/{bucket}", get(search))
        .with_state(Arc::new(AppState { db }))
        .fallback(not_found);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
