use std::path::PathBuf;

use aster::{config::AppConfig, routes, store::Store};

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir).join("aster.db");
    let db = fjall::Database::builder(&db_path).open().expect("failed to open database");

    let config = std::env::var("CONFIG_FILE").ok().map(PathBuf::from);
    let app_config = AppConfig::load(config.as_deref());
    let store_config = app_config.merge_into_store_config();
    let store = Store::with_config(db, store_config);
    let app = routes::create_router(store);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
