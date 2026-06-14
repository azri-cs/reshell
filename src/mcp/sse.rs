use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::{Body, Frame};
use serde_json::json;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;

use super::router::{JsonRpcRequest, Router};

/// SSE server for MCP transport. Runs an HTTP server that accepts
/// SSE connections at `GET /mcp/sse` and JSON-RPC messages at `POST /mcp/messages`.
pub struct SseServer {
    addr: SocketAddr,
    _handle: JoinHandle<()>,
}

struct Session {
    tx: mpsc::Sender<String>,
}

#[derive(Clone)]
struct SseState {
    router: Arc<Router>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl SseServer {
    /// Start the SSE server on the given address. Returns immediately
    /// after spawning the server in a background tokio task.
    pub async fn start(addr: SocketAddr, router: Arc<Router>) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let bound_addr = listener.local_addr()?;

        let state = SseState {
            router,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        };

        let handle = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };
                let io = TokioIo::new(stream);
                let state = state.clone();

                tokio::spawn(async move {
                    let svc = service_fn(move |req| handle_sse_request(req, state.clone()));

                    if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                        eprintln!("SSE connection error: {}", e);
                    }
                });
            }
        });

        Ok(Self {
            addr: bound_addr,
            _handle: handle,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn handle_sse_request(
    req: Request<Incoming>,
    state: SseState,
) -> Result<Response<SseBody>, hyper::Error> {
    match (req.method().as_str(), req.uri().path()) {
        ("GET", "/mcp/sse") => handle_sse_connect(state).await,
        ("POST", "/mcp/messages") => handle_sse_message(req, state).await,
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(SseBody::empty())
            .unwrap()),
    }
}

async fn handle_sse_connect(state: SseState) -> Result<Response<SseBody>, hyper::Error> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<String>(64);

    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(session_id.clone(), Session { tx: tx.clone() });
    }

    let endpoint = format!("/mcp/messages?session_id={}", session_id);

    // Seed initial events before returning the streaming body.
    let _ = tx
        .send(format!("event: endpoint\ndata: {}\n\n", endpoint))
        .await;
    let _ = tx
        .send("data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\n".to_string())
        .await;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(SseBody::new(rx))
        .unwrap())
}

async fn handle_sse_message(
    mut req: Request<Incoming>,
    state: SseState,
) -> Result<Response<SseBody>, hyper::Error> {
    let session_id = req
        .uri()
        .query()
        .and_then(|q| {
            q.split('&')
                .find_map(|pair| pair.strip_prefix("session_id="))
        })
        .unwrap_or("")
        .to_string();

    let body_bytes = match http_body_util::BodyExt::collect(req.body_mut()).await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": format!("Failed to read body: {}", e) } }),
            );
        }
    };

    let body_str = match String::from_utf8(body_bytes.to_vec()) {
        Ok(s) => s,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": format!("Body is not UTF-8: {}", e) } }),
            );
        }
    };

    let rpc_req: JsonRpcRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": format!("Parse error: {}", e) } }),
            );
        }
    };

    if rpc_req._jsonrpc != "2.0" {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "jsonrpc": "2.0", "id": rpc_req.id, "error": { "code": -32600, "message": "Invalid Request: jsonrpc must be \"2.0\"" } }),
        );
    }

    let response = state.router.handle(rpc_req).await;

    // If there's a session, push the response as an SSE event.
    if !session_id.is_empty() {
        if let Some(resp) = response {
            let event = format!(
                "event: message\ndata: {}\n\n",
                serde_json::to_string(&resp).unwrap_or_default()
            );
            let sessions = state.sessions.read().await;
            if let Some(session) = sessions.get(&session_id) {
                let _ = session.tx.send(event).await;
            }
        }
    }

    // Per MCP SSE spec, POST returns an empty 202 Accepted.
    Ok(Response::builder()
        .status(StatusCode::ACCEPTED)
        .body(SseBody::empty())
        .unwrap())
}

fn json_response(
    status: StatusCode,
    value: serde_json::Value,
) -> Result<Response<SseBody>, hyper::Error> {
    let body = serde_json::to_string(&value).unwrap_or_default();
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(SseBody::once(body))
        .unwrap())
}

/// Streaming SSE body backed by an mpsc channel.
pub struct SseBody {
    rx: mpsc::Receiver<String>,
}

impl SseBody {
    fn new(rx: mpsc::Receiver<String>) -> Self {
        Self { rx }
    }

    fn empty() -> Self {
        let (_tx, rx) = mpsc::channel(1);
        Self { rx }
    }

    fn once(text: String) -> Self {
        let (tx, rx) = mpsc::channel(1);
        let _ = tx.try_send(text);
        Self { rx }
    }
}

impl Body for SseBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(text)) => Poll::Ready(Some(Ok(Frame::data(Bytes::from(text))))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
