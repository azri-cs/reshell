use serde_json::{json, Value};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

fn rsh_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rsh")
}

fn unique_home_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("reshell-test-{}", nanos));
    std::fs::create_dir_all(&path).unwrap();
    path
}

// ── MCP framing helpers ─────────────────────────────────────────────

/// Write an MCP-framed JSON-RPC message to stdin.
async fn write_frame(stdin: &mut tokio::process::ChildStdin, value: &Value) {
    let body = serde_json::to_string(value).unwrap();
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    stdin.write_all(frame.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
}

/// Read a single MCP-framed JSON-RPC response from stdout.
/// Returns None on EOF.
async fn read_frame(reader: &mut BufReader<tokio::process::ChildStdout>) -> Option<Value> {
    // Read header line: "Content-Length: N\r\n"
    let mut header_line = String::new();
    match reader.read_line(&mut header_line).await {
        Ok(0) => return None, // EOF
        Ok(_) => {}
        Err(_) => return None,
    }

    let content_length: usize = header_line
        .trim()
        .strip_prefix("Content-Length:")
        .or_else(|| header_line.trim().strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .expect("Missing or invalid Content-Length header");

    // Read the empty line (\r\n)
    let mut empty_line = String::new();
    reader.read_line(&mut empty_line).await.ok()?;

    // Read exactly content_length bytes for the body
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await.ok()?;

    let body_str = String::from_utf8_lossy(&body);
    serde_json::from_str(&body_str).ok()
}

// ── CLI tests (direct execution, no MCP) ────────────────────────────

#[tokio::test]
async fn test_cli_exec_echo() {
    let home = unique_home_dir();
    let output = Command::new(rsh_bin())
        .args(["exec", "--command", "echo hello"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed to spawn rsh");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert_eq!(result["status"], "success");
    assert_eq!(result["recovery_code"], "R10");
    assert!(result["output"]["stdout"]
        .as_str()
        .unwrap()
        .contains("hello"));
}

#[tokio::test]
async fn test_cli_exec_command_not_found() {
    let home = unique_home_dir();
    let output = Command::new(rsh_bin())
        .args(["exec", "--command", "nonexistent_command_xyz"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed to spawn rsh");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["recovery_code"], "R22");
    assert!(result["suggestion"]["action"]
        .as_str()
        .unwrap()
        .contains("install"));
}

#[tokio::test]
async fn test_cli_exec_blocked_interactive() {
    let home = unique_home_dir();
    let output = Command::new(rsh_bin())
        .args(["exec", "--command", "vim file.txt"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed to spawn rsh");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["recovery_code"], "R27");
    assert!(result["output"]["stderr"]
        .as_str()
        .unwrap()
        .contains("blocked"));
}

#[tokio::test]
async fn test_cli_env() {
    let home = unique_home_dir();
    let output = Command::new(rsh_bin())
        .args(["env"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed to spawn rsh");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    assert!(result["shell"].as_str().is_some());
    assert!(result["path"].as_str().is_some());
}

#[tokio::test]
async fn test_cli_compact_output_id_and_view() {
    let home = unique_home_dir();

    let exec_output = Command::new(rsh_bin())
        .args([
            "exec",
            "--command",
            "printf 'INFO start\nWARN slow\nERROR failed\n'",
        ])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed to spawn rsh");
    assert!(exec_output.status.success());

    let stdout = String::from_utf8_lossy(&exec_output.stdout);
    let result: Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    let output_id = result["output_id"].as_str().unwrap();

    let compact_output = Command::new(rsh_bin())
        .args(["compact", "--output-id", output_id, "--view", "errors_only"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed to compact output id");
    assert!(compact_output.status.success());

    let compact_stdout = String::from_utf8_lossy(&compact_output.stdout);
    let compact_result: Value = serde_json::from_str(&compact_stdout).expect("Invalid JSON");
    assert_eq!(compact_result["view"], "errors_only");
    let content = compact_result["content"].as_str().unwrap();
    assert!(content.contains("WARN slow"));
    assert!(content.contains("ERROR failed"));
}

#[tokio::test]
async fn test_cli_compact_output_id_diff_uses_previous_output() {
    let home = unique_home_dir();

    let first = Command::new(rsh_bin())
        .args(["exec", "--command", "printf 'line1\nline2\n'"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed first exec");
    assert!(first.status.success());
    let second_output_id = {
        let second = Command::new(rsh_bin())
            .args(["exec", "--command", "printf 'line1\nline3\n'"])
            .env("HOME", &home)
            .output()
            .await
            .expect("Failed second exec");
        assert!(second.status.success());
        let second_result: Value =
            serde_json::from_str(&String::from_utf8_lossy(&second.stdout)).unwrap();
        second_result["output_id"].as_str().unwrap().to_string()
    };

    let third = Command::new(rsh_bin())
        .args(["exec", "--command", "printf 'later\noutput\n'"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed third exec");
    assert!(third.status.success());

    let diff = Command::new(rsh_bin())
        .args([
            "compact",
            "--output-id",
            &second_output_id,
            "--view",
            "diff",
        ])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed diff compact");
    assert!(diff.status.success());
    let diff_result: Value = serde_json::from_str(&String::from_utf8_lossy(&diff.stdout)).unwrap();
    assert_eq!(diff_result["content"], "line3");
}

// ── MCP tests (framed stdio transport) ──────────────────────────────

#[tokio::test]
async fn test_mcp_initialize() {
    let home = unique_home_dir();
    let mut child = Command::new(rsh_bin())
        .args(["mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Send initialize
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
    });
    write_frame(&mut stdin, &init).await;

    let resp = read_frame(&mut reader).await.expect("No init response");
    assert_eq!(resp["id"], 1);
    assert!(resp["result"]["serverInfo"]["name"]
        .as_str()
        .unwrap()
        .contains("reshell"));
    assert!(resp["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("rsh_exec"));
    assert!(resp["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("R20"));

    // Send initialized notification
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    write_frame(&mut stdin, &note).await;

    // List tools
    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    write_frame(&mut stdin, &list).await;

    let resp = read_frame(&mut reader)
        .await
        .expect("No tools/list response");
    assert_eq!(resp["id"], 2);
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"rsh_exec"));
    assert!(names.contains(&"rsh_env"));
    assert!(names.contains(&"rsh_recover"));
    assert!(names.contains(&"rsh_compact"));

    // Call rsh_exec
    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "rsh_exec",
            "arguments": { "command": "echo mcp_test" }
        }
    });
    write_frame(&mut stdin, &call).await;

    let resp = read_frame(&mut reader).await.expect("No rsh_exec response");
    assert_eq!(resp["id"], 3);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    // With flattened format, the text is the ExecResult directly
    let inner: Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["status"], "success");
    assert!(inner["output"]["stdout"]
        .as_str()
        .unwrap()
        .contains("mcp_test"));

    // Cleanup
    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_recover_suggestion() {
    let home = unique_home_dir();
    let mut child = Command::new(rsh_bin())
        .args(["mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Initialize
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
    });
    write_frame(&mut stdin, &init).await;
    let _ = read_frame(&mut reader).await; // init response

    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    write_frame(&mut stdin, &note).await;

    // Call rsh_recover with a command that has no alternative
    let call = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "rsh_recover",
            "arguments": { "recovery_code": "R22", "original_command": "nonexistent_tool_xyz arg1", "context": "" }
        }
    });
    write_frame(&mut stdin, &call).await;

    let resp = read_frame(&mut reader)
        .await
        .expect("No rsh_recover response");
    assert_eq!(resp["id"], 4);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(text).unwrap();
    // Flattened format: suggestion fields are at the top level
    assert!(inner["action"].as_str().unwrap().contains("install"));

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_rsh_check() {
    let home = unique_home_dir();
    let mut child = Command::new(rsh_bin())
        .args(["mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Initialize
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
    });
    write_frame(&mut stdin, &init).await;
    let _ = read_frame(&mut reader).await; // init response

    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    write_frame(&mut stdin, &note).await;

    // Call rsh_check
    let call = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "rsh_check",
            "arguments": {}
        }
    });
    write_frame(&mut stdin, &call).await;

    let resp = read_frame(&mut reader)
        .await
        .expect("No rsh_check response");
    assert_eq!(resp["id"], 5);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(text).unwrap();
    // Flattened format: guidance fields are at the top level
    assert_eq!(inner["status"], "healthy");
    assert!(inner["environment"]["os"].as_str().is_some());
    assert!(inner["usage"]["workflow"].as_str().is_some());
    assert!(inner["usage"]["recovery_codes"]["R27"].as_str().is_some());
    // New stats fields
    assert!(inner["learned_patterns"]["total"].as_i64().is_some());
    assert!(inner["learned_patterns"]["with_fixes"].as_i64().is_some());

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_jsonrpc_version_validation() {
    let home = unique_home_dir();
    let mut child = Command::new(rsh_bin())
        .args(["mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Send request with invalid jsonrpc version
    let bad_req = json!({
        "jsonrpc": "1.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    write_frame(&mut stdin, &bad_req).await;

    let resp = read_frame(&mut reader).await.expect("No error response");
    assert_eq!(resp["error"]["code"], -32600);
    assert!(resp["error"]["message"]
        .as_str()
        .unwrap()
        .contains("jsonrpc"));

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_ping() {
    let home = unique_home_dir();
    let mut child = Command::new(rsh_bin())
        .args(["mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Send ping (valid even before initialize)
    let ping = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "ping"
    });
    write_frame(&mut stdin, &ping).await;

    let resp = read_frame(&mut reader).await.expect("No ping response");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"], json!({}));

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_missing_content_length() {
    let home = unique_home_dir();
    let mut child = Command::new(rsh_bin())
        .args(["mcp"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    // Send a frame without Content-Length header
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    // Missing Content-Length — just write raw body (old-style)
    stdin.write_all(body.as_bytes()).await.unwrap();
    stdin.write_all(b"\n").await.unwrap();
    stdin.flush().await.unwrap();

    let resp = read_frame(&mut reader).await.expect("No error response");
    // Should get an error about missing Content-Length or parse error
    assert!(resp["error"].is_object());
    let msg = resp["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("Content-Length") || msg.contains("parse") || msg.contains("header"),
        "unexpected error message: {msg}"
    );

    let _ = child.kill().await;
}
