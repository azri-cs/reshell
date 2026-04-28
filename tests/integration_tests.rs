use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::Command;
use serde_json::{json, Value};

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
    assert!(result["output"]["stdout"].as_str().unwrap().contains("hello"));
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
    assert!(result["suggestion"]["action"].as_str().unwrap().contains("install"));
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
    assert!(result["output"]["stderr"].as_str().unwrap().contains("blocked"));
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

    let stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

    // Send initialize
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
    });
    let mut stdin = stdin;
    stdin.write_all(format!("{}\n", init.to_string()).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let line = lines.next_line().await.unwrap().expect("No response");
    let resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(resp["id"], 1);
    assert!(resp["result"]["serverInfo"]["name"].as_str().unwrap().contains("reshell"));

    // Send initialized notification
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    stdin.write_all(format!("{}\n", note.to_string()).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    // List tools
    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    stdin.write_all(format!("{}\n", list.to_string()).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let line = lines.next_line().await.unwrap().expect("No response");
    let resp: Value = serde_json::from_str(&line).unwrap();
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
    stdin.write_all(format!("{}\n", call.to_string()).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let line = lines.next_line().await.unwrap().expect("No response");
    let resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(resp["id"], 3);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["status"], "success");
    assert!(inner["data"]["output"]["stdout"].as_str().unwrap().contains("mcp_test"));

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

    let stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut stdin = stdin;

    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "test", "version": "1.0" } }
    });
    stdin.write_all(format!("{}\n", init).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
    let _ = lines.next_line().await; // init response

    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    stdin.write_all(format!("{}\n", note).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let call = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "rsh_recover",
            "arguments": { "recovery_code": "R22", "original_command": "gh pr view", "context": "" }
        }
    });
    stdin.write_all(format!("{}\n", call).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let line = lines.next_line().await.unwrap().expect("No response");
    let resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(resp["id"], 4);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["status"], "success");
    assert!(inner["data"]["action"].as_str().unwrap().contains("install"));

    let _ = child.kill().await;
}

#[tokio::test]
async fn test_cli_compact_output_id_and_view() {
    let home = unique_home_dir();

    let exec_output = Command::new(rsh_bin())
        .args(["exec", "--command", "printf 'INFO start\nWARN slow\nERROR failed\n'"])
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
        let second_result: Value = serde_json::from_str(&String::from_utf8_lossy(&second.stdout)).unwrap();
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
        .args(["compact", "--output-id", &second_output_id, "--view", "diff"])
        .env("HOME", &home)
        .output()
        .await
        .expect("Failed diff compact");
    assert!(diff.status.success());
    let diff_result: Value = serde_json::from_str(&String::from_utf8_lossy(&diff.stdout)).unwrap();
    assert_eq!(diff_result["content"], "line3");
}
