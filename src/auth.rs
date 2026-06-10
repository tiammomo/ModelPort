use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::http::HeaderMap;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{config::AppConfig, error::AppError};

pub const ADMIN_SESSION_COOKIE: &str = "modelport_admin_session";

const DEFAULT_SESSION_TTL_SECONDS: u64 = 12 * 60 * 60;
const MAX_FAILED_ATTEMPTS: u32 = 5;
const LOCKOUT_SECONDS: u64 = 15 * 60;

#[derive(Debug)]
pub struct AuthStore {
    path: Option<PathBuf>,
    inner: Mutex<AuthInner>,
    session_ttl_seconds: u64,
    cookie_secure: bool,
}

#[derive(Debug, Default)]
struct AuthInner {
    users: BTreeMap<String, AdminUserRecord>,
    sessions: HashMap<String, AdminSession>,
    attempts: HashMap<String, LoginAttempt>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthFile {
    users: Vec<AdminUserRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdminUserRecord {
    id: String,
    username: String,
    email: String,
    role: String,
    status: String,
    password_hash: String,
    created_at_ms: u64,
    updated_at_ms: u64,
    last_login_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct AdminSession {
    user_id: String,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct LoginAttempt {
    failed_count: u32,
    locked_until_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicUser {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub api_key_count: u32,
    pub request_count_24h: u64,
}

#[derive(Debug)]
pub struct LoginResult {
    pub session_token: String,
    pub expires_at_ms: u64,
    pub user: PublicUser,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserInput {
    pub username: String,
    pub email: String,
    pub password: String,
    pub role: Option<String>,
    pub status: Option<String>,
}

impl AuthStore {
    pub fn load_or_bootstrap(config: &AppConfig) -> Result<Self, AppError> {
        let path = auth_store_path();
        let session_ttl_seconds = env_u64(
            "MODELPORT_ADMIN_SESSION_TTL_SECONDS",
            DEFAULT_SESSION_TTL_SECONDS,
        );
        let cookie_secure = env_flag("MODELPORT_ADMIN_COOKIE_SECURE");
        let users = if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let file: AuthFile = serde_json::from_str(&raw)?;
            file.users
                .into_iter()
                .map(|user| (user.id.clone(), user))
                .collect()
        } else {
            BTreeMap::new()
        };

        let store = Self {
            path: Some(path),
            inner: Mutex::new(AuthInner {
                users,
                sessions: HashMap::new(),
                attempts: HashMap::new(),
            }),
            session_ttl_seconds,
            cookie_secure,
        };

        store.bootstrap_first_admin(config)?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            path: None,
            inner: Mutex::new(AuthInner::default()),
            session_ttl_seconds: DEFAULT_SESSION_TTL_SECONDS,
            cookie_secure: false,
        }
    }

    pub fn login(&self, input: LoginInput) -> Result<LoginResult, AppError> {
        let username = normalize_username(&input.username)?;
        if input.password.is_empty() {
            return Err(AppError::Auth);
        }

        let now_ms = now_millis();
        let mut inner = self.inner.lock().expect("auth lock poisoned");
        self.prune_expired_sessions_locked(&mut inner, now_ms);

        if inner
            .attempts
            .get(&username)
            .is_some_and(|attempt| attempt.locked_until_ms > now_ms)
        {
            return Err(AppError::Auth);
        }

        let user_id = inner
            .users
            .iter()
            .find(|(_, user)| user.username.eq_ignore_ascii_case(&username))
            .map(|(id, _)| id.clone());
        let Some(user_id) = user_id else {
            record_failed_attempt(inner.attempts.entry(username.clone()).or_default(), now_ms);
            return Err(AppError::Auth);
        };

        let Some(user) = inner.users.get(&user_id).cloned() else {
            record_failed_attempt(inner.attempts.entry(username.clone()).or_default(), now_ms);
            return Err(AppError::Auth);
        };

        if user.status != "active" || !verify_password(&input.password, &user.password_hash) {
            record_failed_attempt(inner.attempts.entry(username.clone()).or_default(), now_ms);
            return Err(AppError::Auth);
        }

        inner.attempts.remove(&username);

        let token = new_session_token();
        let token_hash = hash_session_token(&token);
        let expires_at_ms = now_ms.saturating_add(self.session_ttl_seconds.saturating_mul(1_000));

        if let Some(user) = inner.users.get_mut(&user_id) {
            user.last_login_at_ms = Some(now_ms);
            user.updated_at_ms = now_ms;
        }

        inner.sessions.insert(
            token_hash,
            AdminSession {
                user_id: user_id.clone(),
                expires_at_ms,
            },
        );

        self.save_locked(&inner)?;

        let user = inner
            .users
            .get(&user_id)
            .map(|user| public_user(user, 1, 0))
            .ok_or_else(|| AppError::Config("admin user disappeared during login".to_owned()))?;

        Ok(LoginResult {
            session_token: token,
            expires_at_ms,
            user,
        })
    }

    pub fn require_session(&self, headers: &HeaderMap) -> Result<PublicUser, AppError> {
        let token = session_token_from_headers(headers).ok_or(AppError::Auth)?;
        let now_ms = now_millis();
        let mut inner = self.inner.lock().expect("auth lock poisoned");
        self.prune_expired_sessions_locked(&mut inner, now_ms);

        let token_hash = hash_session_token(token);
        let session = inner
            .sessions
            .get(&token_hash)
            .cloned()
            .ok_or(AppError::Auth)?;
        if session.expires_at_ms <= now_ms {
            inner.sessions.remove(&token_hash);
            return Err(AppError::Auth);
        }

        let user = inner.users.get(&session.user_id).ok_or(AppError::Auth)?;
        if user.status != "active" {
            return Err(AppError::Auth);
        }

        Ok(public_user(user, 1, 0))
    }

    pub fn logout(&self, headers: &HeaderMap) {
        let Some(token) = session_token_from_headers(headers) else {
            return;
        };
        let mut inner = self.inner.lock().expect("auth lock poisoned");
        inner.sessions.remove(&hash_session_token(token));
    }

    pub fn list_users(&self, request_count_24h: u64) -> Vec<PublicUser> {
        let inner = self.inner.lock().expect("auth lock poisoned");
        let api_key_count = if inner.users.is_empty() { 0 } else { 1 };
        inner
            .users
            .values()
            .map(|user| public_user(user, api_key_count, request_count_24h))
            .collect()
    }

    pub fn create_user(&self, input: CreateUserInput) -> Result<PublicUser, AppError> {
        let username = normalize_username(&input.username)?;
        let email = validate_email(&input.email)?;
        let role = validate_role(input.role.as_deref().unwrap_or("user"))?;
        let status = validate_status(input.status.as_deref().unwrap_or("active"))?;
        validate_password_strength(&input.password)?;

        let password_hash = hash_password(&input.password)?;
        let now_ms = now_millis();
        let user = AdminUserRecord {
            id: format!("usr_{}", Uuid::new_v4().simple()),
            username,
            email,
            role,
            status,
            password_hash,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            last_login_at_ms: None,
        };

        let mut inner = self.inner.lock().expect("auth lock poisoned");
        if inner
            .users
            .values()
            .any(|existing| existing.username.eq_ignore_ascii_case(&user.username))
        {
            return Err(AppError::InvalidRequest(
                "username already exists".to_owned(),
            ));
        }

        let public = public_user(&user, 1, 0);
        inner.users.insert(user.id.clone(), user);
        self.save_locked(&inner)?;
        Ok(public)
    }

    pub fn delete_user(&self, user_id: &str, current_user_id: &str) -> Result<(), AppError> {
        if user_id == current_user_id {
            return Err(AppError::Forbidden(
                "cannot delete the current signed-in user".to_owned(),
            ));
        }

        let mut inner = self.inner.lock().expect("auth lock poisoned");
        let Some(user) = inner.users.get(user_id) else {
            return Ok(());
        };

        if user.role == "admin" {
            let active_admins = inner
                .users
                .values()
                .filter(|candidate| candidate.role == "admin" && candidate.status == "active")
                .count();
            if active_admins <= 1 {
                return Err(AppError::Forbidden(
                    "cannot delete the last active admin".to_owned(),
                ));
            }
        }

        inner.users.remove(user_id);
        inner
            .sessions
            .retain(|_, session| session.user_id.as_str() != user_id);
        self.save_locked(&inner)
    }

    pub fn active_user_count(&self) -> usize {
        let inner = self.inner.lock().expect("auth lock poisoned");
        inner
            .users
            .values()
            .filter(|user| user.status == "active")
            .count()
    }

    pub fn session_cookie(&self, token: &str) -> String {
        let mut cookie = format!(
            "{ADMIN_SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
            self.session_ttl_seconds
        );
        if self.cookie_secure {
            cookie.push_str("; Secure");
        }
        cookie
    }

    pub fn clear_cookie(&self) -> String {
        let mut cookie =
            format!("{ADMIN_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
        if self.cookie_secure {
            cookie.push_str("; Secure");
        }
        cookie
    }

    fn bootstrap_first_admin(&self, config: &AppConfig) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("auth lock poisoned");
        if !inner.users.is_empty() {
            return Ok(());
        }

        let password = env::var("MODELPORT_ADMIN_PASSWORD")
            .ok()
            .or_else(|| config.auth_token.clone())
            .ok_or_else(|| {
                AppError::Config(
                    "MODELPORT_ADMIN_PASSWORD is required to bootstrap the first admin user"
                        .to_owned(),
                )
            })?;
        validate_bootstrap_password(&password)?;

        let username = normalize_username(
            &env::var("MODELPORT_ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_owned()),
        )?;
        let email = validate_email(
            &env::var("MODELPORT_ADMIN_EMAIL")
                .unwrap_or_else(|_| "admin@modelport.local".to_owned()),
        )?;
        let now_ms = now_millis();
        let user = AdminUserRecord {
            id: format!("usr_{}", Uuid::new_v4().simple()),
            username,
            email,
            role: "admin".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password(&password)?,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            last_login_at_ms: None,
        };

        inner.users.insert(user.id.clone(), user);
        self.save_locked(&inner)
    }

    fn save_locked(&self, inner: &AuthInner) -> Result<(), AppError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = path.with_extension("json.tmp");
        let file = AuthFile {
            users: inner.users.values().cloned().collect(),
        };
        fs::write(&tmp_path, serde_json::to_string_pretty(&file)?)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    fn prune_expired_sessions_locked(&self, inner: &mut AuthInner, now_ms: u64) {
        inner
            .sessions
            .retain(|_, session| session.expires_at_ms > now_ms);
    }
}

fn public_user(user: &AdminUserRecord, api_key_count: u32, request_count_24h: u64) -> PublicUser {
    PublicUser {
        id: user.id.clone(),
        username: user.username.clone(),
        email: user.email.clone(),
        role: user.role.clone(),
        status: user.status.clone(),
        created_at: user.created_at_ms.to_string(),
        last_login_at: user.last_login_at_ms.map(|value| value.to_string()),
        api_key_count,
        request_count_24h,
    }
}

fn normalize_username(value: &str) -> Result<String, AppError> {
    let username = value.trim().to_owned();
    if username.len() < 3 || username.len() > 64 {
        return Err(AppError::InvalidRequest(
            "username must be 3-64 characters".to_owned(),
        ));
    }
    if !username
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
    {
        return Err(AppError::InvalidRequest(
            "username may only contain letters, numbers, dots, underscores, and hyphens".to_owned(),
        ));
    }
    Ok(username)
}

fn validate_email(value: &str) -> Result<String, AppError> {
    let email = value.trim().to_owned();
    if email.len() > 254 || !email.contains('@') {
        return Err(AppError::InvalidRequest("invalid email address".to_owned()));
    }
    Ok(email)
}

fn validate_role(value: &str) -> Result<String, AppError> {
    match value {
        "admin" | "user" | "viewer" => Ok(value.to_owned()),
        _ => Err(AppError::InvalidRequest("invalid user role".to_owned())),
    }
}

fn validate_status(value: &str) -> Result<String, AppError> {
    match value {
        "active" | "disabled" | "suspended" => Ok(value.to_owned()),
        _ => Err(AppError::InvalidRequest("invalid user status".to_owned())),
    }
}

fn validate_password_strength(password: &str) -> Result<(), AppError> {
    if password.len() < 12 {
        return Err(AppError::InvalidRequest(
            "password must be at least 12 characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_bootstrap_password(password: &str) -> Result<(), AppError> {
    if password == "admin" {
        return Ok(());
    }
    validate_password_strength(password)
}

fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AppError::Config(format!("failed to hash admin password: {err}")))
}

fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

fn record_failed_attempt(attempt: &mut LoginAttempt, now_ms: u64) {
    attempt.failed_count = attempt.failed_count.saturating_add(1);
    if attempt.failed_count >= MAX_FAILED_ATTEMPTS {
        attempt.locked_until_ms = now_ms.saturating_add(LOCKOUT_SECONDS.saturating_mul(1_000));
        attempt.failed_count = 0;
    }
}

fn session_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    if let Some(token) = headers
        .get("x-admin-session")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
    {
        return Some(token);
    }

    headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == ADMIN_SESSION_COOKIE && !value.is_empty()).then_some(value)
            })
        })
}

fn new_session_token() -> String {
    format!("mps_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn hash_session_token(token: &str) -> String {
    let hash = Sha256::digest(token.as_bytes());
    let mut output = String::with_capacity(hash.len() * 2);
    for byte in hash {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn auth_store_path() -> PathBuf {
    if let Ok(path) = env::var("MODELPORT_AUTH_STORE_PATH") {
        return PathBuf::from(path);
    }
    env::var("MODELPORT_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".modelport"))
        .join("admin-auth.json")
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_sets_and_validates_session_cookie() {
        let store = AuthStore::for_tests();
        let user = AdminUserRecord {
            id: "usr_test".to_owned(),
            username: "admin".to_owned(),
            email: "admin@modelport.local".to_owned(),
            role: "admin".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("strong-password-123").unwrap(),
            created_at_ms: now_millis(),
            updated_at_ms: now_millis(),
            last_login_at_ms: None,
        };
        store
            .inner
            .lock()
            .unwrap()
            .users
            .insert(user.id.clone(), user);

        let login = store
            .login(LoginInput {
                username: "admin".to_owned(),
                password: "strong-password-123".to_owned(),
            })
            .unwrap();
        let cookie = store.session_cookie(&login.session_token);
        let mut headers = HeaderMap::new();
        headers.insert("cookie", cookie.parse().unwrap());

        let current = store.require_session(&headers).unwrap();
        assert_eq!(current.username, "admin");
    }

    #[test]
    fn bootstrap_accepts_local_admin_password() {
        validate_bootstrap_password("admin").unwrap();
    }

    #[test]
    fn normal_user_passwords_still_require_minimum_length() {
        assert!(validate_password_strength("admin").is_err());
    }
}
