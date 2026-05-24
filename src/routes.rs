use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};

use crate::error::AppError;
use crate::models::{
    CollectionInfo, SearchParams, SearchResponse, StatusResponse, SuggestParams, SuggestResponse,
    UpsertRequest,
};
use crate::store::Store;

pub struct AppState {
    pub store: Store,
}

    fn router_with_state(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/status", get(status))
            .route("/collections/{collection}/items", post(upsert_item))
            .route("/collections/{collection}/search", get(search))
            .route("/collections/{collection}/suggest", get(suggest))
            .route("/collections/{collection}/items/{id}", delete(delete_item))
            .route("/collections/{collection}", get(collection_info))
            .route("/collections/{collection}", delete(delete_collection))
            .with_state(state)
            .fallback(not_found)
    }

pub fn create_router(store: Store) -> Router {
    let state = Arc::new(AppState { store });
    router_with_state(state)
}

async fn status() -> Json<StatusResponse> {
    Json(StatusResponse { ok: true })
}

async fn upsert_item(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Json(body): Json<UpsertRequest>,
) -> Result<StatusCode, AppError> {
    state.store.upsert(&collection, &body.id, &body.content)?;
    Ok(StatusCode::OK)
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, AppError> {
    let take = params.take.unwrap_or(20).clamp(1, 100);
    let t0 = std::time::Instant::now();
    let (results, total) =
        state.store.search(&collection, &params.q, take)?;
    let elapsed = t0.elapsed();
    println!(
        "search '{}' on '{}': {:?} ({} hits)",
        params.q, collection, elapsed, total
    );
    Ok(Json(SearchResponse { results, total, take }))
}

async fn suggest(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Query(params): Query<SuggestParams>,
) -> Result<Json<SuggestResponse>, AppError> {
    let suggestions = state.store.suggest(&collection, &params.q)?;
    Ok(Json(SuggestResponse { suggestions }))
}

async fn delete_item(
    State(state): State<Arc<AppState>>,
    Path((collection, id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    state.store.delete_item(&collection, &id)?;
    Ok(StatusCode::OK)
}

async fn collection_info(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
) -> Result<Json<CollectionInfo>, AppError> {
    let info = state.store.collection_info(&collection)?;
    Ok(Json(info))
}

async fn delete_collection(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
) -> Result<StatusCode, AppError> {
    state.store.delete_collection(&collection)?;
    Ok(StatusCode::OK)
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "not found" })),
    )
}
