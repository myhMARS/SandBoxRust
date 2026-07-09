use super::ExecutionResult;
use crate::config::Config;
use crate::models::RunnerOptions;

/// Node.js runner (stub — not yet implemented).
pub async fn run(
    _config: &Config,
    _code_b64: &str,
    _preload: &str,
    _options: &RunnerOptions,
) -> Result<ExecutionResult, String> {
    Err("nodejs runner not yet implemented".into())
}
