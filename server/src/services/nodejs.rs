use crate::models::{ApiResponse, RunCodeData, RunnerOptions};
use crate::runners;
use super::is_seccomp_violation;

pub async fn run_nodejs_code(
    config: &crate::config::Config,
    code: &str,
    preload: &str,
    options: &RunnerOptions,
) -> ApiResponse {
    match runners::nodejs::run(config, code, preload, options).await {
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
