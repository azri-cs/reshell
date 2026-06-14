use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;

/// SSE server for MCP transport. Runs an HTTP server that accepts
/// SSE connections at GET /mcp/sse and JSON-RPC messages at POST /mcp/messages.
pub struct SseServer {
    addr: SocketAddr,
}

impl SseServer {
    /// Start the SSE server on the given address. Returns immediately
    /// after spawning the server in a background tokio task.
    pub async fn start(addr: SocketAddr) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound_addr = listener.local_addr()?;

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };
                let io = TokioIo::new(stream);

                tokio::spawn(async move {
                    let svc = service_fn(handle_sse_request);

                    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                        eprintln!("SSE connection error: {}", e);
                    }
                });
            }
        });

        Ok(Self { addr: bound_addr })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn handle_sse_request(req: Request<Incoming>) -> Result<Response<String>, hyper::Error> {
    match (req.method().as_str(), req.uri().path()) {
        ("GET", "/mcp/sse") => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Connection", "keep-alive")
            .body(
                "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\n"
                    .to_string(),
            )
            .unwrap()),
        ("POST", "/mcp/messages") => Ok(Response::new(
            r#"{"jsonrpc":"2.0","result":"ok"}"#.to_string(),
        )),
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("Not Found".to_string())
            .unwrap()),
    }
}
