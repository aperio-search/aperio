use std::path::PathBuf;

use aperio::{config::AppConfig, routes, store::Store};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir).join("aperio.db");

    let config = std::env::var("CONFIG_FILE").ok().map(PathBuf::from);
    let app_config = AppConfig::load(config.as_deref());

    let log_filter = match std::env::var("RUST_LOG") {
        Ok(val) => EnvFilter::new(val),
        Err(_) => EnvFilter::new(app_config.log_level.as_deref().unwrap_or("info")),
    };
    tracing_subscriber::fmt()
        .with_env_filter(log_filter)
        .init();

    tracing::info!(
        data_dir = %data_dir,
        db_path = %db_path.display(),
        "starting aperio"
    );

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

    tracing::info!("listening on 0.0.0.0:3000");

    axum::serve(listener, app).await.unwrap();
}
