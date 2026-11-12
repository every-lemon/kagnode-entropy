use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use kagnode_entropy::{
    crypto::RandomSource,
    engine::{Engine, Mode},
    server::{self, AppState, Registry},
    sources::SyntheticSource,
};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tower::ServiceExt;
struct Fixed;
impl RandomSource for Fixed {
    fn fill(&self, out: &mut [u8]) -> Result<(), &'static str> {
        out.fill(4);
        Ok(())
    }
}
fn state() -> AppState {
    let e = Arc::new(Engine::new(Mode::OsBacked, 32, Arc::new(Fixed), None).unwrap());
    let mut source = SyntheticSource::default();
    for _ in 0..32 {
        e.ingest(&source.sample()).unwrap();
    }
    AppState {
        engine: e,
        registry: Arc::new(
            Registry::new(
                vec![
                    ("A".repeat(43), "alice".into()),
                    ("B".repeat(43), "bob".into()),
                ],
                16.0,
                32,
            )
            .unwrap(),
        ),
        management: None,
        port: 8098,
        linux_counters: false,
        source_status: Arc::new(std::sync::Mutex::new(
            kagnode_entropy::sources::SourceStatus::synthetic(),
        )),
    }
}
fn request(method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8098");
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    let bytes = body.map(|v| v.to_string()).unwrap_or_default();
    if method == "POST" {
        r = r
            .header("content-type", "application/json")
            .header("content-length", bytes.len());
    }
    r.body(Body::from(bytes)).unwrap()
}
async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn auth_and_query_credentials_rejected() {
    let app = server::app(state());
    assert_eq!(
        app.clone()
            .oneshot(request("GET", "/api/v1/status", None, None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.oneshot(request(
            "GET",
            "/api/v1/status?token=fake",
            Some(&"A".repeat(43)),
            None
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn authenticated_identity_and_exact_delivery() {
    let app = server::app(state());
    let r = app
        .oneshot(request(
            "POST",
            "/api/v1/bytes",
            Some(&"B".repeat(43)),
            Some(json!({"count":2,"context":"test"})),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    let v = json_body(r).await;
    assert_eq!(v["client"], "bob");
    assert_eq!(v["blocks"].as_array().unwrap().len(), 2);
    assert_eq!(v["blocks"][0]["bytes"], 64);
    // A deterministic OS fixture repeats; production does not manufacture uniqueness.
    assert_eq!(v["blocks"][0]["base64"], v["blocks"][1]["base64"]);
    assert_ne!(v["blocks"][0]["sequence"], v["blocks"][1]["sequence"]);
}
#[tokio::test]
async fn caller_identity_and_bad_bounds_rejected() {
    let app = server::app(state());
    for body in [
        json!({"client":"bob"}),
        json!({"count":0}),
        json!({"count":17}),
        json!({"count":true}),
        json!({"context":"x".repeat(129)}),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "POST",
                    "/api/v1/bytes",
                    Some(&"A".repeat(43)),
                    Some(body)
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
#[tokio::test]
async fn host_origin_methods_and_transfer_encoding() {
    let app = server::app(state());
    let mut bad = request("GET", "/", None, None);
    bad.headers_mut()
        .insert("host", "evil.test".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(bad).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let mut bad = request("GET", "/", None, None);
    bad.headers_mut()
        .insert("origin", "https://evil.test".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(bad).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let mut bad = request(
        "POST",
        "/api/v1/bytes",
        Some(&"A".repeat(43)),
        Some(json!({})),
    );
    bad.headers_mut()
        .insert("transfer-encoding", "chunked".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(bad).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.oneshot(request("OPTIONS", "/api/v1/bytes", None, None))
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}
#[tokio::test]
async fn local_assets_csp_and_no_source_paths() {
    let app = server::app(state());
    for path in [
        "/",
        "/app.js",
        "/app.css",
        "/mark.svg",
        "/brand/entropy-lockup-dark.svg",
        "/brand/entropy-lockup-light.svg",
        "/brand/favicon.svg",
        "/brand/favicon.png",
        "/brand/entropy-mark-512.png",
        "/brand/favicon.ico",
        "/favicon.ico",
        "/brand/apple-touch-icon.png",
        "/core/controls.css",
    ] {
        let r = app
            .clone()
            .oneshot(request("GET", path, None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert!(
            r.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
    }
    let health = app
        .clone()
        .oneshot(request("GET", "/health", None, None))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        json_body(health).await["version"],
        env!("CARGO_PKG_VERSION")
    );
    for path in ["/Cargo.toml", "/../Cargo.toml", "/src/engine.rs"] {
        assert_eq!(
            app.clone()
                .oneshot(request("GET", path, None, None))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}
#[tokio::test]
async fn oversized_body() {
    let app = server::app(state());
    let r = app
        .oneshot(request(
            "POST",
            "/api/v1/bytes",
            Some(&"A".repeat(43)),
            Some(json!({"context":"x".repeat(2000)})),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
#[tokio::test]
async fn concurrent_http_requests_do_not_reallocate() {
    let s = state();
    let app = server::app(s);
    let mut jobs = tokio::task::JoinSet::new();
    for n in 0..16 {
        let app = app.clone();
        jobs.spawn(async move {
            let token = if n % 2 == 0 { "A" } else { "B" }.repeat(43);
            let r = app
                .oneshot(request(
                    "POST",
                    "/api/v1/bytes",
                    Some(&token),
                    Some(json!({})),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            json_body(r).await["blocks"][0]["sequence"]
                .as_u64()
                .unwrap()
        });
    }
    let mut values = Vec::new();
    while let Some(v) = jobs.join_next().await {
        values.push(v.unwrap());
    }
    values.sort();
    assert_eq!(values, (1..=16).collect::<Vec<_>>());
}
#[test]
fn rate_buckets_isolated_and_refill_bounded() {
    let registry = Registry::new(
        vec![
            ("A".repeat(43), "alice".into()),
            ("B".repeat(43), "bob".into()),
        ],
        1.0,
        2,
    )
    .unwrap();
    let now = Instant::now();
    assert!(registry.allow_at("alice", 2, now));
    assert!(!registry.allow_at("alice", 1, now));
    assert!(registry.allow_at("bob", 1, now));
    assert!(registry.allow_at("alice", 1, now + Duration::from_secs(1)));
    assert!(!registry.allow_at("unknown", 1, now));
}
#[tokio::test]
async fn public_listener_is_refused() {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let (_, rx) = tokio::sync::watch::channel(false);
    assert!(server::serve(listener, state(), rx).await.is_err());
}

#[test]
fn out_of_order_requests_cannot_rewind_the_rate_bucket_clock() {
    let registry = Registry::new(vec![("A".repeat(43), "alice".into())], 1.0, 2).unwrap();
    let start = Instant::now();
    assert!(registry.allow_at("alice", 2, start));
    let newer = start + Duration::from_secs(2);
    assert!(registry.allow_at("alice", 2, newer));
    // A request can obtain its timestamp before waiting for the shared lock.
    assert!(!registry.allow_at("alice", 1, start + Duration::from_secs(1)));
    assert!(!registry.allow_at("alice", 1, newer));
    assert!(registry.allow_at("alice", 1, newer + Duration::from_secs(1)));
}

#[tokio::test]
async fn only_bound_browser_callback_and_resume_queries_cross_the_query_boundary() {
    let app = server::app(state());
    let query = format!(
        "state={}&iss=https%3A%2F%2Fnebulacoder.com&code={}",
        "S".repeat(43),
        "C".repeat(43)
    );
    let response = app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/auth/kagnode/callback?{query}"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND); // No management configured, not a query-boundary rejection.
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    for (method, path) in [
        ("POST", format!("/auth/kagnode/callback?{query}")),
        (
            "GET",
            format!("/auth/kagnode/callback?{query}&token=secret"),
        ),
        (
            "GET",
            format!(
                "/auth/kagnode/callback?{}",
                query.replace("nebulacoder.com", "attacker.invalid")
            ),
        ),
        (
            "GET",
            format!("/api/v1/status?kagnode_attempt={}", "a".repeat(64)),
        ),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(method, &path, None, None))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        app.oneshot(request(
            "GET",
            &format!("/admin?kagnode_attempt={}", "a".repeat(64)),
            None,
            None
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::OK
    );
}
