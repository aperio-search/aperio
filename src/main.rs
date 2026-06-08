use std::path::PathBuf;
use std::process::ExitCode;

use tikv_jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

use aperio::{auth::AuthConfig, config::AppConfig, routes, store::Store};
use tracing_subscriber::EnvFilter;

/// Bind address for the HTTP server. Kept as a constant rather than a CLI
/// flag for now — the only way to override it is to fork.
const BIND_ADDR: &str = "0.0.0.0:3000";

#[tokio::main]
async fn main() -> ExitCode {
    // Run the actual startup inside a fallible function so any error is
    // caught, logged through the (possibly-uninitialised) tracing pipeline
    // AND also written to stderr, and we exit with a non-zero code rather
    // than panicking.
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Always emit on stderr in case tracing was never initialised
            // (config load can fail before the subscriber is installed).
            eprintln!("aperio: fatal: {e}");
            tracing::error!(error = %e, "aperio: fatal startup error; exiting");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let db_path = PathBuf::from(&data_dir);

    let config = std::env::var("CONFIG_FILE").ok().map(PathBuf::from);
    let app_config = AppConfig::load(config.as_deref())?;

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

    std::fs::create_dir_all(&db_path).map_err(|e| {
        format!(
            "failed to create data directory '{}': {e}",
            db_path.display()
        )
    })?;

    // SAFETY: We are the sole owner of this LMDB env for the lifetime of
    // the process; no other code maps the same path concurrently.
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(10 * 1024 * 1024 * 1024 * 1024) // 10 TB
            .max_dbs(4)
            .open(&db_path)
            .map_err(|e| {
                format!(
                    "failed to open LMDB environment at '{}': {e}",
                    db_path.display()
                )
            })?
    };

    let dumps_folder = app_config.dumps_folder.clone().map(PathBuf::from);
    let main_api_key = app_config.main_api_key.clone();
    let search_api_key = app_config.search_api_key.clone();

    let store_config = app_config.merge_into_store_config();
    let fst_path = db_path.join("fst");
    std::fs::create_dir_all(&fst_path).map_err(|e| {
        format!(
            "failed to create FST directory '{}': {e}",
            fst_path.display()
        )
    })?;

    let store = Store::with_config(env, store_config, fst_path)
        .map_err(|e| format!("failed to initialise store: {e}"))?;
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

    let listener = tokio::net::TcpListener::bind(BIND_ADDR)
        .await
        .map_err(|e| format!("failed to bind {BIND_ADDR}: {e}"))?;

    tracing::info!("listening on {BIND_ADDR}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("axum::serve terminated unexpectedly: {e}"))?;

    Ok(())
}

/// Wait for either SIGINT (Ctrl+C) or SIGTERM. If either signal handler
/// fails to install (rare — usually only happens under sandboxing or seccomp
/// policies), log a warning and return immediately so the caller still
/// shuts down cleanly instead of panicking on `.expect()`.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(
                error = %e,
                "failed to wait on Ctrl+C handler; treating as immediate shutdown"
            );
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to install SIGTERM handler; relying on Ctrl+C only"
                );
                // Park forever so the `select!` below picks Ctrl+C if it fires.
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
