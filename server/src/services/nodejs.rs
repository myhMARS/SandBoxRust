use crate::models::{ApiResponse, RunCodeData, RunnerOptions};
use crate::runners;

pub async fn run_nodejs_code(
    config: &crate::config::Config,
    code: &str,
    preload: &str,
    options: &RunnerOptions,
) -> ApiResponse {
    match runners::nodejs::run(config, code, preload, options).await {
        Ok(result) => ApiResponse::success(RunCodeData {
            stdout: result.stdout,
            stderr: result.stderr,
        }),
        Err(e) => ApiResponse::error(500, e),
    }
}
