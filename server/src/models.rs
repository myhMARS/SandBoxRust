use serde::{Deserialize, Serialize};

// ── Request models ──

#[derive(Debug, Clone, Deserialize)]
pub struct RunCodeRequest {
    pub language: String,
    pub code: String,
    #[serde(default)]
    pub preload: String,
    #[serde(default)]
    pub options: RunnerOptions,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RunnerOptions {
    #[serde(default = "default_true")]
    pub enable_network: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDependencyRequest {
    pub language: String,
}

// ── Response models ──

#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunCodeData {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

impl ApiResponse {
    pub fn success<T: Serialize>(data: T) -> Self {
        Self {
            code: 0,
            message: "success".into(),
            data: Some(serde_json::to_value(data).unwrap_or_default()),
        }
    }

    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

fn default_true() -> bool { true }
