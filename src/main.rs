use std::path::PathBuf;

use aster::{config::AppConfig, routes, store::Store};

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir).join("aster.db");

    let config = std::env::var("CONFIG_FILE").ok().map(PathBuf::from);
    let app_config = AppConfig::load(config.as_deref());

    let mut db_builder = fjall::Database::builder(&db_path);
    if let Some(cache_size) = app_config.block_cache_size {
        db_builder = db_builder.cache_size(cache_size);
    }
    if let Some(threads) = app_config.maintenance_threads {
        db_builder = db_builder.worker_threads(threads);
    }
    let db = db_builder.open().expect("failed to open database");

    let store_config = app_config.merge_into_store_config();
    let store = Store::with_config(db, store_config);
    let app = routes::create_router(store);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
