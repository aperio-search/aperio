use std::path::PathBuf;

use aperio::{auth::AuthConfig, config::AppConfig, routes, store::Store};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir).join("aperio_data");

    let config = std::env::var("CONFIG_FILE").ok().map(PathBuf::from);
    let app_config = AppConfig::load(config.as_deref());

    let log_filter = match std::env::var("RUST_LOG") {
        Ok(val) if !val.is_empty() => EnvFilter::new(val),
        _ => EnvFilter::new(
            app_config
                .log_level
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("info"),
        ),
    };

    eprintln!("aperio: initializing with log filter: {log_filter}");

    tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
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

    let dumps_folder = app_config.dumps_folder.clone().map(PathBuf::from);
    let main_api_key = app_config.main_api_key.clone();
    let search_api_key = app_config.search_api_key.clone();

    let store_config = app_config.merge_into_store_config();
    let store = Store::with_config(db, store_config);
    let store = std::sync::Arc::new(store);
    store.spawn_background();

    let mut auth = AuthConfig::default();
    if let Some(key) = main_api_key.filter(|k| !k.is_empty()) {
        auth.main_api_key = key;
    }
    if let Some(key) = search_api_key.filter(|k| !k.is_empty()) {
        auth.search_api_key = key;
    }
    let app = routes::create_router(store, auth, dumps_folder);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    tracing::info!("listening on 0.0.0.0:3000");

    axum::serve(listener, app).await.unwrap();
}
