use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use uuid::Uuid;

use super::{ExecutionResult, LIB_PATH};
use crate::config::Config;
use crate::models::RunnerOptions;

const NODEJS_PRESCRIPT: &str = include_str!("../../../prescript.js");

pub async fn run(
    config: &Config,
    code_b64: &str,
    preload: &str,
    options: &RunnerOptions,
) -> Result<ExecutionResult, String> {
    let checked_preload = if config.enable_preload { preload } else { "" };

    // Template replacement — Python version uses XOR crypto,
    // Node.js version uses base64 eval injection directly.
    let mut script = NODEJS_PRESCRIPT.replace("{{preload}}", checked_preload);
    let eval_code = format!(
        "eval(Buffer.from('{}', 'base64').toString('utf-8'))",
        code_b64
    );
    script = script.replace("{{code}}", &eval_code);

    let file_name = format!("{}.js", Uuid::new_v4().simple());
    let tmp_dir = format!("{LIB_PATH}/tmp");
    let code_path = format!("{tmp_dir}/{file_name}");

    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("mkdir: {e}"))?;
    tokio::fs::write(&code_path, &script)
        .await
        .map_err(|e| format!("write: {e}"))?;

    let nodejs_path = config.nodejs_path.clone();
    let timeout_secs = config.worker_timeout;
    let sandbox_uid = config.sandbox_uid;
    let sandbox_gid = config.sandbox_gid;
    let enable_network = options.enable_network && config.enable_network;

    // Build options JSON string for the prescript
    let opts_json = format!(
        r#"{{"enable_network":{}}}"#,
        enable_network
    );

    let mut cmd = Command::new(&nodejs_path);
    cmd.env("UV_USE_IO_URING", "0");
    if let Some(socks5) = config.proxy.socks5_option() {
        cmd.env("HTTPS_PROXY", socks5).env("HTTP_PROXY", socks5);
    } else {
        if let Some(h) = config.proxy.https_option() {
            cmd.env("HTTPS_PROXY", h);
        }
        if let Some(h) = config.proxy.http_option() {
            cmd.env("HTTP_PROXY", h);
        }
    }

    cmd.arg(&code_path)
        .arg(LIB_PATH)
        .arg(sandbox_uid.to_string())
        .arg(sandbox_gid.to_string())
        .arg(&opts_json)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(LIB_PATH);

    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn nodejs: {e}"))?;

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
