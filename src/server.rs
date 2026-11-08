use crate::engine::{Engine, Error};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
};

struct Client {
    digest: [u8; 32],
    id: String,
}
struct Bucket {
    available: f64,
    time: Instant,
}
pub struct Registry {
    clients: Vec<Client>,
    buckets: Mutex<HashMap<String, Bucket>>,
    rate: f64,
    burst: f64,
}
impl Registry {
    pub fn new(tokens: Vec<(String, String)>, rate: f64, burst: u32) -> Result<Self, &'static str> {
        if tokens.is_empty()
            || tokens.len() > 64
            || !rate.is_finite()
            || rate <= 0.0
            || !(1..=1024).contains(&burst)
        {
            return Err("Invalid registry bounds");
        }
        let mut clients = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for (mut token, id) in tokens {
            if !(32..=128).contains(&token.len())
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            {
                return Err("Invalid client credentials");
            }
            let digest = Sha256::digest(token.as_bytes()).into();
            use zeroize::Zeroize;
            token.zeroize();
            if clients.iter().any(|c: &Client| c.digest == digest) || !ids.insert(id.clone()) {
                return Err("Duplicate client credential or identity");
            }
            clients.push(Client { digest, id });
        }
        Ok(Self {
            clients,
            buckets: Mutex::new(HashMap::new()),
            rate,
            burst: burst as f64,
        })
    }
    pub fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
            return None;
        }
        let token = headers
            .get(header::AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;
        if !(32..=128).contains(&token.len()) {
            return None;
        }
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let mut identity = None;
        for client in &self.clients {
            if bool::from(digest.ct_eq(&client.digest)) {
                identity = Some(client.id.clone());
            }
        }
        identity
    }
    pub fn allow_at(&self, client: &str, count: usize, now: Instant) -> bool {
        if !(1..=16).contains(&count) || !self.clients.iter().any(|c| c.id == client) {
            return false;
        }
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        let bucket = buckets.entry(client.to_owned()).or_insert(Bucket {
            available: self.burst,
            time: now,
        });
        bucket.available = (bucket.available
            + now.saturating_duration_since(bucket.time).as_secs_f64() * self.rate)
            .min(self.burst);
        bucket.time = now;
        if bucket.available < count as f64 {
            return false;
        }
        bucket.available -= count as f64;
        true
    }
}
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub registry: Arc<Registry>,
    pub port: u16,
    pub linux_counters: bool,
    pub source_status: Arc<Mutex<crate::sources::SourceStatus>>,
}
fn failure(status: StatusCode, code: &str, message: &str) -> Response {
    let mut response = (
        status,
        Json(json!({"error":{"code":code,"message":message}})),
    )
        .into_response();
    if status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, "1".parse().unwrap());
    }
    response
}
async fn boundary(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let valid_host =
        host == format!("127.0.0.1:{}", state.port) || host == format!("localhost:{}", state.port);
    let valid_origin = request
        .headers()
        .get(header::ORIGIN)
        .is_none_or(|v| v.to_str().ok() == Some(format!("http://{host}").as_str()));
    let mut response = if !valid_host || !valid_origin || request.uri().query().is_some() {
        failure(
            StatusCode::FORBIDDEN,
            "origin_rejected",
            "Use the exact local preview URL; query credentials are not accepted",
        )
    } else if request.headers().contains_key(header::TRANSFER_ENCODING) {
        failure(
            StatusCode::BAD_REQUEST,
            "body_encoding",
            "Chunked requests are not accepted",
        )
    } else {
        next.run(request).await
    };
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    h.insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'".parse().unwrap());
    response
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(client) = state.registry.authenticate(&headers) else {
        return failure(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "A valid local client token is required",
        );
    };
    match state.engine.snapshot() {
        Ok(snapshot) => {
            let mut value = serde_json::to_value(snapshot).unwrap();
            value["client"] = json!(client);
            let source = state.source_status.lock().ok().map(|s| s.clone());
            value["source"] = json!(match source.as_ref().map(|s| s.kind) {
                Some("public-video") => "public video",
                Some("synthetic") => "synthetic pixels",
                _ => "unavailable",
            });
            value["source_status"] = json!(source);
            value["counter_source"] = json!(if state.linux_counters {
                "linux-proc-net-dev"
            } else {
                "synthetic"
            });
            Json(value).into_response()
        }
        Err(_) => failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Engine unavailable",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocation {
    #[serde(default = "one")]
    count: usize,
    #[serde(default)]
    context: String,
}
fn one() -> usize {
    1
}
async fn allocate(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(client) = state.registry.authenticate(&headers) else {
        return failure(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "A valid local client token is required",
        );
    };
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/json")
    {
        return failure(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_required",
            "Use application/json",
        );
    }
    let length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if length != Some(body.len()) || body.is_empty() || body.len() > 1024 {
        return failure(
            StatusCode::BAD_REQUEST,
            "invalid_body",
            "Send a bounded JSON body with Content-Length",
        );
    }
    let Ok(input) = serde_json::from_slice::<Allocation>(&body) else {
        return failure(
            StatusCode::BAD_REQUEST,
            "invalid_fields",
            "Only count and context are accepted",
        );
    };
    if !(1..=16).contains(&input.count) || input.context.len() > 128 {
        return failure(
            StatusCode::BAD_REQUEST,
            "invalid_fields",
            "Count must be 1–16 and context at most 128 UTF-8 bytes",
        );
    }
    if !state
        .registry
        .allow_at(&client, input.count, Instant::now())
    {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Client allocation rate exceeded",
        );
    }
    match state.engine.consume(&client,input.count,&input.context) {
        Ok(deliveries)=>Json(json!({"client":client,"mode":state.engine.mode,"blocks":deliveries.iter().map(|d|json!({"instance":state.engine.instance,"sequence":d.sequence,"bytes":64,"base64":STANDARD.encode(d.value.as_ref())})).collect::<Vec<_>>()})).into_response(),
        Err(Error::Empty)=>failure(StatusCode::SERVICE_UNAVAILABLE,"pool_empty","Not enough queued chunks; no allocation occurred"),
        Err(_)=>failure(StatusCode::SERVICE_UNAVAILABLE,"unavailable","Engine unavailable; do not automatically retry an ambiguous allocation"),
    }
}
async fn assets(request: Request) -> Response {
    let (content, mime): (&[u8], &str) = match request.uri().path() {
        "/" => (
            include_bytes!("../static/index.html"),
            "text/html; charset=utf-8",
        ),
        "/app.js" => (
            include_bytes!("../static/app.js"),
            "text/javascript; charset=utf-8",
        ),
        "/app.css" => (
            include_bytes!("../static/app.css"),
            "text/css; charset=utf-8",
        ),
        "/mark.svg" => (include_bytes!("../static/mark.svg"), "image/svg+xml"),
        "/brand/entropy-lockup-dark.svg" => (
            include_bytes!("../static/brand/entropy-lockup-dark.svg"),
            "image/svg+xml",
        ),
        "/brand/entropy-lockup-light.svg" => (
            include_bytes!("../static/brand/entropy-lockup-light.svg"),
            "image/svg+xml",
        ),
        "/brand/entropy-mark-dark.svg" => (
            include_bytes!("../static/brand/entropy-mark-dark.svg"),
            "image/svg+xml",
        ),
        "/brand/entropy-mark-light.svg" => (
            include_bytes!("../static/brand/entropy-mark-light.svg"),
            "image/svg+xml",
        ),
        "/brand/entropy-mark-black.svg" => (
            include_bytes!("../static/brand/entropy-mark-black.svg"),
            "image/svg+xml",
        ),
        "/brand/entropy-mark-white.svg" => (
            include_bytes!("../static/brand/entropy-mark-white.svg"),
            "image/svg+xml",
        ),
        "/brand/favicon.svg" => (
            include_bytes!("../static/brand/favicon.svg"),
            "image/svg+xml",
        ),
        "/brand/favicon.png" => (include_bytes!("../static/brand/favicon.png"), "image/png"),
        "/brand/favicon.ico" | "/favicon.ico" => (
            include_bytes!("../static/brand/favicon.ico"),
            "image/x-icon",
        ),
        "/brand/apple-touch-icon.png" => (
            include_bytes!("../static/brand/apple-touch-icon.png"),
            "image/png",
        ),
        "/core/controls.css" => (
            include_bytes!("../static/core/controls.css"),
            "text/css; charset=utf-8",
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, mime)], content).into_response()
}
pub fn app(state: AppState) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"status":"source-preview","service":"KagNode Entropy","trusted_execution":"none"})) }),
        )
        .route("/api/v1/status", get(status))
        .route("/api/v1/bytes", post(allocate))
        .fallback(get(assets))
        .layer(DefaultBodyLimit::max(1024))
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state)
}
/// Explicitly loopback-only; at most 8 live connections, 5s headers / 10s total.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    mut stop: watch::Receiver<bool>,
) -> std::io::Result<()> {
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Preview requires loopback",
        ));
    }
    let permits = Arc::new(Semaphore::new(8));
    let app = app(state);
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            result=listener.accept()=>{
                let (stream,_)=result?;
                let Ok(permit)=permits.clone().try_acquire_owned() else{drop(stream);continue;};
                let service=hyper_util::service::TowerToHyperService::new(app.clone());
                tasks.spawn(async move {
                    let _permit=permit;
                    let mut builder=hyper::server::conn::http1::Builder::new();
                    builder.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(Duration::from_secs(5)).keep_alive(false).max_buf_size(16384);
                    let connection=builder.serve_connection(hyper_util::rt::TokioIo::new(stream),service);
                    let _=tokio::time::timeout(Duration::from_secs(10),connection).await;
                });
            },
            _=stop.changed()=>{break;},
            Some(_)=tasks.join_next(), if !tasks.is_empty()=>{},
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}
