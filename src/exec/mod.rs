pub mod runner;
pub mod validator;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout: u64,
    pub env: HashMap<String, String>,
    pub retry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub status: String,
    pub recovery_code: String,
    pub recovery_class: String,
    pub original_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_id: Option<String>,
    pub suggestion: serde_json::Value,
    pub output: OutputInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputInfo {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub truncated: bool,
}
