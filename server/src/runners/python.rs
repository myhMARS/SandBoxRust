use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use tokio::process::Command;
use uuid::Uuid;

use super::{ExecutionResult, LIB_PATH};
use crate::config::Config;
use crate::crypto;
use crate::models::RunnerOptions;

/// Template embedded at compile time.
const PYTHON_PRESCRIPT: &str = include_str!("../../../prescript.py");

pub async fn run(
    config: &Config,
    code_b64: &str,
    preload: &str,
    options: &RunnerOptions,
) -> Result<ExecutionResult, String> {
    let code = base64::engine::general_purpose::STANDARD
        .decode(code_b64)
        .map_err(|e| format!("base64 decode: {e}"))?;

    let key = crypto::generate_key(64);
    let encoded_key = base64::engine::general_purpose::STANDARD.encode(&key);

    let enable_net = if options.enable_network && config.enable_network {
        1
    } else {
        0
    };

    let mut script = PYTHON_PRESCRIPT
        .replace("{{uid}}", &config.sandbox_uid.to_string())
        .replace("{{gid}}", &config.sandbox_gid.to_string())
        .replace("{{enable_network}}", &enable_net.to_string())
        .replace("{{preload}}", &format!("{preload}\n"));
    let encoded_code = crypto::encrypt_code(&code, &key);
    script = script.replace("{{code}}", &encoded_code);

    let file_name = format!("{}.py", Uuid::new_v4().simple());
    let tmp_dir = format!("{LIB_PATH}/tmp");
    let code_path = format!("{tmp_dir}/{file_name}");

    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("mkdir: {e}"))?;
    tokio::fs::write(&code_path, &script)
        .await
        .map_err(|e| format!("write: {e}"))?;

    let python_path = config.python_path.clone();
    let timeout_secs = config.worker_timeout;

    let child = Command::new(&python_path)
        .arg(&code_path)
        .arg(LIB_PATH)
        .arg(&encoded_key)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(LIB_PATH)
        .spawn()
        .map_err(|e| format!("spawn python: {e}"))?;

    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "Execution timeout".to_string())?
    .map_err(|e| format!("process: {e}"))?;

    let _ = tokio::fs::remove_file(&code_path).await;

    Ok(ExecutionResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}
