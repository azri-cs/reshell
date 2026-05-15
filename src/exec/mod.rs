pub mod analyze;
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
    /// Unique ID for this execution, for linking to recovery and feedback flows.
    pub execution_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_id: Option<String>,
    pub suggestion: serde_json::Value,
    pub output: OutputInfo,
    /// When the command failed, a ready-to-use tool call the agent can make
    /// to get a recovery suggestion (calls `rsh_recover`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<NextAction>,
    /// When output was truncated, a hint telling the agent how to compact it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction_hint: Option<CompactionHint>,
    /// Platform where this execution ran (for pattern matching context).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Non-fatal warnings (e.g., blocked env vars, security notices).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
    /// If a high-confidence R22 fix was auto-applied, the result of that attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_retry: Option<Box<ExecResult>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextAction {
    /// MCP tool name to call next.
    pub tool: String,
    /// Parameters to pass (ready to use).
    pub params: serde_json::Value,
    /// Human-readable reason for the suggested action.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionHint {
    /// MCP tool name to call for compacted view.
    pub tool: String,
    /// Parameters including output_id and suggested view.
    pub params: serde_json::Value,
    /// Human-readable reason.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputInfo {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub truncated: bool,
}
