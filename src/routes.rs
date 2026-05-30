use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use tower_http::trace::TraceLayer;

use crate::auth::AuthConfig;
use crate::error::AppError;
use crate::models::{
    BackupFile, CollectionCreated, CollectionInfo, CreateCollectionRequest, ExportResponse,
    ImportResponse, ListCollectionsResponse, SearchParams, SearchResponse, StatusResponse,
    SuggestParams, SuggestResponse,
};
use crate::store::Store;

pub struct AppState {
    pub store: Arc<Store>,
    pub dumps_folder: Option<PathBuf>,
}

fn router_with_state(state: Arc<AppState>, auth: AuthConfig) -> Router {
    Router::new()
        .route("/status", get(status))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route("/collections/{collection}/items", post(upsert_item))
        .route("/collections/{collection}/search", get(search))
        .route("/collections/{collection}/suggest", get(suggest))
        .route("/collections/{collection}/items/{id}", delete(delete_item))
        .route("/collections/{collection}", get(collection_info))
        .route("/collections/{collection}", delete(delete_collection))
        .route("/backup/export", post(export_handler))
        .route("/backup/import", post(import_handler))
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(auth, crate::auth::check_auth))
        .with_state(state)
        .fallback(not_found)
}

pub fn create_router(store: Arc<Store>, auth: AuthConfig, dumps_folder: Option<PathBuf>) -> Router {
    let state = Arc::new(AppState { store, dumps_folder });
    router_with_state(state, auth)
}

async fn status() -> Json<StatusResponse> {
    Json(StatusResponse { ok: true })
}

async fn list_collections(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ListCollectionsResponse>, AppError> {
    let response = state.store.list_collections()?;
    Ok(Json(response))
}

async fn create_collection(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateCollectionRequest>,
) -> Result<(StatusCode, Json<CollectionCreated>), AppError> {
    let created =
        state
            .store
            .create_collection(&body.name, &body.id_type, &body.searchable_fields)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn upsert_item(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, AppError> {
    state.store.upsert(&collection, body)?;
    Ok(StatusCode::OK)
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(collection): Path<String>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, AppError> {
    let sort_desc = params.sort.as_deref().unwrap_or("desc") != "asc";
    let take = params.take.unwrap_or(20).clamp(1, 100);
    let after = params.after.as_deref();
    let t0 = std::time::Instant::now();
    let results = state
        .store
        .search(&collection, &params.q, sort_desc, take, after)?;
    let elapsed = t0.elapsed();
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    tracing::info!(
        query = %params.q,
        collection = %collection,
        elapsed_us = elapsed.as_micros(),
        results = results.len(),
        "search completed"
    );
    Ok(Json(SearchResponse { results, take, elapsed_ms }))
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

fn dump_filename() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Format as UTC date-time without colons (filesystem-safe)
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    // Days since epoch → year-month-day (simplified, good until 2100)
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{hours:02}-{minutes:02}-{seconds:02}.aperio")
}

/// Convert days since 1970-01-01 to (year, month, day).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn require_dumps_folder(state: &AppState) -> Result<&PathBuf, AppError> {
    state
        .dumps_folder
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("dumps_folder not configured — set it in aperio.toml".into()))
}

async fn export_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ExportResponse>, AppError> {
    let dumps = require_dumps_folder(&state)?;
    let data = state.store.export_snapshot()?;
    let file = dump_filename();
    let path = dumps.join(&file);
    std::fs::create_dir_all(dumps)
        .map_err(|e| AppError::Internal(format!("failed to create dumps folder: {e}")))?;
    std::fs::write(&path, &data)
        .map_err(|e| AppError::Internal(format!("failed to write export file: {e}")))?;
    let size = data.len() as u64;
    tracing::info!(file = %file, size, "export completed");
    Ok(Json(ExportResponse { ok: true, size, file }))
}

async fn import_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BackupFile>,
) -> Result<Json<ImportResponse>, AppError> {
    let dumps = require_dumps_folder(&state)?;
    let path = dumps.join(&body.name);
    let data = std::fs::read(&path)
        .map_err(|e| AppError::Internal(format!("failed to read '{}': {e}", body.name)))?;
    state.store.import_snapshot(&data)?;
    tracing::info!(file = %body.name, "import completed");
    Ok(Json(ImportResponse { ok: true }))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "not found" })),
    )
}
