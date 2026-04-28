use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: Option<i64>,
    pub command_hash: String,
    pub command_template: String,
    pub recovery_code: String,
    pub stderr_pattern: String,
    pub fix_command: Option<String>,
    pub fix_success_rate: f64,
    pub last_used: Option<DateTime<Utc>>,
    pub usage_count: i64,
    /// Platform where this pattern was observed: "linux", "macos", "windows-wsl", "unknown"
    pub platform_tag: Option<String>,
}

/// Returns the current platform tag for pattern storage/lookup.
pub fn current_platform_tag() -> &'static str {
    match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows-wsl",
        _ => "unknown",
    }
}
