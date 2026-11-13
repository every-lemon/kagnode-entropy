#[path = "account.rs"]
mod account;
pub(crate) use account::browser_query_allowed;
// Local administration is separate from consumer bearer credentials. No outputs
// or credential plaintext are stored in the administrative event ring.
use crate::server::{AppState, Client, Registry};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    owner: Option<account::Owner>,
    #[serde(default)]
    account_enabled: bool,
    version: u8,
    admin_digest: [u8; 32],
    clients: Vec<Client>,
}
struct Session {
    digest: [u8; 32],
    csrf: String,
    expires: Instant,
}
struct Store {
    healthy: Arc<std::sync::atomic::AtomicBool>,
    attempts: Vec<account::Attempt>,
    generation: u64,
    config: Config,
    sessions: Vec<Session>,
    events: VecDeque<String>,
    login_times: VecDeque<Instant>,
}
pub struct Management {
    healthy: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(test)]
    fail_after_rename: std::sync::atomic::AtomicBool,
    path: PathBuf,
    _lease: fs::File,
    store: Mutex<Store>,
    registry: Arc<Registry>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
fn secret() -> io::Result<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| invalid("OS randomness unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes.as_ref()))
}
fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn check_directory(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir()
        || meta.mode() & 0o077 != 0
        || meta.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(invalid(
            "Administration directory must be owned by the service user with mode 0700",
        ));
    }
    Ok(())
}
pub fn initialize(directory: &Path) -> io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(directory)?;
    let credential = Zeroizing::new(secret()?);
    let config = Config {
        owner: None,
        account_enabled: false,
        version: 1,
        admin_digest: digest(&credential),
        clients: vec![],
    };
    write_new(
        &directory.join("credentials.json"),
        &serde_json::to_vec_pretty(&config)?,
    )?;
    write_new(&directory.join("admin.key"), credential.as_bytes())?;
    fs::File::open(directory)?.sync_all()
}
impl Management {
    pub fn load(path: &Path, registry: Arc<Registry>) -> io::Result<Arc<Self>> {
        check_directory(path.parent().ok_or_else(|| invalid("Missing directory"))?)?;
        use std::os::fd::AsRawFd;
        let lease = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path.parent().unwrap().join(".service.lock"))?;
        let lease_meta = lease.metadata()?;
        if !lease_meta.is_file()
            || lease_meta.mode() & 0o077 != 0
            || lease_meta.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(invalid("Invalid service lock"));
        }
        // SAFETY: flock operates on this owned live descriptor. Keep it open for
        // the entire Management lifetime; concurrent writers fail without waiting.
        if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(invalid("This administration directory is already in use"));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.len() > 262144
            || meta.mode() & 0o077 != 0
            || meta.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(invalid(
                "Credentials must be a private bounded owned regular file",
            ));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.read_to_end(&mut bytes)?;
        let config: Config = serde_json::from_slice(&bytes)?;
        if config.version != 1 || config.clients.len() > 64 || config.admin_digest == [0; 32] {
            return Err(invalid("Invalid credential configuration"));
        }
        let mut ids = std::collections::HashSet::new();
        let mut digests = std::collections::HashSet::new();
        for client in &config.clients {
            if !valid_id(&client.id)
                || !ids.insert(client.id.clone())
                || !digests.insert(client.digest)
                || client.digest == config.admin_digest
            {
                return Err(invalid("Invalid or duplicated client"));
            }
        }
        registry
            .replace_clients(config.clients.clone())
            .map_err(invalid)?;
        let healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        Ok(Arc::new(Self {
            healthy: healthy.clone(),
            #[cfg(test)]
            fail_after_rename: std::sync::atomic::AtomicBool::new(false),
            path: path.to_owned(),
            _lease: lease,
            registry,
            store: Mutex::new(Store {
                healthy,
                config,
                attempts: vec![],
                generation: 0,
                sessions: vec![],
                events: VecDeque::new(),
                login_times: VecDeque::new(),
            }),
        }))
    }
    fn fail_closed(&self) {
        self.healthy
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.registry.disable();
    }
    fn is_healthy(&self) -> bool {
        self.healthy.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn persist(&self, config: &Config) -> io::Result<()> {
        if !self.is_healthy() {
            return Err(invalid(
                "Configuration state is uncertain; stop and review before restart",
            ));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| invalid("Missing directory"))?;
        check_directory(parent)?;
        let temporary = parent.join(format!(".credentials-{}", secret()?));
        let mut renamed = false;
        let result = (|| {
            write_new(&temporary, &serde_json::to_vec_pretty(config)?)?;
            fs::rename(&temporary, &self.path)?;
            renamed = true;
            #[cfg(test)]
            if self
                .fail_after_rename
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(invalid("Injected directory sync failure"));
            }
            fs::File::open(parent)?.sync_all()
        })();
        if result.is_err() {
            if renamed {
                self.fail_closed();
            }
            let _ = fs::remove_file(temporary);
        }
        result
    }
    fn authorized(store: &mut Store, headers: &HeaderMap, write: bool) -> bool {
        if !store.healthy.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        if headers.get_all(header::COOKIE).iter().count() != 1 {
            return false;
        }
        store.sessions.retain(|s| s.expires > Instant::now());
        let cookies: Vec<_> = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|v| v.trim().strip_prefix("entropy_session="))
            .collect();
        if cookies.len() != 1 {
            return false;
        }
        let hash = digest(cookies[0]);
        store.sessions.iter().any(|s| {
            bool::from(hash.ct_eq(&s.digest))
                && (!write
                    || headers
                        .get("x-csrf-token")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| bool::from(v.as_bytes().ct_eq(s.csrf.as_bytes()))))
        })
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":{"code":"configuration_unavailable","message":"Configuration storage is uncertain. Stop the service and review its private configuration before restarting."}}))).into_response()
}
fn error(status: StatusCode) -> Response {
    (status,Json(json!({"error":{"code":"admin_rejected","message":"Administrative request rejected; sign in again or check the request."}}))).into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    credential: String,
}
async fn login(State(state): State<AppState>, Json(mut input): Json<Login>) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let now = Instant::now();
    store
        .login_times
        .retain(|t| now.duration_since(*t) < Duration::from_secs(60));
    if store.login_times.len() >= 10 {
        return error(StatusCode::TOO_MANY_REQUESTS);
    }
    store.login_times.push_back(now);
    let supplied = digest(&input.credential);
    use zeroize::Zeroize;
    input.credential.zeroize();
    if !bool::from(supplied.ct_eq(&store.config.admin_digest)) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let (Ok(credential), Ok(csrf)) = (secret(), secret()) else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    store.sessions.retain(|s| s.expires > now);
    if store.sessions.len() >= 16 {
        store.sessions.remove(0);
    }
    store.sessions.push(Session {
        digest: digest(&credential),
        csrf: csrf.clone(),
        expires: now + Duration::from_secs(900),
    });
    let mut response = Json(json!({"csrf":csrf,"expires_in":900})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        format!("entropy_session={credential}; HttpOnly; SameSite=Strict; Path=/; Max-Age=900")
            .parse()
            .unwrap(),
    );
    response
}
async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    if !Management::authorized(&mut store, &headers, false) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';')
                .find_map(|v| v.trim().strip_prefix("entropy_session="))
        })
        .unwrap_or("");
    let csrf = store
        .sessions
        .iter()
        .find(|s| bool::from(s.digest.ct_eq(&digest(cookie))))
        .map(|s| s.csrf.clone());
    Json(json!({"csrf":csrf,"clients":store.config.clients.iter().map(|c| &c.id).collect::<Vec<_>>(),"events":store.events,"engine":state.engine.snapshot().ok(),"rate_blocks_per_second":16,"burst_blocks":32})).into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    id: String,
    action: String,
    credential: String,
}
async fn clients(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<Change>,
) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    if !Management::authorized(&mut store, &headers, true) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let hash = digest(&input.credential);
    use zeroize::Zeroize;
    input.credential.zeroize();
    if !bool::from(hash.ct_eq(&store.config.admin_digest))
        || !valid_id(&input.id)
        || !["create", "rotate", "revoke"].contains(&input.action.as_str())
    {
        return error(StatusCode::BAD_REQUEST);
    }
    let exists = store.config.clients.iter().any(|c| c.id == input.id);
    if (input.action == "create" && (exists || store.config.clients.len() >= 64))
        || (input.action != "create" && !exists)
    {
        return error(StatusCode::CONFLICT);
    }
    let mut config = store.config.clone();
    config.clients.retain(|c| c.id != input.id);
    let credential = if input.action == "revoke" {
        None
    } else {
        match secret() {
            Ok(token) => {
                config.clients.push(Client {
                    id: input.id.clone(),
                    digest: digest(&token),
                });
                Some(token)
            }
            Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE),
        }
    };
    if admin.persist(&config).is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    }
    if admin
        .registry
        .replace_clients(config.clients.clone())
        .is_err()
    {
        admin.fail_closed();
        return error(StatusCode::SERVICE_UNAVAILABLE);
    }
    store.config = config;
    store
        .events
        .push_back(format!("{}:{}", input.action, input.id));
    if store.events.len() > 100 {
        store.events.pop_front();
    }
    Json(json!({"id":input.id,"action":input.action,"token":credential})).into_response()
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(mut store) = admin.store.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    if !Management::authorized(&mut store, &headers, true) {
        return error(StatusCode::UNAUTHORIZED);
    }
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split(';')
                .find_map(|v| v.trim().strip_prefix("entropy_session="))
        })
        .unwrap_or("");
    store
        .sessions
        .retain(|s| !bool::from(s.digest.ct_eq(&digest(cookie))));
    store.generation = store.generation.wrapping_add(1);
    store.attempts.clear();
    (
        [(
            header::SET_COOKIE,
            "entropy_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        )],
        Json(json!({"signed_out":true})),
    )
        .into_response()
}
// This embedded build has no automatic Core manager/event channel. Do not
// manufacture an update notice or expose privileged local configuration.
async fn core_notice(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(mut store) = admin.store.lock() else {
        return unavailable();
    };
    if !Management::authorized(&mut store, &headers, false) {
        return error(StatusCode::UNAUTHORIZED);
    }
    Json(json!({"available":false,"notice":null})).into_response()
}
async fn core_notice_dismiss(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(admin) = state.management else {
        return error(StatusCode::NOT_FOUND);
    };
    if !admin.is_healthy() {
        return unavailable();
    }
    let Ok(mut store) = admin.store.lock() else {
        return unavailable();
    };
    if !Management::authorized(&mut store, &headers, true) {
        return error(StatusCode::UNAUTHORIZED);
    }
    error(StatusCode::NOT_FOUND)
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(account::routes())
        .route("/api/v1/kagnode-core/whats-new", get(core_notice))
        .route(
            "/api/v1/kagnode-core/whats-new/dismiss",
            post(core_notice_dismiss),
        )
        .route("/api/v1/admin/login", post(login))
        .route("/api/v1/admin/status", get(status))
        .route("/api/v1/admin/clients", post(clients))
        .route("/api/v1/admin/logout", post(logout))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_session_cannot_authorize_and_csrf_is_bound() {
        let mut store = Store {
            healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            config: Config {
                owner: None,
                account_enabled: false,
                version: 1,
                admin_digest: [1; 32],
                clients: vec![],
            },
            attempts: vec![],
            generation: 0,
            sessions: vec![Session {
                digest: digest("session"),
                csrf: "csrf".into(),
                expires: Instant::now() - Duration::from_secs(1),
            }],
            events: VecDeque::new(),
            login_times: VecDeque::new(),
        };
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, "entropy_session=session".parse().unwrap());
        headers.insert("x-csrf-token", "csrf".parse().unwrap());
        assert!(!Management::authorized(&mut store, &headers, true));
        assert!(store.sessions.is_empty());
        store.sessions.push(Session {
            digest: digest("session"),
            csrf: "correct".into(),
            expires: Instant::now() + Duration::from_secs(60),
        });
        assert!(Management::authorized(&mut store, &headers, false));
        assert!(!Management::authorized(&mut store, &headers, true));
    }
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    #[test]
    fn post_rename_sync_failure_disables_old_consumers_and_admin_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("service");
        initialize(&dir).unwrap();
        let registry = Arc::new(Registry::managed());
        let admin = Management::load(&dir.join("credentials.json"), registry.clone()).unwrap();
        let mut store = admin.store.lock().unwrap();
        let token = "A".repeat(43);
        store.config.clients.push(Client {
            id: "old-client".into(),
            digest: digest(&token),
        });
        admin.persist(&store.config).unwrap();
        registry
            .replace_clients(store.config.clients.clone())
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert!(registry.authenticate(&headers).is_some());
        store.sessions.push(Session {
            digest: digest("session"),
            csrf: "csrf".into(),
            expires: Instant::now() + Duration::from_secs(60),
        });
        let mut config = store.config.clone();
        config.clients.clear();
        admin
            .fail_after_rename
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(admin.persist(&config).is_err());
        assert!(!admin.is_healthy());
        assert!(registry.authenticate(&headers).is_none());
        headers.insert(header::COOKIE, "entropy_session=session".parse().unwrap());
        assert!(!Management::authorized(&mut store, &headers, false));
        let disk: Config =
            serde_json::from_slice(&fs::read(dir.join("credentials.json")).unwrap()).unwrap();
        assert!(disk.clients.is_empty());
        assert!(!store.config.clients.is_empty());
        assert!(admin.persist(&store.config).is_err()); // No ordinary retry can restore old authority.
    }
}
