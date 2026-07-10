mod config;
mod crypto;
mod dependencies;
mod env;
mod middleware;
mod models;
#[cfg(unix)]
mod pool;
mod queue;
mod routes;
mod runners;
mod services;

#[cfg(unix)]
use std::sync::{Arc, RwLock};

use actix_web::{web, App, HttpServer};

use crate::config::Config;
use crate::queue::QueueController;
#[cfg(unix)]
use crate::runners::LIB_PATH;

/// Global zygote manager — supports auto-restart on connection loss.
/// Only available on Unix (Linux Docker).
#[cfg(unix)]
use crate::pool::zygote::ZygoteManager;

#[cfg(unix)]
static ZYGOTE: RwLock<Option<Arc<ZygoteManager>>> = RwLock::new(None);

/// Get a handle to the current zygote manager (if alive).
///
/// Clones the `Arc` out while briefly holding the read lock, then releases
/// the guard. The returned handle is `Send`, so callers can hold it across
/// `.await` points without making their future `!Send`.
#[cfg(unix)]
pub(crate) fn get_zygote() -> Option<Arc<ZygoteManager>> {
    let guard = ZYGOTE.read().ok()?;
    guard.as_ref().cloned()
}

/// Attempt to restart the zygote worker. Called after a connection loss
/// is detected. Returns true if restart succeeded.
#[cfg(unix)]
pub(crate) async fn try_restart_zygote(config: &Config) -> bool {
    let mut guard = match ZYGOTE.write() {
        Ok(g) => g,
        Err(_) => return false,
    };
    // Already running?
    if let Some(ref z) = *guard {
        if z.is_running() {
            return true;
        }
    }
    // Kill old and start new.
    *guard = None;
    match ZygoteManager::new(
        &config.python_path,
        "./libpython.so",
        LIB_PATH,
        &config.python_zygote_preload_modules,
    ) {
        Ok(new_z) => {
            tracing::info!(modules = ?config.python_zygote_preload_modules,
                           "Python zygote pool restarted");
            *guard = Some(Arc::new(new_z));
            true
        }
        Err(e) => {
            tracing::error!("Failed to restart zygote: {e}");
            false
        }
    }
}

// Non-Unix stub.
#[cfg(not(unix))]
#[allow(dead_code)]
pub(crate) fn get_zygote() -> Option<bool> { None }

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_path =
        std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.toml".into());
    let config = Config::load(&config_path).expect("Failed to load config");

    let queue = QueueController::start(config.max_workers);

    let port = config.app.port;
    tracing::info!(
        port = port,
        max_workers = config.max_workers,
        worker_timeout = config.worker_timeout,
        python_path = %config.python_path,
        nodejs_path = %config.nodejs_path,
        "RedBear Sandbox starting (Rust)"
    );
    tracing::info!(
        python_zygote = config.python_zygote,
        enable_preload = config.enable_preload,
        enable_network = config.enable_network,
        "Sandbox runtime flags"
    );

    // Install initial dependencies
    if let Err(e) = crate::dependencies::install_python_dependencies(&config).await {
        tracing::warn!("Failed to install initial Python dependencies: {e}");
    }

    // Prepare sandbox environment (stdlib, system libs into chroot jail)
    crate::env::prepare_environment(&config).await;

    // Start pre-warmed Python zygote (if enabled) for fast cold-start.
    // Unix-only: requires fork() + seccomp, which are Linux features.
    #[cfg(unix)]
    if config.python_zygote {
        tracing::info!(
            count = config.python_zygote_preload_modules.len(),
            modules = ?config.python_zygote_preload_modules,
            "Starting Python zygote with preloaded stdlib modules"
        );
        match ZygoteManager::new(
            &config.python_path,
            "./libpython.so",
            LIB_PATH,
            &config.python_zygote_preload_modules,
        ) {
            Ok(zygote) => {
                tracing::info!(
                    modules = ?config.python_zygote_preload_modules,
                    "Python zygote pool started"
                );
                *ZYGOTE.write().unwrap() = Some(Arc::new(zygote));
            }
            Err(e) => {
                tracing::error!("Failed to start Python zygote: {e}");
            }
        }
    } else {
        #[cfg(unix)]
        tracing::info!("Python zygote disabled (set python_zygote=true to enable)");
        #[cfg(not(unix))]
        tracing::info!("Python zygote not available on this platform");
    }

    let cfg = config.clone();
    let q = queue.clone();

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(cfg.clone()))
            .app_data(web::Data::new(q.clone()))
            .route("/health", web::get().to(routes::health))
            .service(
                web::scope("/v1/sandbox")
                    .route("/run", web::post().to(routes::run_code))
                    .route("/dependencies", web::get().to(routes::get_dependencies))
                    .route(
                        "/dependencies/update",
                        web::post().to(routes::update_dependencies),
                    ),
            )
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
