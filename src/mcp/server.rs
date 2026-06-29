use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::router::{JsonRpcRequest, Router};
use crate::memory::metrics::Metrics;
use crate::memory::Store;

/// Maximum allowed size for a single incoming JSON-RPC message body (1 MB).
const MAX_REQUEST_SIZE: usize = 1024 * 1024;

/// Maximum header section size before we stop looking for Content-Length.
const MAX_HEADER_BYTES: usize = 4096;

pub struct McpServer {
    router: Arc<Router>,
}

impl McpServer {
    /// Open the pattern store and start server state. Fails if `~/.reshell` or the DB cannot be opened.
    pub fn new() -> anyhow::Result<Self> {
        // Suppress stderr warnings in MCP mode so they don't interleave with JSON-RPC frames.
        crate::config::suppress_stderr_warnings();
        let store = Store::new()?;
        Ok(Self {
            router: Arc::new(Router::new(store, Arc::new(Metrics::new()))),
        })
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut stdout = io::stdout();

        loop {
            match Self::read_frame(&mut reader).await {
                Ok(Some(body)) => {
                    if body.trim().is_empty() {
                        // Empty frame — skip, don't error
                        continue;
                    }
                    if body.len() > MAX_REQUEST_SIZE {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": null,
                            "error": { "code": -32600, "message": format!("Request too large: {} bytes (max {})", body.len(), MAX_REQUEST_SIZE) }
                        });
                        Self::write_frame(&mut stdout, &resp).await?;
                        continue;
                    }
                    let req: JsonRpcRequest = match serde_json::from_str(&body) {
                        Ok(r) => r,
                        Err(e) => {
                            let detail = e.to_string();
                            let detail = if detail.len() > 240 {
                                format!("{}…", &detail[..240])
                            } else {
                                detail
                            };
                            let resp = json!({
                                "jsonrpc": "2.0",
                                "id": null,
                                "error": { "code": -32700, "message": format!("Parse error: invalid JSON-RPC request ({})", detail) }
                            });
                            Self::write_frame(&mut stdout, &resp).await?;
                            continue;
                        }
                    };

                    // Validate JSON-RPC version
                    if req._jsonrpc != "2.0" {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req.id,
                            "error": { "code": -32600, "message": "Invalid Request: jsonrpc must be \"2.0\"" }
                        });
                        Self::write_frame(&mut stdout, &resp).await?;
                        continue;
                    }

                    if let Some(resp) = self.router.handle(req).await {
                        Self::write_frame(&mut stdout, &resp).await?;
                    }
                }
                Ok(None) => {
                    // EOF — clean shutdown
                    break;
                }
                Err(e) => {
                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": { "code": -32600, "message": format!("Frame read error: {}", e) }
                    });
                    Self::write_frame(&mut stdout, &resp).await?;
                    // Continue reading — don't break on a single bad frame
                }
            }
        }
        Ok(())
    }

    /// Read a single MCP-framed message from stdin.
    /// Returns `Ok(Some(body))` for a valid frame, `Ok(None)` on EOF,
    /// `Err(...)` for framing errors (bad header, truncated body, etc.).
    async fn read_frame(reader: &mut BufReader<io::Stdin>) -> anyhow::Result<Option<String>> {
        let mut header_buf = Vec::with_capacity(MAX_HEADER_BYTES);
        let mut content_length: Option<usize> = None;
        let mut header_lines: usize = 0;

        // Read header lines until \r\n\r\n or \n\n
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                // EOF — if we already have a partial frame, that's an error
                if content_length.is_some() || header_lines > 0 {
                    return Err(anyhow::anyhow!("EOF while reading frame headers"));
                }
                return Ok(None);
            }

            header_lines += 1;

            // Check for header overflow (too many lines or too many bytes)
            if header_lines > 32 || header_buf.len() + line.len() > MAX_HEADER_BYTES {
                return Err(anyhow::anyhow!(
                    "Frame headers exceed limits ({} lines, {} bytes)",
                    header_lines,
                    header_buf.len()
                ));
            }

            header_buf.extend_from_slice(line.as_bytes());

            // End of headers: empty line (\r\n or \n)
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }

            // Parse Content-Length header (case-insensitive key)
            if let Some(value) = trimmed
                .strip_prefix("Content-Length:")
                .or_else(|| trimmed.strip_prefix("content-length:"))
            {
                let value = value.trim();
                content_length = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| anyhow::anyhow!("Invalid Content-Length: {}", value))?,
                );
            } else if header_lines == 1 && trimmed.starts_with('{') {
                // First line looks like raw JSON (no Content-Length header).
                // This happens with misconfigured clients sending old-style
                // newline-delimited JSON instead of MCP-framed messages.
                return Err(anyhow::anyhow!(
                    "Missing Content-Length header (received raw JSON instead of framed message)"
                ));
            }
            // Unknown headers are tolerated (future extensibility).
        }

        let content_length = content_length
            .ok_or_else(|| anyhow::anyhow!("Missing Content-Length header in frame"))?;

        if content_length > MAX_REQUEST_SIZE {
            return Err(anyhow::anyhow!(
                "Content-Length {} exceeds maximum {}",
                content_length,
                MAX_REQUEST_SIZE
            ));
        }

        // Read exactly content_length bytes for the body
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).await?;

        let body_str = String::from_utf8(body)
            .map_err(|e| anyhow::anyhow!("Frame body is not valid UTF-8: {}", e))?;

        Ok(Some(body_str))
    }

    /// Write an MCP-framed response to stdout.
    async fn write_frame(stdout: &mut io::Stdout, value: &Value) -> anyhow::Result<()> {
        let text = serde_json::to_string(value)?;
        let header = format!("Content-Length: {}\r\n\r\n", text.len());
        stdout.write_all(header.as_bytes()).await?;
        stdout.write_all(text.as_bytes()).await?;
        stdout.flush().await?;
        Ok(())
    }
}
