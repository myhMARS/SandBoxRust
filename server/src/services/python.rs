use crate::dependencies;
use crate::models::{ApiResponse, Dependency, RunCodeData, RunnerOptions};
use crate::runners;
use super::is_seccomp_violation;

pub async fn run_python_code(
    config: &crate::config::Config,
    code: &str,
    preload: &str,
    options: &RunnerOptions,
) -> ApiResponse {
    match runners::python::run(config, code, preload, options).await {
        Ok(result) => {
            if is_seccomp_violation(result.exit_code) {
                return ApiResponse::error(31, "sandbox security policy violation");
            }
            if !result.stderr.is_empty() && result.exit_code != 0 {
                return ApiResponse::error(500, result.stderr);
            }
            ApiResponse::success(RunCodeData {
                stdout: result.stdout,
                stderr: result.stderr,
            })
        }
        Err(e) => ApiResponse::error(500, e),
    }
}

pub async fn list_python_dependencies(config: &crate::config::Config) -> ApiResponse {
    let packages = dependencies::list_python_packages(config).await;
    let deps: Vec<Dependency> = packages
        .into_iter()
        .map(|p| Dependency {
            name: p.name,
            version: p.version,
        })
        .collect();
    ApiResponse::success(serde_json::json!({ "dependencies": deps }))
}

pub async fn update_python_dependencies(config: &crate::config::Config) -> ApiResponse {
    match dependencies::install_python_dependencies(config).await {
        Ok(()) => ApiResponse::success(serde_json::json!({"success": true})),
        Err(e) => ApiResponse::error(500, e),
    }
}
