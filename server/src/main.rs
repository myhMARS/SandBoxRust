mod config;
mod crypto;
mod middleware;
mod models;
mod queue;
mod routes;
mod runners;
mod services;

use actix_web::{web, App, HttpServer};

use crate::config::Config;
use crate::queue::QueueController;

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
        "RedBear Sandbox starting (Rust)"
    );

    // Clone for closure — HttpServer::new calls the factory Fn multiple times
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
