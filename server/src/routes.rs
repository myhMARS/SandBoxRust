use actix_web::{web, HttpResponse};
use std::time::Instant;

use crate::config::Config;
use crate::middleware::ApiKey;
use crate::models::{ApiResponse, RunCodeRequest, UpdateDependencyRequest};
use crate::queue::QueueController;
use crate::services;

pub async fn health(queue: web::Data<QueueController>) -> HttpResponse {
    let stats = &queue.stats;
    HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "role": "sandbox",
        "queue_depth": stats.queue_depth.load(std::sync::atomic::Ordering::Relaxed),
        "workers": stats.workers,
    }))
}

pub async fn run_code(
    req: web::Json<RunCodeRequest>,
    config: web::Data<Config>,
    queue: web::Data<QueueController>,
    _api_key: ApiKey,
) -> HttpResponse {
    let t_enqueue = Instant::now();
    let req = req.into_inner();
    // Clone the `web::Data` handle (an Arc) rather than deep-cloning Config.
    let cfg = config.clone();
    let language = req.language.clone();

    let result = queue
        .submit(language.clone(), move || {
            async move {
                match req.language.as_str() {
                    "python3" => {
                        services::python::run_python_code(
                            &cfg, &req.code, &req.preload, &req.options,
                        )
                        .await
                    }
                    "javascript" => {
                        services::nodejs::run_nodejs_code(
                            &cfg, &req.code, &req.preload, &req.options,
                        )
                        .await
                    }
                    _ => ApiResponse::error(400, "unsupported language"),
                }
            }
        })
        .await;

    let total_ms = t_enqueue.elapsed().as_millis();
    tracing::debug!(total_ms = total_ms, "request completed");

    HttpResponse::Ok().json(result)
}

pub async fn get_dependencies(
    query: web::Query<std::collections::HashMap<String, String>>,
    config: web::Data<Config>,
    _api_key: ApiKey,
) -> HttpResponse {
    match query.get("language").map(String::as_str) {
        Some("python3") => {
            HttpResponse::Ok().json(services::python::list_python_dependencies(&config).await)
        }
        _ => HttpResponse::Ok().json(ApiResponse::error(400, "unsupported language")),
    }
}

pub async fn update_dependencies(
    req: web::Json<UpdateDependencyRequest>,
    config: web::Data<Config>,
    _api_key: ApiKey,
) -> HttpResponse {
    match req.language.as_str() {
        "python3" => {
            HttpResponse::Ok().json(services::python::update_python_dependencies(&config).await)
        }
        _ => HttpResponse::Ok().json(ApiResponse::error(400, "unsupported language")),
    }
}
