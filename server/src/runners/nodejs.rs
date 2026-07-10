use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{ExecutionResult, LIB_PATH};
use crate::config::Config;
use crate::models::RunnerOptions;

const NODEJS_PRESCRIPT: &str = include_str!("../../../prescript.js");

/// Build the sandbox script in memory (no temp file).
/// The script is fed to Node.js via stdin (`node -`), so node reads and
/// compiles it before embedded init_seccomp() applies chroot/seccomp.
fn build_script(
    code_b64: &str,
    preload: &str,
) -> String {
    let checked_preload = preload; // enable_preload check is done in run()
    let mut script = NODEJS_PRESCRIPT.replace("{{preload}}", checked_preload);
    let eval_code = format!(
        "eval(Buffer.from('{}', 'base64').toString('utf-8'))",
        code_b64
    );
    script = script.replace("{{code}}", &eval_code);
    script
}

pub async fn run(
    config: &Config,
    code_b64: &str,
    preload: &str,
    options: &RunnerOptions,
) -> Result<ExecutionResult, String> {
    let checked_preload = if config.enable_preload { preload } else { "" };
    let enable_network = options.enable_network && config.enable_network;
    let timeout_secs = config.worker_timeout;

    let script = build_script(code_b64, checked_preload);
    let script_bytes = script.into_bytes();

    let nodejs_path = config.nodejs_path.clone();
    let sandbox_uid = config.sandbox_uid;
    let sandbox_gid = config.sandbox_gid;

    // Build options JSON string for the prescript
    let opts_json = format!(
        r#"{{"enable_network":{}}}"#,
        enable_network
    );

    // `node -` — feed the script via stdin (no temp file).
    // The interpreter reads and compiles the whole program before the
    // embedded init_seccomp() applies chroot/seccomp.
    let mut cmd = Command::new(&nodejs_path);
    cmd.env_clear();
    // Minimal environment — only what the interpreter needs.
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", &path);
    }
    cmd.env("UV_USE_IO_URING", "0");
    // Point NODE_PATH at the bundled node_modules so require('koffi')
    // resolves before init_seccomp() applies chroot.
    cmd.env("NODE_PATH", format!("{LIB_PATH}/node_modules"));
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

    cmd.arg("-")
        .arg(LIB_PATH)
        .arg(sandbox_uid.to_string())
        .arg(sandbox_gid.to_string())
        .arg(&opts_json)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(LIB_PATH)
        // On timeout the wait future is dropped; kill_on_drop guarantees the
        // child (running untrusted code) is SIGKILLed rather than orphaned.
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn nodejs: {e}"))?;

    // Write the script to stdin, then close it so Node.js starts execution.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&script_bytes)
            .await
            .map_err(|e| format!("stdin write: {e}"))?;
    }

    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "Execution timeout".to_string())?
    .map_err(|e| format!("process: {e}"))?;

    Ok(ExecutionResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: super::exit_code_from_status(output.status),
    })
}
