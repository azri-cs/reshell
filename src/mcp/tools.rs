use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

use crate::exec::{ExecRequest, runner::Runner};
use crate::env::Detector;
use crate::classify::taxonomy::RecoveryCode;
use crate::recover::suggest;
use crate::compact::view::{CompactView, render_view};
use crate::sandbox::paths;

use super::server::ServerState;

#[derive(Debug, serde::Serialize)]
pub struct ToolResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip)]
    pub is_error: bool,
}

pub fn list_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "rsh_exec",
            "description": "Execute a shell command with automatic failure classification, secret scrubbing, and recovery suggestions. PREFER THIS over raw bash for any command that might fail. When this returns status='failed', check the next_action field for the recovery tool to call, or use rsh_recover with the returned recovery_code and original_command. When output is truncated (truncated=true), use rsh_compact with the returned output_id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The shell command to execute" },
                    "cwd": { "type": "string", "description": "Working directory" },
                    "timeout": { "type": "integer", "default": 120, "description": "Maximum execution time in seconds (capped at 600)" },
                    "env": { "type": "object", "additionalProperties": { "type": "string" } },
                    "retry": { "type": "boolean", "default": true, "description": "Automatically retry with a fallback shell on environment mismatch (R25)" }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "rsh_env",
            "description": "Detect and describe the current shell environment: OS, shell type/version, available dev tools, package manager. Call this at the start of a session to understand the target environment before running commands.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "rsh_recover",
            "description": "Get a deterministic recovery strategy for a known failure. CALL THIS when rsh_exec returns status='failed' with a recovery_code other than R10. Pass the recovery_code and original_command from the rsh_exec response (or use the next_action field which provides ready-to-use parameters).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recovery_code": { "type": "string", "description": "Recovery code from rsh_exec response (R20-R30)" },
                    "original_command": { "type": "string", "description": "The command that failed (from rsh_exec original_command)" },
                    "context": { "type": "string", "description": "Optional additional context about the failure" }
                },
                "required": ["recovery_code", "original_command"]
            }
        }),
        json!({
            "name": "rsh_compact",
            "description": "Retrieve a compacted view of a previously stored large output. Use when rsh_exec returns truncated=true or when inspecting previous command outputs. Views: 'skeleton' (structural summary — function defs, error/warn lines, class/struct defs), 'diff' (only new lines since previous read), 'errors_only' (only ERROR/WARN/FATAL lines), 'full' (complete output).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "output_id": { "type": "string", "description": "The output_id from a previous rsh_exec response" },
                    "file": { "type": "string", "description": "Path to a file to compact (must be within working directory)" },
                    "view": { "type": "string", "enum": ["full", "skeleton", "diff", "errors_only"], "default": "skeleton", "description": "Compaction view to apply" }
                }
            }
        }),
        json!({
            "name": "rsh_check",
            "description": "Quick health check and onboarding guide. Call this at the start of a session to verify reshell is functioning and to get usage guidance for the recovery pipeline (rsh_exec → rsh_recover → rsh_compact).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    ]
}

pub(crate) async fn handle_tool_call(name: &str, arguments: Value, state: &Arc<Mutex<ServerState>>) -> ToolResponse {
    match name {
        "rsh_exec" => {
            let wrapper = match serde_json::from_value::<ExecRequestWrapper>(arguments) {
                Ok(w) => w,
                Err(e) => return ToolResponse {
                    status: "error".to_string(),
                    error: Some(format!("Invalid arguments: {}", e)),
                    data: None,
                    is_error: true,
                },
            };
            let req = ExecRequest {
                command: wrapper.command,
                cwd: wrapper.cwd,
                timeout: wrapper.timeout.unwrap_or(120).min(600),
                env: wrapper.env.unwrap_or_default(),
                retry: wrapper.retry.unwrap_or(true),
            };
            let store = {
                let s = state.lock().await;
                s.store.clone()
            };
            let runner = Runner::with_store(store);
            match runner.run(&req).await {
                Ok(result) => ToolResponse {
                    status: result.status.clone(),
                    error: None,
                    data: Some(json!(result)),
                    is_error: result.status == "failed",
                },
                Err(e) => ToolResponse {
                    status: "error".to_string(),
                    error: Some(e.to_string()),
                    data: None,
                    is_error: true,
                },
            }
        }
        "rsh_env" => {
            let detector = Detector::cached().await.clone();
            ToolResponse {
                status: "success".to_string(),
                error: None,
                data: Some(json!(detector)),
                is_error: false,
            }
        }
        "rsh_check" => {
            let detector = Detector::cached().await.clone();
            let store = {
                let s = state.lock().await;
                s.store.clone()
            };
            let pattern_count = store.pattern_count().await.unwrap_or(0);

            let guidance = json!({
                "status": "healthy",
                "environment": {
                    "os": detector.os,
                    "shell": detector.shell,
                    "package_manager": detector.package_manager,
                },
                "usage": {
                    "workflow": "rsh_exec → on failure check next_action → call rsh_recover → rsh_exec (retry with fix)",
                    "tools": {
                        "rsh_exec": "Execute commands with automatic failure classification and recovery hints. Always use this instead of raw bash.",
                        "rsh_recover": "Call when rsh_exec returns status='failed'. Pass recovery_code and original_command from the response.",
                        "rsh_compact": "Call when rsh_exec returns truncated=true. Use the output_id from the response with view='skeleton' for structural summary.",
                        "rsh_env": "Detect OS, shell, available dev tools, and package manager. Call at session start.",
                        "rsh_check": "This tool — verify reshell is working and get usage guidance."
                    },
                    "recovery_codes": {
                        "R10": "Success — no action needed.",
                        "R20": "Syntax Error — check --help for correct flags.",
                        "R21": "Permission Denied — try with sudo or fix file permissions.",
                        "R22": "Command Not Found — install the missing tool via package manager.",
                        "R23": "Timeout — increase timeout or run in smaller chunks.",
                        "R24": "Subcommand Failure — run diagnostic command to investigate.",
                        "R25": "Environment Mismatch — use POSIX-compatible syntax. rsh will auto-retry with fallback shell.",
                        "R26": "Output Overflow — use rsh_compact to get a compacted view.",
                        "R30": "Fatal/Unknown — requires human escalation."
                    }
                },
                "learned_patterns": pattern_count,
            });
            ToolResponse {
                status: "success".to_string(),
                error: None,
                data: Some(guidance),
                is_error: false,
            }
        }
        "rsh_recover" => {
            let req = match serde_json::from_value::<RecoverRequest>(arguments) {
                Ok(r) => r,
                Err(e) => return ToolResponse {
                    status: "error".to_string(),
                    error: Some(format!("Invalid arguments: {}", e)),
                    data: None,
                    is_error: true,
                },
            };
            let code = parse_recovery_code(&req.recovery_code);
            let detector = Detector::cached().await.clone();
            let suggestion = suggest::suggest(
                code,
                &req.original_command,
                &req.context.unwrap_or_default(),
                &detector,
            );

            // Log telemetry
            {
                let s = state.lock().await;
                let _ = s.store.log_recovery_attempt(
                    &req.recovery_code,
                    &req.original_command,
                    &suggestion.action,
                ).await;
            }

            ToolResponse {
                status: "success".to_string(),
                error: None,
                data: Some(json!(suggestion)),
                is_error: false,
            }
        }
        "rsh_compact" => {
            let req = match serde_json::from_value::<CompactRequest>(arguments) {
                Ok(r) => r,
                Err(e) => return ToolResponse {
                    status: "error".to_string(),
                    error: Some(format!("Invalid arguments: {}", e)),
                    data: None,
                    is_error: true,
                },
            };
            let store = {
                let s = state.lock().await;
                s.store.clone()
            };
            let view = CompactView::parse(req.view.as_deref().unwrap_or("skeleton"));
            if let Some(file_path) = req.file {
                // Validate path and read file atomically to prevent TOCTOU races
                match paths::validate_and_read_file(&file_path) {
                    Ok((_path, content)) => {
                        let compacted = render_view(&content, view, None, None);
                        ToolResponse {
                            status: "success".to_string(),
                            error: None,
                            data: Some(json!(compacted)),
                            is_error: false,
                        }
                    }
                    Err(e) => ToolResponse {
                        status: "error".to_string(),
                        error: Some(format!("File access failed: {}", e)),
                        data: None,
                        is_error: true,
                    },
                }
            } else if let Some(output_id) = req.output_id {
                match store.get_output(&output_id).await {
                    Ok(Some(output)) => {
                        let previous = if matches!(view, CompactView::Diff) {
                            store.previous_output(&output.output_id).await.ok().flatten().map(|previous| previous.stdout)
                        } else {
                            None
                        };
                        let compacted = render_view(&output.stdout, view, previous.as_deref(), Some(output.output_id));
                        ToolResponse {
                            status: "success".to_string(),
                            error: None,
                            data: Some(json!(compacted)),
                            is_error: false,
                        }
                    }
                    Ok(None) => ToolResponse {
                        status: "error".to_string(),
                        error: Some(format!("Unknown output_id: {}", output_id)),
                        data: None,
                        is_error: true,
                    },
                    Err(e) => ToolResponse {
                        status: "error".to_string(),
                        error: Some(format!("Failed to fetch output: {}", e)),
                        data: None,
                        is_error: true,
                    },
                }
            } else {
                ToolResponse {
                    status: "error".to_string(),
                    error: Some("No file or output_id provided for compact".to_string()),
                    data: None,
                    is_error: true,
                }
            }
        }
        _ => ToolResponse {
            status: "error".to_string(),
            error: Some(format!("Unknown tool: {}", name)),
            data: None,
            is_error: true,
        },
    }
}

fn parse_recovery_code(code: &str) -> RecoveryCode {
    match code {
        "R10" => RecoveryCode::R10,
        "R20" => RecoveryCode::R20,
        "R21" => RecoveryCode::R21,
        "R22" => RecoveryCode::R22,
        "R23" => RecoveryCode::R23,
        "R24" => RecoveryCode::R24,
        "R25" => RecoveryCode::R25,
        "R26" => RecoveryCode::R26,
        _ => RecoveryCode::R30,
    }
}

#[derive(Debug, serde::Deserialize)]
struct ExecRequestWrapper {
    command: String,
    cwd: Option<String>,
    timeout: Option<u64>,
    env: Option<HashMap<String, String>>,
    retry: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct RecoverRequest {
    recovery_code: String,
    original_command: String,
    context: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CompactRequest {
    #[serde(rename = "output_id")]
    output_id: Option<String>,
    file: Option<String>,
    #[serde(rename = "view")]
    view: Option<String>,
}
