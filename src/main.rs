use std::path::PathBuf;

use aster::{routes, store::Store};

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir).join("aster.db");
    let db = fjall::Database::builder(&db_path).open().expect("failed to open database");
    let store = Store::new(db);
    let app = routes::create_router(store);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
