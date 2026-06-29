use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::tools::list_tools;
use crate::budget::BudgetCounters;
use crate::memory::metrics::Metrics;
use crate::memory::Store;

pub struct ServerState {
    pub store: Store,
    pub metrics: Arc<Metrics>,
    /// One UUID per server process — the cross-request correlation id stamped
    /// onto every audit row so calls are traceable end-to-end.
    pub session_id: String,
    /// In-process session budget counters (invocations / bytes / wall time).
    /// Lives here (not in stdio-only `McpServer`) so both stdio and SSE
    /// transports enforce the same budget guardrail.
    pub budget: BudgetCounters,
}

/// Per-request context threaded through every tool call so audit rows carry
/// the session id (one per server process) and the JSON-RPC request id
/// (per call). Built in `Router::handle` from the shared state + request id.
#[derive(Debug, Clone)]
pub struct CallContext {
    pub session_id: String,
    pub request_id: Option<String>,
}

impl CallContext {
    /// Serialize the request id to a plain string for the audit row.
    pub fn request_id_str(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

/// Shared JSON-RPC router used by both stdio and SSE MCP transports.
pub struct Router {
    state: Arc<Mutex<ServerState>>,
}

impl Router {
    pub fn new(store: Store, metrics: Arc<Metrics>) -> Self {
        let session_id = uuid::Uuid::new_v4().to_string();
        Self {
            state: Arc::new(Mutex::new(ServerState {
                store,
                metrics,
                session_id,
                budget: BudgetCounters::default(),
            })),
        }
    }

    pub fn state(&self) -> Arc<Mutex<ServerState>> {
        self.state.clone()
    }

    pub async fn handle(&self, req: JsonRpcRequest) -> Option<Value> {
        let id = req.id.clone();

        match req.method.as_str() {
            "ping" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            })),
            "initialize" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "reshell",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": "Reshell wraps shell commands with self-healing execution. **Use `rsh_exec` INSTEAD of raw bash for EVERY command** — never use both.\n\n## Why rsh_exec beats raw bash\n- **Secrets are auto-redacted** — API keys, tokens, and passwords in output are scrubbed\n- **Failing commands self-heal** — missing tools are auto-installed (check `auto_retry`), wrong syntax gets a fix suggestion\n- **Pattern memory learns** — one call to `rsh_feedback` after a fix makes the same failure auto-resolve next time\n- **Output is compacted** — large outputs are truncated with a skeleton view available via `rsh_compact`\n- **Safety sandbox blocks dangerous commands** — interactive shells, destructive pipes, and unsafe paths are rejected\n\n## Complete tool set\n| Tool | When to use |\n|------|-------------|\n| `rsh_exec` | **ALL shell commands** — installs, builds, file ops, git, etc. |\n| `rsh_recover` | Only if `rsh_exec` failed AND `auto_retry` was absent or failed |\n| `rsh_feedback` | **CRITICAL** — call after every fix attempt so pattern memory improves |\n| `rsh_compact` | When `rsh_exec` returns `truncated=true` — gets skeleton/diff view |\n| `rsh_read_file` | Read files through safety sandbox (blocks traversal, sensitive paths) |\n| `rsh_write_file` | Write files through safety sandbox (create dirs, block sensitive paths) |\n| `rsh_env` | Detect OS, shell, available tools, package manager |\n| `rsh_check` | Health check and workflow guidance — call at session start |\n| `rsh_stats` | View learned patterns and their success rates |\n\n## Full example workflow\n```\nrsh_exec \"gh pr view\"\n→ fails: status=\"failed\", recovery_code=\"R22\", suggestion.command=\"brew install gh\"\n→ auto_retry.status=\"success\" (brew install gh was auto-executed)\n→ rsh_feedback { original_command: \"gh pr view\", fix_command: \"brew install gh\", success: true }\n→ NEXT TIME: rsh_exec \"gh pr view\" auto-recovers instantly (no round-trips needed)\n```\n\n## Recovery codes\n- **R10** Success — no action needed\n- **R20** Syntax Error — check --help\n- **R21** Permission Denied — try sudo\n- **R22** Command Not Found — install the missing tool via package manager\n- **R23** Timeout — increase timeout\n- **R24** Subcommand Failure — run diagnostic\n- **R25** Environment Mismatch — use POSIX-compatible syntax. rsh will auto-retry with fallback shell\n- **R26** Output Overflow — use rsh_compact\n- **R27** Blocked / Safety Violation — command was blocked by the safety validator\n- **R28** Approval Required — high-risk command; re-issue with approve:true after a human approves\n- **R29** Budget Exhausted — a configured call/byte/time cap was hit; wait for the window to reset\n- **R30** Fatal / Unknown — requires human escalation"
                }
            })),
            "notifications/initialized" => None,
            "tools/list" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": list_tools() }
            })),
            "tools/call" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));
                // Build a per-call context: the session id (one per server
                // process, the cross-request correlation id) plus this
                // request's JSON-RPC id, so audit rows are traceable.
                let (session_id, call_ctx) = {
                    let s = self.state.lock().await;
                    let request_id = match &id {
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(Value::Number(n)) => Some(n.to_string()),
                        _ => None,
                    };
                    (
                        s.session_id.clone(),
                        CallContext {
                            session_id: s.session_id.clone(),
                            request_id,
                        },
                    )
                };
                let _ = session_id; // retained for clarity; call_ctx carries it
                let (data, is_error) =
                    super::tools::handle_tool_call(name, arguments, &self.state, call_ctx).await;
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [
                            { "type": "text", "text": serde_json::to_string(&data).unwrap_or_default() }
                        ],
                        "isError": is_error
                    }
                }))
            }
            "resources/list" => {
                let store = {
                    let s = self.state.lock().await;
                    s.store.clone()
                };
                let outputs = store.list_recent_outputs(50).await.unwrap_or_default();
                let resources: Vec<Value> = outputs
                    .into_iter()
                    .map(|o| {
                        let label = if o.original_command.len() > 60 {
                            format!("{}…", &o.original_command[..57])
                        } else {
                            o.original_command.clone()
                        };
                        json!({
                            "uri": format!("reshell://output/{}", o.output_id),
                            "name": label,
                            "description": format!("Command output (exit {})", o.exit_code),
                            "mimeType": "text/plain"
                        })
                    })
                    .collect();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "resources": resources }
                }))
            }
            "resources/read" => {
                let uri = req.params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                let output_id = uri.trim_start_matches("reshell://output/");
                let store = {
                    let s = self.state.lock().await;
                    s.store.clone()
                };
                match store.get_output(output_id).await {
                    Ok(Some(output)) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "contents": [
                                { "uri": uri, "mimeType": "text/plain", "text": output.stdout }
                            ]
                        }
                    })),
                    Ok(None) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": format!("Unknown resource: {}", uri) }
                    })),
                    Err(e) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32603, "message": format!("Internal error: {}", e) }
                    })),
                }
            }
            "prompts/list" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "prompts": super::prompts::list_prompts() }
            })),
            "prompts/get" => {
                let name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = req.params.get("arguments");
                match super::prompts::get_prompt(name, arguments) {
                    Some(content) => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "description": format!("Prompt: {}", name),
                            "messages": [
                                { "role": "user", "content": { "type": "text", "text": content } }
                            ]
                        }
                    })),
                    None => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": format!("Unknown prompt: {}", name) }
                    })),
                }
            }
            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {}", req.method) }
            })),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    pub _jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub id: Option<Value>,
}
