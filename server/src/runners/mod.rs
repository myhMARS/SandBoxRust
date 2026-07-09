pub mod python;
pub mod nodejs;


/// Result of a code execution.
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Path where the seccomp library, prescript, and temp dirs live.
pub const LIB_PATH: &str = "/usr/local/share/sandbox";
