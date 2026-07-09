use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub port: u16,
    #[serde(default)]
    pub debug: bool,
    pub key: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProxyConfig {
    #[serde(default)]
    pub socks5: String,
    #[serde(default)]
    pub http: String,
    #[serde(default)]
    pub https: String,
}

impl ProxyConfig {
    pub fn socks5_option(&self) -> Option<&String> {
        if self.socks5.is_empty() { None } else { Some(&self.socks5) }
    }
    pub fn http_option(&self) -> Option<&String> {
        if self.http.is_empty() { None } else { Some(&self.http) }
    }
    pub fn https_option(&self) -> Option<&String> {
        if self.https.is_empty() { None } else { Some(&self.https) }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub app: AppConfig,

    #[serde(default = "default_max_workers")]
    pub max_workers: usize,

    #[serde(default = "default_worker_timeout")]
    pub worker_timeout: u64,

    #[serde(default = "default_true")]
    pub enable_network: bool,

    #[serde(default)]
    pub enable_preload: bool,

    #[serde(default = "default_python_path")]
    pub python_path: String,

    #[serde(default = "default_nodejs_path")]
    pub nodejs_path: String,

    #[serde(default)]
    pub sandbox_user: String,

    #[serde(default = "default_sandbox_uid")]
    pub sandbox_uid: u32,

    #[serde(default)]
    pub sandbox_gid: u32,

    #[serde(default)]
    pub proxy: ProxyConfig,
}

fn default_max_workers() -> usize { 4 }
fn default_worker_timeout() -> u64 { 30 }
fn default_true() -> bool { true }
fn default_python_path() -> String { "/usr/bin/python3".into() }
fn default_nodejs_path() -> String { "/usr/bin/node".into() }
fn default_sandbox_uid() -> u32 { 65537 }

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let mut cfg: Config = toml::from_str(&content)?;
        cfg.override_with_env();
        Ok(cfg)
    }

    fn override_with_env(&mut self) {
        if let Ok(v) = std::env::var("SANDBOX_PORT") {
            if let Ok(p) = v.parse() { self.app.port = p; }
        }
        if let Ok(v) = std::env::var("MAX_WORKERS") {
            if let Ok(p) = v.parse() { self.max_workers = p; }
        }
        if let Ok(v) = std::env::var("WORKER_TIMEOUT") {
            if let Ok(p) = v.parse() { self.worker_timeout = p; }
        }
        if let Ok(v) = std::env::var("API_KEY") { self.app.key = v; }
        if let Ok(v) = std::env::var("PYTHON_PATH") { self.python_path = v; }
        if let Ok(v) = std::env::var("NODEJS_PATH") { self.nodejs_path = v; }
        if let Ok(v) = std::env::var("ENABLE_NETWORK") {
            self.enable_network = matches!(v.as_str(), "1" | "true" | "yes");
        }
    }
}
