pub mod analyze;
pub mod audit;
pub mod runner;
pub mod validator;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BinaryHandling {
    /// Return a structured summary (MIME, hash, byte count, first/last bytes).
    #[default]
    Summary,
    /// Reject binary output with an error.
    Reject,
    /// Allow raw binary through (legacy behavior, not recommended for MCP).
    Allow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout: u64,
    pub env: HashMap<String, String>,
    pub retry: bool,
    #[serde(default)]
    pub binary_handling: BinaryHandling,
    /// Per-call tracing context (session id, request id, scrubbed raw args).
    /// Populated by the MCP server; absent for direct CLI invocations.
    /// Skipped on the wire so it never appears in JSON input/output.
    #[serde(skip)]
    pub call_ctx: Option<ExecCallContext>,
    /// The raw, scrubbed JSON of the original tool-call arguments, for the
    /// audit log. Already secret-scrubbed by the caller before being set.
    #[serde(skip)]
    pub raw_args_json: Option<String>,
    /// Human approval for a high-risk command (R28). Set to true by the agent
    /// after the host's permission UI confirms; lets the command through the
    /// validator's risk-tier check. Skipped on the wire (the wrapper reads it).
    #[serde(skip, default)]
    pub approve: bool,
}

/// Tracing context attached to an `ExecRequest` so audit rows can correlate
/// an execution back to the MCP session and JSON-RPC request that caused it.
/// Kept minimal and free of MCP-specific types so `exec` does not depend on
/// the `mcp` module.
#[derive(Debug, Clone)]
pub struct ExecCallContext {
    /// One per server process — the cross-request correlation id.
    pub session_id: String,
    /// The JSON-RPC request id (stringified).
    pub request_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub binary_summary: Option<crate::utils::BinarySummary>,
}
