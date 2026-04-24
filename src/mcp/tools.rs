use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;

use crate::exec::{ExecRequest, runner::Runner};
use crate::env::Detector;
use crate::classify::taxonomy::RecoveryCode;
use crate::recover::suggest;
use crate::compact::view::{CompactView, render_view};

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
            "description": "Execute a shell command with resilient failure handling",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The shell command to execute" },
                    "cwd": { "type": "string", "description": "Working directory" },
                    "timeout": { "type": "integer", "default": 120 },
                    "env": { "type": "object", "additionalProperties": { "type": "string" } },
                    "retry": { "type": "boolean", "default": true }
                },
                "required": ["command"]
            }
        }),
        json!({
            "name": "rsh_env",
            "description": "Detect and describe the current shell environment",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "rsh_recover",
            "description": "Apply a deterministic recovery strategy for a known failure",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recovery_code": { "type": "string" },
                    "original_command": { "type": "string" },
                    "context": { "type": "string" }
                },
                "required": ["recovery_code", "original_command"]
            }
        }),
        json!({
            "name": "rsh_compact",
            "description": "Retrieve a compacted view of a previously stored large output or a file",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "output_id": { "type": "string" },
                    "file": { "type": "string" },
                    "view": { "type": "string", "enum": ["full", "skeleton", "diff", "errors_only"] }
                }
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
                timeout: wrapper.timeout.unwrap_or(120),
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
                match tokio::fs::read_to_string(&file_path).await {
                    Ok(content) => {
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
                        error: Some(format!("Failed to read file: {}", e)),
                        data: None,
                        is_error: true,
                    },
                }
            } else if let Some(output_id) = req.output_id {
                match store.get_output(&output_id) {
                    Ok(Some(output)) => {
                        let previous = if matches!(view, CompactView::Diff) {
                            store.previous_output(&output.output_id).ok().flatten().map(|previous| previous.stdout)
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
