use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use kagnode_entropy::{
    crypto::OsRandom,
    engine::{Engine, Mode},
    management::{self, Management},
    server::{self, AppState, Registry},
    sources::SourceStatus,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
fn fixture() -> (tempfile::TempDir, AppState, String) {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("service");
    management::initialize(&dir).unwrap();
    let key = std::fs::read_to_string(dir.join("admin.key")).unwrap();
    let registry = Arc::new(Registry::managed());
    let admin = Management::load(&dir.join("credentials.json"), registry.clone()).unwrap();
    (
        temp,
        AppState {
            engine: Arc::new(Engine::new(Mode::OsBacked, 32, Arc::new(OsRandom), None).unwrap()),
            registry,
            management: Some(admin),
            port: 8098,
            linux_counters: false,
            source_status: Arc::new(Mutex::new(SourceStatus::synthetic())),
        },
        key,
    )
}
fn request(
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    csrf: Option<&str>,
) -> Request<Body> {
    let mut req = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri(path)
        .header("host", "127.0.0.1:8098");
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie)
    }
    if let Some(csrf) = csrf {
        req = req.header("x-csrf-token", csrf)
    }
    let body = body.map(|v| v.to_string()).unwrap_or_default();
    req.header("content-type", "application/json")
        .header("content-length", body.len())
        .body(Body::from(body))
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
async fn login(app: axum::Router, key: &str) -> (String, String) {
    let response = app
        .oneshot(request(
            "/api/v1/admin/login",
            Some(json!({"credential":key})),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (
        cookie,
        value(response).await["csrf"].as_str().unwrap().to_owned(),
    )
}
#[tokio::test]
async fn credentials_are_separate_csrf_reauth_rotation_revocation_and_restart() {
    let (temp, state, key) = fixture();
    let registry = state.registry.clone();
    let app = server::app(state);
    assert_eq!(
        app.clone()
            .oneshot(request("/api/v1/admin/status", None, None, None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let (cookie, csrf) = login(app.clone(), &key).await;
    let change = json!({"id":"consumer-a","action":"create","credential":key});
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/api/v1/admin/clients",
                Some(change.clone()),
                Some(&cookie),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let created = app
        .clone()
        .oneshot(request(
            "/api/v1/admin/clients",
            Some(change),
            Some(&cookie),
            Some(&csrf),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let token = value(created).await["token"].as_str().unwrap().to_owned();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    assert_eq!(registry.authenticate(&headers), Some("consumer-a".into()));
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/api/v1/admin/login",
                Some(json!({"credential":token})),
                None,
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let persisted = std::fs::read_to_string(temp.path().join("service/credentials.json")).unwrap();
    assert!(!persisted.contains(&token));
    assert!(!persisted.contains(&key));
    let rotated = app
        .clone()
        .oneshot(request(
            "/api/v1/admin/clients",
            Some(json!({"id":"consumer-a","action":"rotate","credential":key})),
            Some(&cookie),
            Some(&csrf),
        ))
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::OK);
    let next = value(rotated).await["token"].as_str().unwrap().to_owned();
    assert!(registry.authenticate(&headers).is_none());
    headers.insert("authorization", format!("Bearer {next}").parse().unwrap());
    assert!(registry.authenticate(&headers).is_some());
    // A second service may not reload/write this store concurrently.
    assert!(
        Management::load(
            &temp.path().join("service/credentials.json"),
            Arc::new(Registry::managed())
        )
        .is_err()
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/api/v1/admin/clients",
                Some(json!({"id":"consumer-a","action":"revoke","credential":"wrong"})),
                Some(&cookie),
                Some(&csrf)
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/api/v1/admin/clients",
                Some(json!({"id":"consumer-a","action":"revoke","credential":key})),
                Some(&cookie),
                Some(&csrf)
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(registry.authenticate(&headers).is_none());
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/api/v1/admin/logout",
                Some(json!({})),
                Some(&cookie),
                Some(&csrf)
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(request("/api/v1/admin/status", None, Some(&cookie), None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn duplicate_cookie_cross_origin_and_login_budget() {
    let (_temp, state, key) = fixture();
    let app = server::app(state);
    let (cookie, _) = login(app.clone(), &key).await;
    let duplicate = format!("{cookie}; {cookie}");
    assert_eq!(
        app.clone()
            .oneshot(request(
                "/api/v1/admin/status",
                None,
                Some(&duplicate),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut cross = request(
        "/api/v1/admin/login",
        Some(json!({"credential":key})),
        None,
        None,
    );
    cross
        .headers_mut()
        .insert("origin", "https://attacker.invalid".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(cross).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for _ in 0..9 {
        assert_eq!(
            app.clone()
                .oneshot(request(
                    "/api/v1/admin/login",
                    Some(json!({"credential":"wrong"})),
                    None,
                    None
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        app.oneshot(request(
            "/api/v1/admin/login",
            Some(json!({"credential":key})),
            None,
            None
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
#[test]
fn private_config_never_reinitializes_or_follows_links() {
    let (temp, state, _) = fixture();
    drop(state);
    let dir = temp.path().join("service");
    assert!(management::initialize(&dir).is_err());
    use std::os::unix::fs::{PermissionsExt, symlink};
    symlink(dir.join("credentials.json"), dir.join("alias.json")).unwrap();
    assert!(Management::load(&dir.join("alias.json"), Arc::new(Registry::managed())).is_err());
    std::fs::set_permissions(
        dir.join("credentials.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(
        Management::load(&dir.join("credentials.json"), Arc::new(Registry::managed())).is_err()
    );
}

#[test]
fn private_administrative_configuration_reopens_after_clean_restart() {
    let (temp, state, key) = fixture();
    drop(state);
    let registry = Arc::new(Registry::managed());
    let management =
        Management::load(&temp.path().join("service/credentials.json"), registry).unwrap();
    drop(management);
    assert!(
        !std::fs::read_to_string(temp.path().join("service/credentials.json"))
            .unwrap()
            .contains(&key)
    );
}

#[tokio::test]
async fn consumer_credentials_are_retained_by_restart() {
    let (temp, state, key) = fixture();
    let app = server::app(state);
    let (cookie, csrf) = login(app.clone(), &key).await;
    let response = app
        .clone()
        .oneshot(request(
            "/api/v1/admin/clients",
            Some(json!({"id":"persisted-client","action":"create","credential":key})),
            Some(&cookie),
            Some(&csrf),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let token = value(response).await["token"].as_str().unwrap().to_owned();
    drop(app);
    let registry = Arc::new(Registry::managed());
    let _service = Management::load(
        &temp.path().join("service/credentials.json"),
        registry.clone(),
    )
    .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    assert_eq!(
        registry.authenticate(&headers),
        Some("persisted-client".into())
    );
}

#[tokio::test]
async fn embedded_core_notice_is_authenticated_and_truthfully_unavailable() {
    let (_temp, state, key) = fixture();
    let app = server::app(state);
    let path = "/api/v1/kagnode-core/whats-new";
    assert_eq!(
        app.clone()
            .oneshot(request(path, None, None, None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let (cookie, csrf) = login(app.clone(), &key).await;
    let response = app
        .clone()
        .oneshot(request(path, None, Some(&cookie), None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value, json!({"available":false,"notice":null}));
    let dismiss = "/api/v1/kagnode-core/whats-new/dismiss";
    assert_eq!(
        app.clone()
            .oneshot(request(
                dismiss,
                Some(json!({"id":"fake"})),
                Some(&cookie),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.oneshot(request(
            dismiss,
            Some(json!({"id":"fake"})),
            Some(&cookie),
            Some(&csrf)
        ))
        .await
        .unwrap()
        .status(),
        StatusCode::NOT_FOUND
    );
}
