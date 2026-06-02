use std::path::PathBuf;

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use aperio::{auth::AuthConfig, config::AppConfig, routes, store::Store};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir).join("aperio");

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

    std::fs::create_dir_all(&db_path).expect("failed to create data directory");
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(1024 * 1024 * 1024)
            .max_dbs(4)
            .open(&db_path)
            .expect("failed to open database environment")
    };

    let dumps_folder = app_config.dumps_folder.clone().map(PathBuf::from);
    let main_api_key = app_config.main_api_key.clone();
    let search_api_key = app_config.search_api_key.clone();

    let store_config = app_config.merge_into_store_config();
    let store = Store::with_config(env, store_config);
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

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
