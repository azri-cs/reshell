use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::tools::{list_tools, handle_tool_call};
use crate::memory::Store;

pub struct McpServer {
    state: Arc<Mutex<ServerState>>,
}

pub(crate) struct ServerState {
    #[allow(dead_code)]
    pub store: Store,
}

impl McpServer {
    pub fn new() -> Self {
        let store = Store::new().expect("Failed to open pattern DB");
        Self {
            state: Arc::new(Mutex::new(ServerState { store })),
        }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let stdin = io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        let mut stdout = io::stdout();

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                    });
                    Self::write_line(&mut stdout, &resp).await?;
                    continue;
                }
            };

            if let Some(resp) = self.handle_request(req).await {
                Self::write_line(&mut stdout, &resp).await?;
            }
        }
        Ok(())
    }

    async fn write_line(stdout: &mut io::Stdout, value: &Value) -> anyhow::Result<()> {
        let text = serde_json::to_string(value)?;
        stdout.write_all(text.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
        Ok(())
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> Option<Value> {
        let id = req.id.clone();

        if req.method == "initialize" {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "reshell",
                        "version": "0.1.0"
                    }
                }
            }));
        }

        if req.method == "notifications/initialized" {
            return None;
        }

        if req.method == "tools/list" {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": list_tools() }
            }));
        }

        if req.method == "tools/call" {
            let name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let result = handle_tool_call(name, arguments, &self.state).await;
            let is_error = result.is_error;
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [
                        { "type": "text", "text": serde_json::to_string(&result).unwrap_or_default() }
                    ],
                    "isError": is_error
                }
            }));
        }

        Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("Method not found: {}", req.method) }
        }))
    }
}

#[derive(Debug, serde::Deserialize)]
struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    _jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Value,
    id: Option<Value>,
}
