//! Explicit immutable-owner linking. Profile grants never substitute for a fresh
//! browser-bound sign-in authorization. Provider tokens stay server-side.
use super::*;
use serde_json::Value;
const ISSUER: &str = "https://nebulacoder.com";
const CLIENT: &str = "kagnode-entropy";
fn configured_off() -> bool {
    std::env::var("KAGNODE_ACCOUNT_SYNC")
        .is_ok_and(|v| matches!(v.to_lowercase().as_str(), "off" | "false" | "0"))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Owner {
    subject: String,
    name: String,
    email: String,
    photo: String,
}
#[derive(Clone)]
pub(super) struct Attempt {
    browser: [u8; 32],
    nonce: String,
    request: String,
    state: String,
    verifier: Zeroizing<String>,
    code: Zeroizing<String>,
    denied: bool,
    installation: String,
    started: f64,
    until: Instant,
    polled: Instant,
    interval: u64,
    generation: u64,
    busy: bool,
    link: bool,
}
fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_owned()
}
#[cfg(test)]
thread_local! { static REMOTE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {std::cell::RefCell::new(None)}; }
#[cfg(test)]
thread_local! { static RESPONSES: std::cell::RefCell<std::collections::VecDeque<Value>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) }; }
#[cfg(test)]
async fn remote(_endpoint: &str, _body: Option<Value>, _bearer: Option<&str>) -> Result<Value, ()> {
    let base = TEST_TRANSPORT.with(|v| v.borrow().clone());
    if let Some(base) = base {
        return remote_http(_endpoint, _body, _bearer, &base).await;
    }
    REMOTE_HOOK.with(|v| {
        if let Some(hook) = v.borrow_mut().take() {
            hook();
        }
    });
    let value = RESPONSES.with(|v| v.borrow_mut().pop_front().ok_or(()))?;
    if let Some(ms) = value["_test_delay_ms"].as_u64() {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    Ok(value)
}
#[cfg(test)]
thread_local! {static TEST_TRANSPORT:std::cell::RefCell<Option<String>>=const {std::cell::RefCell::new(None)};}
#[cfg(not(test))]
async fn remote(endpoint: &str, body: Option<Value>, bearer: Option<&str>) -> Result<Value, ()> {
    remote_http(endpoint, body, bearer, ISSUER).await
}
async fn remote_http(
    endpoint: &str,
    body: Option<Value>,
    bearer: Option<&str>,
    base: &str,
) -> Result<Value, ()> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(7))
        .build()
        .map_err(|_| ())?;
    let url = format!("{base}/api/v1/account-sync/{endpoint}");
    let mut request = if let Some(body) = body {
        client
            .post(url)
            .header("content-type", "application/json")
            .body(body.to_string())
    } else {
        client.get(url)
    };
    if let Some(token) = bearer {
        request = request.bearer_auth(token)
    }
    let mut response = request.send().await.map_err(|_| ())?;
    if response.status().is_redirection() {
        return Err(());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > 262144 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() && response.status().is_success() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}
fn browser(headers: &HeaderMap) -> Option<[u8; 32]> {
    if headers.get_all(header::COOKIE).iter().count() != 1 {
        return None;
    }
    let values: Vec<_> = headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|v| v.trim().strip_prefix("entropy_attempt="))
        .collect();
    if values.len() != 1 || values[0].len() != 43 {
        return None;
    }
    Some(digest(values[0]))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    #[serde(default)]
    credential: String,
}
async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<Start>,
) -> Response {
    if configured_off() {
        return error(StatusCode::FORBIDDEN);
    }
    let Some(admin) = state.management.clone() else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let (Ok(cookie), Ok(nonce), Ok(oauth_state), Ok(verifier)) =
        (secret(), secret(), secret(), secret())
    else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    let verifier = Zeroizing::new(verifier);
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|h| {
            *h == format!("127.0.0.1:{}", state.port) || *h == format!("localhost:{}", state.port)
        })
    else {
        return error(StatusCode::FORBIDDEN);
    };
    let installation = format!("http://{host}");
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let (link, generation) = {
        let Ok(mut store) = admin.store.lock() else {
            return error(StatusCode::SERVICE_UNAVAILABLE);
        };
        if !admin.is_healthy() {
            return unavailable();
        }
        store.attempts.retain(|a| a.until > Instant::now());
        if store.attempts.len() >= 32 {
            return error(StatusCode::TOO_MANY_REQUESTS);
        }
        let link = !input.credential.is_empty();
        if link {
            if !Management::authorized(&mut store, &headers, true)
                || !bool::from(digest(&input.credential).ct_eq(&store.config.admin_digest))
            {
                return error(StatusCode::UNAUTHORIZED);
            }
        } else if !store.config.account_enabled || store.config.owner.is_none() {
            return error(StatusCode::FORBIDDEN);
        }
        use zeroize::Zeroize;
        input.credential.zeroize();
        // Reserve capacity before opening the network connection.
        let generation = store.generation;
        store.attempts.push(Attempt {
            browser: digest(&cookie),
            nonce: nonce.clone(),
            request: String::new(),
            state: oauth_state.clone(),
            verifier: verifier.clone(),
            code: Zeroizing::new(String::new()),
            denied: false,
            installation: installation.clone(),
            started: now(),
            until: Instant::now() + Duration::from_secs(600),
            polled: Instant::now(),
            interval: 5,
            generation,
            busy: true,
            link,
        });
        (link, generation)
    };
    let value=remote("browser/start",Some(json!({"client_id":CLIENT,"installation":installation,"redirect_uri":format!("{installation}/auth/kagnode/callback"),"scope":"profile.read email.read","purpose":"sign_in","nonce":nonce,"state":oauth_state,"code_challenge":challenge,"code_challenge_method":"S256"})),None).await;
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Some(index) = store
        .attempts
        .iter()
        .position(|a| a.browser == digest(&cookie) && a.generation == generation && a.link == link)
    else {
        return error(StatusCode::CONFLICT);
    };
    if store.attempts[index].until <= Instant::now() {
        store.attempts.remove(index);
        return error(StatusCode::GONE);
    }
    let value = match value {
        Ok(value) => value,
        Err(_) => {
            store.attempts.remove(index);
            return error(StatusCode::BAD_GATEWAY);
        }
    };
    let request = text(&value, "request_id");
    let uri = text(&value, "authorization_uri");
    let valid = url::Url::parse(&uri).is_ok_and(|u| {
        u.origin().ascii_serialization() == ISSUER
            && u.username().is_empty()
            && u.password().is_none()
            && u.path() == "/oauth/authorize"
            && u.fragment().is_none()
            && u.query_pairs().count() == 1
            && u.query_pairs().any(|(k, v)| k == "request" && v == request)
    });
    if !(32..=200).contains(&request.len()) || !valid || value["error"].is_string() {
        store.attempts.remove(index);
        return error(StatusCode::BAD_GATEWAY);
    }
    let interval = 1;
    let attempt = &mut store.attempts[index];
    attempt.request = request;
    attempt.interval = interval;
    attempt.busy = false;
    let mut response = Json(
        json!({"authorization_uri":uri,"interval":interval,"pending":true,"attempt_id":hex::encode(digest(&cookie)),"expires_in":600}),
    )
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("entropy_attempt={cookie}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600")
            .parse()
            .unwrap(),
    );
    response
}
/// The only accepted query strings are OAuth callbacks and local opaque resume IDs.
pub(crate) fn browser_query_allowed(path: &str, query: &str) -> bool {
    parse_browser_query(path, query).is_some()
}
fn parse_browser_query(
    path: &str,
    query: &str,
) -> Option<std::collections::HashMap<String, String>> {
    if query.len() > 2048 {
        return None;
    }
    let mut values = std::collections::HashMap::new();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        if values.insert(k.into_owned(), v.into_owned()).is_some() {
            return None;
        }
    }
    let opaque = |v: &str, min, max| {
        (min..=max).contains(&v.len())
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    };
    if matches!(path, "/admin" | "/admin/link") {
        return (values.len() == 1
            && values.get("kagnode_attempt").is_some_and(|v| {
                v.len() == 64
                    && v.bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            }))
        .then_some(values);
    }
    if path != "/auth/kagnode/callback"
        || values.len() != 3
        || values.get("iss").map(String::as_str) != Some(ISSUER)
        || !values.get("state").is_some_and(|v| opaque(v, 43, 43))
    {
        return None;
    }
    if values.get("code").is_some_and(|v| opaque(v, 40, 200))
        || values.get("error").is_some_and(|v| opaque(v, 1, 80))
    {
        Some(values)
    } else {
        None
    }
}
async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    if configured_off() {
        return error(StatusCode::FORBIDDEN);
    }
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    let Some(values) = query.and_then(|q| parse_browser_query("/auth/kagnode/callback", &q)) else {
        return error(StatusCode::BAD_REQUEST);
    };
    let Some(browser) = browser(&headers) else {
        return error(StatusCode::UNAUTHORIZED);
    };
    let Ok(mut store) = admin.store.lock() else {
        return unavailable();
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let generation = store.generation;
    let Some(index) = store.attempts.iter().position(|a| {
        a.browser == browser && a.generation == generation && a.until > Instant::now()
    }) else {
        return error(StatusCode::GONE);
    };
    let attempt = &mut store.attempts[index];
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if attempt.busy
        || attempt.installation != format!("http://{host}")
        || !bool::from(attempt.state.as_bytes().ct_eq(values["state"].as_bytes()))
    {
        return error(StatusCode::BAD_REQUEST);
    }
    let destination = if attempt.link {
        "/admin/link"
    } else {
        "/admin"
    };
    attempt.state.clear();
    attempt.until = attempt.until.min(Instant::now() + Duration::from_secs(60));
    if let Some(code) = values.get("code") {
        attempt.code = Zeroizing::new(code.clone());
    } else {
        attempt.denied = true;
    }
    axum::response::Redirect::to(&format!(
        "{destination}?kagnode_attempt={}",
        hex::encode(browser)
    ))
    .into_response()
}
fn validate(
    attempt: &Attempt,
    value: &Value,
    profile: &Value,
    installation: &str,
) -> Result<Owner, ()> {
    let proof = &value["authorization"];
    for (key, expected) in [
        ("issuer", ISSUER),
        ("client_id", CLIENT),
        ("installation", installation),
        ("purpose", "sign_in"),
        ("nonce", attempt.nonce.as_str()),
    ] {
        if proof[key].as_str() != Some(expected) {
            return Err(());
        }
    }
    let auth = proof["auth_time"].as_f64().ok_or(())?;
    let approved = proof["approved_at"].as_f64().ok_or(())?;
    let issued = proof["issued_at"].as_f64().ok_or(())?;
    let expiry = proof["expires_at"].as_f64().ok_or(())?;
    if !(auth > 0.0
        && auth <= approved + 30.0
        && approved >= attempt.started - 30.0
        && approved <= issued
        && issued <= now() + 30.0
        && expiry > now()
        && expiry <= issued + 120.0)
    {
        return Err(());
    }
    let subject = text(profile, "subject");
    let name = text(profile, "name");
    let photo = text(profile, "photo");
    if profile["issuer"].as_str() != Some(ISSUER)
        || subject.is_empty()
        || subject.len() > 200
        || proof["subject"].as_str() != Some(&subject)
        || name.trim().is_empty()
        || name.chars().count() > 100
        || name.chars().any(char::is_control)
        || photo.len() > 180000
    {
        return Err(());
    }
    if !photo.is_empty()
        && (!photo.starts_with("data:image/webp;base64,")
            || base64::engine::general_purpose::STANDARD
                .decode(&photo[23..])
                .is_err())
    {
        return Err(());
    }
    let scopes = text(value, "scope");
    if !scopes.split_whitespace().any(|s| s == "profile.read") {
        return Err(());
    }
    let email = if scopes.split_whitespace().any(|s| s == "email.read")
        && profile["email_verified"] == true
    {
        text(profile, "email")
    } else {
        String::new()
    };
    if email.len() > 254
        || email
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "<>".contains(c))
        || (!email.is_empty() && email.matches('@').count() != 1)
    {
        return Err(());
    }
    Ok(Owner {
        subject,
        name,
        email,
        photo,
    })
}
struct PendingLease {
    admin: Arc<Management>,
    browser: [u8; 32],
    generation: u64,
    keep: bool,
}
impl Drop for PendingLease {
    fn drop(&mut self) {
        if !self.keep
            && let Ok(mut store) = self.admin.store.lock()
        {
            store
                .attempts
                .retain(|a| !(a.browser == self.browser && a.generation == self.generation));
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptRef {
    attempt_id: String,
}
async fn poll(state: State<AppState>, headers: HeaderMap, input: Json<AttemptRef>) -> Response {
    match tokio::time::timeout(Duration::from_secs(6), poll_inner(state, headers, input)).await {
        Ok(response) => response,
        Err(_) => error(StatusCode::GATEWAY_TIMEOUT),
    }
}
async fn poll_inner(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AttemptRef>,
) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if configured_off() {
        return error(StatusCode::FORBIDDEN);
    }
    let Some(browser) = browser(&headers) else {
        return error(StatusCode::UNAUTHORIZED);
    };
    if input.attempt_id != hex::encode(browser) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let attempt = {
        let Ok(mut store) = admin.store.lock() else {
            return error(StatusCode::SERVICE_UNAVAILABLE);
        };
        if !admin.is_healthy() {
            return unavailable();
        }
        store.attempts.retain(|a| a.until > Instant::now());
        let generation = store.generation;
        let Some(a) = store
            .attempts
            .iter_mut()
            .find(|a| a.browser == browser && a.generation == generation)
        else {
            return error(StatusCode::GONE);
        };
        if a.denied {
            store.attempts.retain(|a| a.browser != browser);
            return (StatusCode::FORBIDDEN,Json(json!({"error":{"code":"authorization_denied","message":"KagNode sign-in was not approved. Try again or sign in with your local key."}}))).into_response();
        }
        if a.busy || a.code.is_empty() || a.polled.elapsed() < Duration::from_secs(a.interval) {
            return Json(json!({"pending":true})).into_response();
        }
        a.busy = true;
        a.polled = Instant::now();
        a.clone()
    };
    let _lease = PendingLease {
        admin: admin.clone(),
        browser,
        generation: attempt.generation,
        keep: false,
    };
    let result=remote("token",Some(json!({"client_id":CLIENT,"grant_type":"authorization_code","code":attempt.code.as_str(),"redirect_uri":format!("{}/auth/kagnode/callback",attempt.installation),"code_verifier":attempt.verifier.as_str()})),None).await;
    // Ambiguous exchanges are never retried; no refresh token is a sign-in proof.
    let owner = if let Ok(ref value) = result {
        let access = Zeroizing::new(text(value, "access_token"));
        let refresh = Zeroizing::new(text(value, "refresh_token"));
        let token_valid = |v: &str| {
            (40..=200).contains(&v.len())
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        };
        let profile = if token_valid(&access) && token_valid(&refresh) {
            remote(&format!("profile?client_id={CLIENT}"), None, Some(&access)).await
        } else {
            Err(())
        };
        let revoked = if token_valid(&refresh) {
            remote(
                "revoke",
                Some(json!({"client_id":CLIENT,"token":refresh.as_str()})),
                None,
            )
            .await
            .is_ok_and(|v| v.get("error").is_none())
        } else {
            false
        };
        let profile = if revoked { profile } else { Err(()) };
        profile.and_then(|profile| validate(&attempt, value, &profile, &attempt.installation))
    } else {
        Err(())
    };
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Some(index) = store.attempts.iter().position(|a| {
        a.browser == browser && a.generation == store.generation && a.until > Instant::now()
    }) else {
        return error(StatusCode::GONE);
    };
    store.attempts.remove(index);
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(owner) = owner else {
        return error(StatusCode::BAD_GATEWAY);
    };
    if !attempt.link
        && (!store.config.account_enabled
            || !store
                .config
                .owner
                .as_ref()
                .is_some_and(|old| old.subject == owner.subject))
    {
        return error(StatusCode::FORBIDDEN);
    }
    let mut config = store.config.clone();
    config.owner = Some(owner.clone());
    config.account_enabled = true;
    if admin.persist(&config).is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    }
    store.config = config;
    if attempt.until <= Instant::now() {
        admin.fail_closed();
        return error(StatusCode::GONE);
    }
    let (Ok(cookie), Ok(csrf)) = (secret(), secret()) else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    store.sessions.retain(|s| s.expires > Instant::now());
    if store.sessions.len() >= 16 {
        store.sessions.remove(0);
    }
    store.sessions.push(Session {
        digest: digest(&cookie),
        csrf: csrf.clone(),
        expires: Instant::now() + Duration::from_secs(900),
    });
    let mut response = Json(json!({"authorized":true,"csrf":csrf,"profile":owner})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("entropy_session={cookie}; HttpOnly; SameSite=Strict; Path=/; Max-Age=900")
            .parse()
            .unwrap(),
    );
    response
}
async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AttemptRef>,
) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if configured_off() {
        return error(StatusCode::FORBIDDEN);
    }
    let Some(browser) = browser(&headers) else {
        return error(StatusCode::UNAUTHORIZED);
    };
    if input.attempt_id != hex::encode(browser) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let removed = if let Ok(mut store) = admin.store.lock() {
        store
            .attempts
            .iter()
            .position(|a| a.browser == browser)
            .map(|i| store.attempts.remove(i))
    } else {
        None
    };
    if let Some(a) = removed {
        let _ = remote(
            "browser/cancel",
            Some(json!({"client_id":CLIENT,"request_id":a.request,"code_verifier":a.verifier.as_str()})),
            None,
        )
        .await;
    }
    Json(json!({"pending":false,"cancelled":true})).into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    credential: String,
    enabled: bool,
    #[serde(default)]
    unlink: bool,
}
async fn policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<Policy>,
) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    if !Management::authorized(&mut store, &headers, true)
        || !bool::from(digest(&input.credential).ct_eq(&store.config.admin_digest))
    {
        return error(StatusCode::UNAUTHORIZED);
    }
    use zeroize::Zeroize;
    input.credential.zeroize();
    let mut config = store.config.clone();
    config.account_enabled = input.enabled;
    if input.unlink {
        config.owner = None;
        config.account_enabled = false;
    }
    if admin.persist(&config).is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    }
    store.config = config;
    store.generation = store.generation.wrapping_add(1);
    store.attempts.clear();
    store.sessions.clear();
    Json(json!({"updated":true,"sign_in_again":true})).into_response()
}
async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    if !Management::authorized(&mut store, &headers, false) {
        return error(StatusCode::UNAUTHORIZED);
    }
    Json(json!({"enabled":store.config.account_enabled,"owner":store.config.owner})).into_response()
}
async fn availability(State(state): State<AppState>) -> Json<Value> {
    let enabled = state
        .management
        .and_then(|a| {
            a.store.lock().ok().map(|s| {
                !configured_off()
                    && s.healthy.load(std::sync::atomic::Ordering::SeqCst)
                    && s.config.account_enabled
                    && s.config.owner.is_some()
            })
        })
        .unwrap_or(false);
    Json(json!({"enabled":enabled}))
}
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/kagnode/callback", get(callback))
        .route("/api/v1/admin/kagnode/availability", get(availability))
        .route("/api/v1/admin/kagnode/start", post(start))
        .route("/api/v1/admin/kagnode/poll", post(poll))
        .route("/api/v1/admin/kagnode/cancel", post(cancel))
        .route("/api/v1/admin/kagnode/policy", post(policy))
        .route("/api/v1/admin/kagnode/settings", get(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Attempt, Value, Value) {
        let time = now();
        let attempt = Attempt {
            browser: [0; 32],
            nonce: "N".repeat(43),
            request: "D".repeat(64),
            state: "S".repeat(43),
            verifier: Zeroizing::new("V".repeat(43)),
            code: Zeroizing::new("C".repeat(43)),
            denied: false,
            installation: "http://127.0.0.1:8098".into(),
            started: time - 5.0,
            until: Instant::now() + Duration::from_secs(600),
            polled: Instant::now(),
            interval: 5,
            generation: 0,
            busy: false,
            link: false,
        };
        let value = json!({"scope":"profile.read email.read","authorization":{"issuer":ISSUER,"client_id":CLIENT,"installation":"http://127.0.0.1:8098","purpose":"sign_in","nonce":attempt.nonce,"subject":"immutable-owner","auth_time":time-10.0,"approved_at":time-1.0,"issued_at":time-1.0,"expires_at":time+119.0}});
        let profile = json!({"issuer":ISSUER,"subject":"immutable-owner","name":"Fictional Owner","email":"owner@example.invalid","email_verified":true,"photo":""});
        (attempt, value, profile)
    }
    #[test]
    fn callback_query_is_narrow_issuer_bound_and_has_no_duplicate_or_bearer_fields() {
        let valid = format!(
            "state={}&iss=https%3A%2F%2Fnebulacoder.com&code={}",
            "S".repeat(43),
            "C".repeat(43)
        );
        assert!(browser_query_allowed("/auth/kagnode/callback", &valid));
        for bad in [
            format!("{valid}&state={}", "S".repeat(43)),
            format!("{valid}&error=access_denied"),
            valid.replace("nebulacoder.com", "attacker.invalid"),
            valid.replace("&iss=https%3A%2F%2Fnebulacoder.com", ""),
            format!("{valid}&access_token=secret"),
        ] {
            assert!(!browser_query_allowed("/auth/kagnode/callback", &bad));
        }
        assert!(!browser_query_allowed("/admin", &valid));
        assert!(browser_query_allowed(
            "/admin",
            &format!("kagnode_attempt={}", "a".repeat(64))
        ));
        assert!(!browser_query_allowed(
            "/api/v1/admin/status",
            &format!("kagnode_attempt={}", "a".repeat(64))
        ));
    }
    #[test]
    fn fresh_owner_proof_validates_every_binding_and_expiry() {
        let (a, value, profile) = fixture();
        assert!(validate(&a, &value, &profile, "http://127.0.0.1:8098").is_ok());
        for field in [
            "issuer",
            "client_id",
            "installation",
            "purpose",
            "nonce",
            "subject",
        ] {
            let mut altered = value.clone();
            altered["authorization"][field] = json!("different");
            assert!(
                validate(&a, &altered, &profile, "http://127.0.0.1:8098").is_err(),
                "{field}"
            );
        }
        for field in ["auth_time", "approved_at", "issued_at", "expires_at"] {
            let mut altered = value.clone();
            altered["authorization"][field] = json!(0);
            assert!(
                validate(&a, &altered, &profile, "http://127.0.0.1:8098").is_err(),
                "{field}"
            );
        }
        let mut refreshed = value.clone();
        refreshed.as_object_mut().unwrap().remove("authorization");
        assert!(validate(&a, &refreshed, &profile, "http://127.0.0.1:8098").is_err());
        let mut wrong = profile.clone();
        wrong["subject"] = json!("unlinked-owner");
        assert!(validate(&a, &value, &wrong, "http://127.0.0.1:8098").is_err());
    }
    #[test]
    fn verified_email_requires_explicit_scope_and_profile_is_bounded() {
        let (a, mut value, mut profile) = fixture();
        value["scope"] = json!("profile.read");
        assert_eq!(
            validate(&a, &value, &profile, "http://127.0.0.1:8098")
                .unwrap()
                .email,
            ""
        );
        value["scope"] = json!("profile.read email.read");
        profile["email_verified"] = json!(false);
        assert_eq!(
            validate(&a, &value, &profile, "http://127.0.0.1:8098")
                .unwrap()
                .email,
            ""
        );
        profile["name"] = json!("injected\nname");
        assert!(validate(&a, &value, &profile, "http://127.0.0.1:8098").is_err());
        profile["name"] = json!("Owner");
        profile["photo"] = json!("https://tracking.invalid/photo");
        assert!(validate(&a, &value, &profile, "http://127.0.0.1:8098").is_err());
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use http_body_util::BodyExt;
    async fn value(response: Response) -> Value {
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    fn state() -> (tempfile::TempDir, AppState, String) {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("service");
        initialize(&dir).unwrap();
        let key = fs::read_to_string(dir.join("admin.key")).unwrap();
        let registry = Arc::new(Registry::managed());
        let admin = Management::load(&dir.join("credentials.json"), registry.clone()).unwrap();
        (
            temp,
            AppState {
                engine: Arc::new(
                    crate::engine::Engine::new(
                        crate::engine::Mode::OsBacked,
                        4,
                        Arc::new(crate::crypto::OsRandom),
                        None,
                    )
                    .unwrap(),
                ),
                registry,
                management: Some(admin),
                port: 8098,
                linux_counters: false,
                source_status: Arc::new(Mutex::new(crate::sources::SourceStatus::synthetic())),
            },
            key,
        )
    }
    fn responses(values: Vec<Value>) {
        RESPONSES.with(|r| *r.borrow_mut() = values.into());
    }
    async fn local_session(state: &AppState, key: &str) -> HeaderMap {
        let response = super::super::login(
            State(state.clone()),
            Json(Login {
                credential: key.into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let csrf = text(&value(response).await, "csrf");
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, cookie.parse().unwrap());
        headers.insert("x-csrf-token", csrf.parse().unwrap());
        headers
    }
    async fn begin_pending(
        state: &AppState,
        mut headers: HeaderMap,
        credential: &str,
    ) -> (HeaderMap, String) {
        headers.insert(header::HOST, "127.0.0.1:8098".parse().unwrap());
        responses(vec![
            json!({"request_id":"D".repeat(64),"authorization_uri":format!("{ISSUER}/oauth/authorize?request={}","D".repeat(64)),"expires_in":600}),
        ]);
        let response = start(
            State(state.clone()),
            headers,
            Json(Start {
                credential: credential.into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let id = text(&value(response).await, "attempt_id");
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, cookie.parse().unwrap());
        {
            let mut store = state.management.as_ref().unwrap().store.lock().unwrap();
            store.attempts.last_mut().unwrap().polled = Instant::now() - Duration::from_secs(6);
        }
        headers.insert(header::HOST, "127.0.0.1:8098".parse().unwrap());
        (headers, id)
    }
    async fn begin(state: &AppState, headers: HeaderMap, credential: &str) -> (HeaderMap, String) {
        let (headers, id) = begin_pending(state, headers, credential).await;
        let oauth_state = state
            .management
            .as_ref()
            .unwrap()
            .store
            .lock()
            .unwrap()
            .attempts
            .last()
            .unwrap()
            .state
            .clone();
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("state", &oauth_state)
            .append_pair("iss", ISSUER)
            .append_pair("code", &"C".repeat(43))
            .finish();
        let response = callback(
            State(state.clone()),
            headers.clone(),
            axum::extract::RawQuery(Some(query)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        (headers, id)
    }
    fn approved(state: &AppState, subject: &str) {
        let attempt = state
            .management
            .as_ref()
            .unwrap()
            .store
            .lock()
            .unwrap()
            .attempts
            .last()
            .unwrap()
            .clone();
        let time = now();
        responses(vec![
            json!({"access_token":"A".repeat(64),"refresh_token":"R".repeat(64),"scope":"profile.read email.read","authorization":{"issuer":ISSUER,"client_id":CLIENT,"installation":"http://127.0.0.1:8098","purpose":"sign_in","nonce":attempt.nonce,"subject":subject,"auth_time":time-1.0,"approved_at":time,"issued_at":time,"expires_at":time+120.0}}),
            json!({"issuer":ISSUER,"subject":subject,"name":"Fictional Owner","photo":"","email":"owner@example.invalid","email_verified":true}),
            json!({}),
        ]);
    }
    #[tokio::test]
    async fn callback_requires_pending_cookie_state_issuer_and_one_use_then_denial_is_terminal() {
        let (_temp, state, key) = state();
        let local = local_session(&state, &key).await;
        let (headers, id) = begin_pending(&state, local, &key).await;
        assert!(
            state
                .management
                .as_ref()
                .unwrap()
                .store
                .lock()
                .unwrap()
                .attempts[0]
                .code
                .is_empty()
        );
        assert_eq!(
            value(
                poll(
                    State(state.clone()),
                    headers.clone(),
                    Json(AttemptRef {
                        attempt_id: id.clone()
                    })
                )
                .await
            )
            .await["pending"],
            true
        );
        let oauth_state = state
            .management
            .as_ref()
            .unwrap()
            .store
            .lock()
            .unwrap()
            .attempts[0]
            .state
            .clone();
        let query =
            format!("state={oauth_state}&iss=https%3A%2F%2Fnebulacoder.com&error=access_denied");
        let wrong = query.replace(&oauth_state, &"X".repeat(43));
        assert_eq!(
            callback(
                State(state.clone()),
                headers.clone(),
                axum::extract::RawQuery(Some(wrong))
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            callback(
                State(state.clone()),
                HeaderMap::new(),
                axum::extract::RawQuery(Some(query.clone()))
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let r = callback(
            State(state.clone()),
            headers.clone(),
            axum::extract::RawQuery(Some(query.clone())),
        )
        .await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            r.headers()[header::LOCATION],
            format!("/admin/link?kagnode_attempt={id}")
        );
        assert_eq!(
            callback(
                State(state.clone()),
                headers.clone(),
                axum::extract::RawQuery(Some(query))
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let r = poll(
            State(state.clone()),
            headers.clone(),
            Json(AttemptRef {
                attempt_id: id.clone(),
            }),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
        assert_eq!(value(r).await["error"]["code"], "authorization_denied");
        assert_eq!(
            poll(
                State(state.clone()),
                headers,
                Json(AttemptRef { attempt_id: id })
            )
            .await
            .status(),
            StatusCode::GONE
        );
        assert!(
            state
                .management
                .as_ref()
                .unwrap()
                .store
                .lock()
                .unwrap()
                .config
                .owner
                .is_none()
        );
    }
    #[tokio::test]
    async fn owner_link_sign_in_replay_wrong_owner_and_cancellation() {
        let (_temp, state, key) = state();
        assert_eq!(
            start(
                State(state.clone()),
                HeaderMap::new(),
                Json(Start {
                    credential: String::new()
                })
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let local = local_session(&state, &key).await;
        let (headers, id) = begin(&state, local, &key).await;
        approved(&state, "owner-a");
        let response = poll(
            State(state.clone()),
            headers.clone(),
            Json(AttemptRef {
                attempt_id: id.clone(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(value(response).await["profile"]["subject"], "owner-a");
        assert_eq!(
            poll(
                State(state.clone()),
                headers,
                Json(AttemptRef { attempt_id: id })
            )
            .await
            .status(),
            StatusCode::GONE
        );
        let (headers, id) = begin(&state, HeaderMap::new(), "").await;
        approved(&state, "owner-b");
        assert_eq!(
            poll(
                State(state.clone()),
                headers,
                Json(AttemptRef { attempt_id: id })
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            state
                .management
                .as_ref()
                .unwrap()
                .store
                .lock()
                .unwrap()
                .config
                .owner
                .as_ref()
                .unwrap()
                .subject,
            "owner-a"
        );
        let (headers, id) = begin(&state, HeaderMap::new(), "").await;
        responses(vec![json!({})]);
        assert_eq!(
            cancel(
                State(state.clone()),
                headers.clone(),
                Json(AttemptRef {
                    attempt_id: id.clone()
                })
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            poll(
                State(state.clone()),
                headers,
                Json(AttemptRef { attempt_id: id })
            )
            .await
            .status(),
            StatusCode::GONE
        );
    }
    #[tokio::test]
    async fn local_logout_invalidates_in_flight_owner_link() {
        let (_temp, state, key) = state();
        let local = local_session(&state, &key).await;
        let (headers, id) = begin(&state, local.clone(), &key).await;
        assert_eq!(
            super::super::logout(State(state.clone()), local)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            poll(State(state), headers, Json(AttemptRef { attempt_id: id }))
                .await
                .status(),
            StatusCode::GONE
        );
    }
    #[tokio::test]
    async fn deadline_is_rechecked_after_remote_exchange() {
        let (_temp, state, key) = state();
        let local = local_session(&state, &key).await;
        let (headers, id) = begin(&state, local, &key).await;
        approved(&state, "late-owner");
        let admin = state.management.as_ref().unwrap().clone();
        REMOTE_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                admin.store.lock().unwrap().attempts[0].until =
                    Instant::now() - Duration::from_secs(1);
            }))
        });
        assert_eq!(
            poll(
                State(state.clone()),
                headers,
                Json(AttemptRef { attempt_id: id })
            )
            .await
            .status(),
            StatusCode::GONE
        );
        assert!(
            state
                .management
                .as_ref()
                .unwrap()
                .store
                .lock()
                .unwrap()
                .config
                .owner
                .is_none()
        );
    }
    #[tokio::test]
    async fn stalled_provider_is_terminal_and_does_not_leave_a_busy_attempt() {
        let (_temp, state, key) = state();
        let local = local_session(&state, &key).await;
        let (headers, id) = begin(&state, local, &key).await;
        responses(vec![json!({"_test_delay_ms":7000})]);
        assert_eq!(
            poll(
                State(state.clone()),
                headers,
                Json(AttemptRef { attempt_id: id })
            )
            .await
            .status(),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert!(
            state
                .management
                .as_ref()
                .unwrap()
                .store
                .lock()
                .unwrap()
                .attempts
                .is_empty()
        );
    }
    #[tokio::test]
    async fn browser_binding_and_disabled_policy_invalidate_pending_proof() {
        let (_temp, state, key) = state();
        let local = local_session(&state, &key).await;
        let (headers, id) = begin(&state, local.clone(), &key).await;
        assert_eq!(
            poll(
                State(state.clone()),
                headers.clone(),
                Json(AttemptRef {
                    attempt_id: "wrong-browser".into()
                })
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = policy(
            State(state.clone()),
            local,
            Json(Policy {
                credential: key,
                enabled: false,
                unlink: true,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            poll(State(state), headers, Json(AttemptRef { attempt_id: id }))
                .await
                .status(),
            StatusCode::GONE
        );
    }
}

/// Explicit local QA server, compiled only into the unit-test executable.
#[cfg(test)]
mod browser_fixture {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "explicit fictional issuer/browser fixture; no production transport override exists"]
    async fn serve() {
        let base = std::env::var("ENTROPY_TEST_ISSUER").expect("fictional issuer origin required");
        let parsed = url::Url::parse(&base).unwrap();
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert_eq!(parsed.origin().ascii_serialization(), base);
        assert_eq!(parsed.path(), "/");
        assert!(
            parsed.query().is_none()
                && parsed.fragment().is_none()
                && parsed.username().is_empty()
                && parsed.password().is_none()
        );
        TEST_TRANSPORT.with(|v| *v.borrow_mut() = Some(base));
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("service");
        initialize(&directory).unwrap();
        let credential = fs::read_to_string(directory.join("admin.key")).unwrap();
        let registry = Arc::new(Registry::managed());
        let admin =
            Management::load(&directory.join("credentials.json"), registry.clone()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = AppState {
            engine: Arc::new(
                crate::engine::Engine::new(
                    crate::engine::Mode::OsBacked,
                    4,
                    Arc::new(crate::crypto::OsRandom),
                    None,
                )
                .unwrap(),
            ),
            registry,
            management: Some(admin),
            port,
            linux_counters: false,
            source_status: Arc::new(Mutex::new(crate::sources::SourceStatus::synthetic())),
        };
        let metadata = PathBuf::from(std::env::var("ENTROPY_TEST_STATE").unwrap());
        let stop = metadata.with_extension("stop");
        assert!(!metadata.exists() && !stop.exists());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&metadata)
            .unwrap();
        file.write_all(json!({"origin":format!("http://127.0.0.1:{port}"),"credential":credential,"directory":directory}).to_string().as_bytes()).unwrap();
        drop(file);
        let (tx, rx) = tokio::sync::watch::channel(false);
        let running = tokio::spawn(crate::server::serve(listener, state, rx));
        while !stop.exists() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tx.send(true).unwrap();
        running.await.unwrap().unwrap();
        fs::remove_file(metadata).unwrap();
        fs::remove_file(stop).unwrap();
    }
}
