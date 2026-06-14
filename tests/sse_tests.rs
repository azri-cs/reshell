use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reshell::memory::metrics::Metrics;
use reshell::memory::Store;
use reshell::mcp::Router;

fn unique_home_dir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join("reshell-sse-tests");
    let id = uuid::Uuid::new_v4().to_string();
    let home = base.join(id);
    std::fs::create_dir_all(&home).unwrap();
    home
}

#[tokio::test]
async fn test_sse_tools_list() {
    let home = unique_home_dir();
    std::env::set_var("HOME", &home);

    let store = Store::new().unwrap();
    let metrics = Arc::new(Metrics::new());
    let router = Arc::new(Router::new(store, metrics));

    let addr: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
    let server = reshell::mcp::sse::SseServer::start(addr, router).await.unwrap();
    let local_addr = server.local_addr();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // Open SSE stream and read the endpoint event.
    let resp = client
        .get(format!("http://{}/mcp/sse", local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut endpoint: Option<String> = None;

    while let Ok(Some(chunk)) = tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        let chunk = chunk.unwrap();
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        if let Some(line) = buffer.lines().find(|l| l.starts_with("data: ")) {
            endpoint = Some(line.strip_prefix("data: ").unwrap().to_string());
            break;
        }
    }

    let endpoint = endpoint.expect("SSE should emit an endpoint event");
    assert!(endpoint.contains("/mcp/messages?session_id="));

    let session_id = endpoint.split("session_id=").nth(1).unwrap();

    // Send tools/list request via POST.
    let list_resp = client
        .post(format!(
            "http://{}/mcp/messages?session_id={}",
            local_addr, session_id
        ))
        .body(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 202);
}
