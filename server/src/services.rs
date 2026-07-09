//! Service layer — thin wrappers around runners, producing ApiResponse.

use crate::models::{ApiResponse, Dependency, RunCodeData, RunnerOptions};
use crate::runners;

// ── Python ──

pub async fn run_python_code(
    config: &crate::config::Config,
    code: &str,
    preload: &str,
    options: &RunnerOptions,
) -> ApiResponse {
    match runners::run_python(config, code, preload, options).await {
        Ok(result) => {
            if result.stderr.is_empty() || result.exit_code == 0 {
                ApiResponse::success(RunCodeData {
                    stdout: result.stdout,
                    stderr: result.stderr,
                })
            } else {
                ApiResponse::error(500, result.stderr)
            }
        }
        Err(e) => ApiResponse::error(500, e),
    }
}

pub async fn list_python_dependencies() -> ApiResponse {
    ApiResponse::success(serde_json::json!({"dependencies": []}))
}

pub async fn update_python_dependencies() -> ApiResponse {
    ApiResponse::success(serde_json::json!({"success": true}))
}

// ── Node.js ──

pub async fn run_nodejs_code(
    config: &crate::config::Config,
    code: &str,
    preload: &str,
    options: &RunnerOptions,
) -> ApiResponse {
    match runners::run_nodejs(config, code, preload, options).await {
        Ok(result) => ApiResponse::success(RunCodeData {
            stdout: result.stdout,
            stderr: result.stderr,
        }),
        Err(e) => ApiResponse::error(500, e),
    }
}
