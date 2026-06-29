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
    std::fs::create_dir_all(path.join(".reshell")).unwrap();
    path
}

/// Isolate pattern DB and home dir per test (Windows uses USERPROFILE, not HOME).
fn apply_test_env(cmd: &mut Command, home: &std::path::Path) {
    let db_path = home.join(".reshell").join("patterns.db");
    cmd.env("HOME", home);
    cmd.env("USERPROFILE", home);
    cmd.env("RSH_PATTERN_DB", db_path);
}

fn rsh_command(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(rsh_bin());
    apply_test_env(&mut cmd, home);
    cmd
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
    let output = rsh_command(&home)
        .args(["exec", "--command", "echo hello"])
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
    let output = rsh_command(&home)
        .args(["exec", "--command", "nonexistent_command_xyz"])
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
    let output = rsh_command(&home)
        .args(["exec", "--command", "vim file.txt"])
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
    let output = rsh_command(&home)
        .args(["env"])
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

    let exec_output = rsh_command(&home)
        .args([
            "exec",
            "--command",
            "printf 'INFO start\nWARN slow\nERROR failed\n' 2>&1 | cat",
        ])
        .output()
        .await
        .expect("Failed to spawn rsh");
    assert!(exec_output.status.success());

    let stdout = String::from_utf8_lossy(&exec_output.stdout);
    let result: Value = serde_json::from_str(&stdout).expect("Invalid JSON");
    let output_id = result["output_id"].as_str().unwrap();

    let compact_output = rsh_command(&home)
        .args(["compact", "--output-id", output_id, "--view", "errors_only"])
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

    let first = rsh_command(&home)
        .args(["exec", "--command", "printf '%s\n' line1 line2 | cat"])
        .output()
        .await
        .expect("Failed first exec");
    assert!(
        first.status.success(),
        "first exec failed: status={} stdout={:?} stderr={:?}",
        first.status,
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let second_output_id = {
        let second = rsh_command(&home)
            .args(["exec", "--command", "printf '%s\n' line1 line3 | cat"])
            .output()
            .await
            .expect("Failed second exec");
        assert!(
            second.status.success(),
            "second exec failed: status={} stdout={:?} stderr={:?}",
            second.status,
            String::from_utf8_lossy(&second.stdout),
            String::from_utf8_lossy(&second.stderr)
        );
        let second_result: Value =
            serde_json::from_str(&String::from_utf8_lossy(&second.stdout)).unwrap();
        second_result["output_id"].as_str().unwrap().to_string()
    };

    let third = rsh_command(&home)
        .args(["exec", "--command", "printf '%s\n' later output | cat"])
        .output()
        .await
        .expect("Failed third exec");
    assert!(
        third.status.success(),
        "third exec failed: status={} stdout={:?} stderr={:?}",
        third.status,
        String::from_utf8_lossy(&third.stdout),
        String::from_utf8_lossy(&third.stderr)
    );

    let diff = rsh_command(&home)
        .args([
            "compact",
            "--output-id",
            &second_output_id,
            "--view",
            "diff",
        ])
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
    let mut child = rsh_command(&home)
        .args(["mcp"])
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
    let mut child = rsh_command(&home)
        .args(["mcp"])
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
    let mut child = rsh_command(&home)
        .args(["mcp"])
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
    let mut child = rsh_command(&home)
        .args(["mcp"])
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
    let mut child = rsh_command(&home)
        .args(["mcp"])
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
    let mut child = rsh_command(&home)
        .args(["mcp"])
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

/// Helper: spawn an MCP server, initialize, return (child, stdin, reader).
async fn spawn_mcp_server(
    home: &std::path::Path,
) -> (
    tokio::process::Child,
    tokio::process::ChildStdin,
    BufReader<tokio::process::ChildStdout>,
) {
    let mut child = rsh_command(home)
        .args(["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn MCP server");
    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let mut reader = BufReader::new(stdout);

    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}});
    write_frame(&mut stdin, &init).await;
    let _ = read_frame(&mut reader).await;
    let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    write_frame(&mut stdin, &note).await;

    (child, stdin, reader)
}

#[tokio::test]
async fn test_mcp_feedback_and_stats() {
    let home = unique_home_dir();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;

    // Execute a failing command and capture its stderr for feedback
    let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"nonexistent_feedback_test_cmd_xyz"}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let exec_result: Value = serde_json::from_str(text).unwrap();
    let stderr = exec_result["output"]["stderr"].as_str().unwrap_or("");

    // Send feedback with the stderr from the original failure
    let fb = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rsh_feedback","arguments":{"original_command":"nonexistent_feedback_test_cmd_xyz","fix_command":"echo fixed","success":true,"stderr":stderr}}});
    write_frame(&mut stdin, &fb).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let fb_result: Value = serde_json::from_str(text).unwrap();
    assert_eq!(fb_result["status"], "recorded");

    // Verify stats reflect the learning
    let stats = json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"rsh_stats","arguments":{}}});
    write_frame(&mut stdin, &stats).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let st: Value = serde_json::from_str(text).unwrap();
    assert!(
        st["patterns"]["total"].as_i64().unwrap_or(0) > 0,
        "stats should show at least 1 pattern"
    );
    assert!(
        st["patterns"]["by_recovery_code"].is_array()
            || st["patterns"]["by_recovery_code"].is_object(),
        "by_recovery_code should be present"
    );

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_audit_session_id_and_recent_invocations() {
    // One home dir shared across two server processes — the audit rows they
    // write must carry distinct session ids (one per process) so calls are
    // traceable to the server that made them.
    let home = unique_home_dir();

    // First server: run an exec, then read rsh_stats to get the session id + recent slice.
    let (mut child1, mut stdin1, mut reader1) = spawn_mcp_server(&home).await;
    let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"echo audit_session_one"}}});
    write_frame(&mut stdin1, &exec).await;
    let _ = read_frame(&mut reader1).await.unwrap();

    let stats = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rsh_stats","arguments":{}}});
    write_frame(&mut stdin1, &stats).await;
    let resp = read_frame(&mut reader1).await.unwrap();
    let st: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let recent = st["recent_invocations"]
        .as_array()
        .expect("recent_invocations present");
    assert!(
        !recent.is_empty(),
        "recent_invocations should include the exec"
    );
    let session1 = recent
        .iter()
        .find(|e| e["command"].as_str() == Some("echo audit_session_one"))
        .map(|e| e["session_id"].as_str().unwrap().to_string())
        .expect("session1 exec should appear in recent invocations");
    assert!(!session1.is_empty());
    // Every exec row is tagged rsh_exec / R10.
    assert!(recent.iter().any(|e| e["tool"] == "rsh_exec"));
    assert!(recent.iter().any(|e| e["recovery_code"] == "R10"));
    let _ = child1.kill().await;

    // Second server process at the same home — must get a different session id.
    let (mut child2, mut stdin2, mut reader2) = spawn_mcp_server(&home).await;
    let exec2 = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"echo audit_session_two"}}});
    write_frame(&mut stdin2, &exec2).await;
    let _ = read_frame(&mut reader2).await.unwrap();

    let stats2 = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rsh_stats","arguments":{}}});
    write_frame(&mut stdin2, &stats2).await;
    let resp2 = read_frame(&mut reader2).await.unwrap();
    let st2: Value =
        serde_json::from_str(resp2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let recent2 = st2["recent_invocations"].as_array().unwrap();
    // Find the row belonging to session2's exec by its command (the shared DB
    // also contains session1's rows, and timestamp ties make index order
    // unreliable within the same second).
    let session2 = recent2
        .iter()
        .find(|e| e["command"].as_str() == Some("echo audit_session_two"))
        .map(|e| e["session_id"].as_str().unwrap().to_string())
        .expect("session2 exec should appear in recent invocations");
    assert_ne!(
        session1, session2,
        "each server process gets a unique session id"
    );
    let _ = child2.kill().await;
}

#[tokio::test]
async fn test_mcp_budget_session_cap_refuses_call() {
    // Configure a 2-call session cap and verify the 3rd call returns R29.
    let home = unique_home_dir();
    let cfg_dir = home.join(".reshell");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.toml"),
        "[budget]\nmax_invocations_per_session = 2\n",
    )
    .unwrap();

    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;

    // Two calls succeed.
    for _ in 0..2 {
        let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"echo ok"}}});
        write_frame(&mut stdin, &exec).await;
        let resp = read_frame(&mut reader).await.unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let r: Value = serde_json::from_str(text).unwrap();
        assert_eq!(r["status"], "success", "first two calls should succeed");
    }

    // Third call is refused with R29 (Budget Exhausted).
    let exec = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"echo blocked"}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let r: Value = serde_json::from_str(text).unwrap();
    assert_eq!(r["status"], "failed");
    assert_eq!(r["recovery_code"], "R29");
    assert!(r["suggestion"]["reason"]
        .as_str()
        .unwrap()
        .contains("max_invocations_per_session"));
    // The refused call must not have executed the command.
    assert!(r["output"]["stdout"].as_str().unwrap().is_empty());

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_budget_unlimited_by_default() {
    // With no [budget] config, no cap applies — many calls all succeed.
    let home = unique_home_dir();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;
    for _ in 0..15 {
        let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"echo ok"}}});
        write_frame(&mut stdin, &exec).await;
        let resp = read_frame(&mut reader).await.unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let r: Value = serde_json::from_str(text).unwrap();
        assert_eq!(r["status"], "success");
    }
    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_high_risk_command_needs_approval_then_runs() {
    // A high-risk command (git push --force) returns R28 without approve:true.
    // With approve:true it is allowed past the gate (it then fails as a normal
    // git error, not R28 — proving approval worked).
    let home = unique_home_dir();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;

    // 1. Without approval → R28, status needs_approval, command not executed.
    let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"git push --force origin main"}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let r: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(r["status"], "needs_approval");
    assert_eq!(r["recovery_code"], "R28");
    assert_eq!(r["recovery_class"], "Approval Required");
    assert_eq!(r["next_action"]["params"]["approve"], true);
    assert!(r["output"]["stdout"].as_str().unwrap().is_empty());

    // 2. With approve:true → past the gate. It executes and fails as a normal
    //    git error (not a git repo), so recovery_code must NOT be R28.
    let exec = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"git push --force origin main", "approve": true}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let r2: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_ne!(
        r2["recovery_code"], "R28",
        "approve:true must bypass the R28 gate"
    );
    assert_ne!(r2["status"], "needs_approval");

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_sudo_triggers_review() {
    // sudo-bearing commands are flagged for review (R28) by default.
    let home = unique_home_dir();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;
    let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"sudo ls /root"}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let r: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(r["status"], "needs_approval");
    assert_eq!(r["recovery_code"], "R28");
    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_safety_auto_approve_skips_review() {
    // With [safety] auto_approve = true, high-risk commands run without R28.
    let home = unique_home_dir();
    let cfg_dir = home.join(".reshell");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.toml"),
        "[safety]\nauto_approve = true\n",
    )
    .unwrap();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;
    // sudo echo is auto-approved, then executes. It may fail on password, but
    // it must NOT return R28 (the gate is skipped entirely).
    let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"sudo echo hi"}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let r: Value =
        serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_ne!(
        r["recovery_code"], "R28",
        "auto_approve must skip the R28 gate"
    );
    assert_ne!(r["status"], "needs_approval");
    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_resources_list_and_read() {
    let home = unique_home_dir();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;

    // Execute a command to create stored output
    let exec = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rsh_exec","arguments":{"command":"echo resource_test_output_42 && true"}}});
    write_frame(&mut stdin, &exec).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(text).unwrap();
    let output_id = inner["output_id"].as_str().unwrap().to_string();

    // List resources
    let list = json!({"jsonrpc":"2.0","id":3,"method":"resources/list"});
    write_frame(&mut stdin, &list).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let resources = resp["result"]["resources"].as_array().unwrap();
    assert!(
        !resources.is_empty(),
        "Resources should not be empty after exec"
    );
    let found = resources
        .iter()
        .any(|r| r["uri"].as_str().unwrap().contains(&output_id));
    assert!(found, "Resource should include the output from exec");

    // Read the specific resource
    let read = json!({"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri": format!("reshell://output/{}", output_id)}});
    write_frame(&mut stdin, &read).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let contents = resp["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(contents.contains("resource_test_output_42"));

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_mcp_prompts_list_and_get() {
    let home = unique_home_dir();
    let (mut child, mut stdin, mut reader) = spawn_mcp_server(&home).await;

    // List prompts
    let list = json!({"jsonrpc":"2.0","id":2,"method":"prompts/list"});
    write_frame(&mut stdin, &list).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let prompts = resp["result"]["prompts"].as_array().unwrap();
    let names: Vec<&str> = prompts
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"recovery_analysis"),
        "Should have recovery_analysis prompt"
    );
    assert!(
        names.contains(&"environment_audit"),
        "Should have environment_audit prompt"
    );

    // Get recovery_analysis prompt
    let get = json!({"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{"name":"recovery_analysis","arguments":{"recovery_code":"R22","original_command":"gh pr view"}}});
    write_frame(&mut stdin, &get).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let messages = resp["result"]["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    let text = messages[0]["content"]["text"].as_str().unwrap();
    assert!(
        text.contains("recovery_code"),
        "Prompt should discuss failure analysis"
    );

    // Get environment_audit prompt
    let get = json!({"jsonrpc":"2.0","id":4,"method":"prompts/get","params":{"name":"environment_audit"}});
    write_frame(&mut stdin, &get).await;
    let resp = read_frame(&mut reader).await.unwrap();
    let messages = resp["result"]["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    let text = messages[0]["content"]["text"].as_str().unwrap();
    assert!(
        text.contains("rsh_env") || text.contains("environment"),
        "Prompt should reference environment detection"
    );

    let _ = child.kill().await;
}

#[test]
fn test_cli_version_flag() {
    let output = std::process::Command::new(rsh_bin())
        .arg("--version")
        .output()
        .expect("failed to execute rsh");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("rsh"),
        "version output should contain 'rsh': {}",
        stdout
    );
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "should contain version: {}",
        stdout
    );
}
