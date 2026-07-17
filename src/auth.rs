use std::{
    collections::{BTreeMap, HashMap},
    env,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::http::HeaderMap;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    config::{AppConfig, env_flag},
    error::AppError,
    storage::JsonStore,
};

pub const ADMIN_SESSION_COOKIE: &str = "modelport_admin_session";

const DEFAULT_SESSION_TTL_SECONDS: u64 = 12 * 60 * 60;
const MAX_FAILED_ATTEMPTS: u32 = 5;
const LOCKOUT_SECONDS: u64 = 15 * 60;
const MAX_PASSWORD_BYTES: usize = 1_024;
const LOGIN_ATTEMPT_RETENTION_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_LOGIN_ATTEMPT_RECORDS: usize = 10_000;

pub struct AuthStore {
    store: Option<JsonStore>,
    inner: Mutex<AuthInner>,
    persistence_degraded: AtomicBool,
    session_ttl_seconds: u64,
    cookie_secure: bool,
}

#[derive(Debug, Clone, Default)]
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
    #[serde(default)]
    federated_identities: Vec<FederatedIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FederatedIdentity {
    issuer: String,
    subject: String,
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
    last_attempt_ms: u64,
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

pub struct LoginResult {
    pub session_token: String,
    pub expires_at_ms: u64,
    pub user: PublicUser,
}

pub struct FederatedLoginInput {
    pub issuer: String,
    pub subject: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub email_verified: bool,
    pub auto_provision: bool,
}

impl std::fmt::Debug for FederatedLoginInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FederatedLoginInput")
            .field("issuer", &self.issuer)
            .field("subject", &"[redacted]")
            .field("username_present", &self.username.is_some())
            .field("email_present", &self.email.is_some())
            .field("email_verified", &self.email_verified)
            .field("auto_provision", &self.auto_provision)
            .finish()
    }
}

impl std::fmt::Debug for AuthStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthStore")
            .field("data_path", &self.data_path())
            .field("session_ttl_seconds", &self.session_ttl_seconds)
            .field("cookie_secure", &self.cookie_secure)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for LoginResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginResult")
            .field("session_token", &"[redacted]")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("user", &self.user)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserInput {
    pub username: String,
    pub email: String,
    pub password: String,
    pub role: Option<String>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserInput {
    pub email: Option<String>,
    pub password: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
}

impl std::fmt::Debug for LoginInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginInput")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

impl std::fmt::Debug for CreateUserInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateUserInput")
            .field("username", &self.username)
            .field("email", &self.email)
            .field("password", &"[redacted]")
            .field("role", &self.role)
            .field("status", &self.status)
            .finish()
    }
}

impl std::fmt::Debug for UpdateUserInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateUserInput")
            .field("email", &self.email)
            .field("password_present", &self.password.is_some())
            .field("role", &self.role)
            .field("status", &self.status)
            .finish()
    }
}

impl AuthStore {
    pub fn load_or_bootstrap(config: &AppConfig) -> Result<Self, AppError> {
        let path = auth_store_path();
        let store = JsonStore::open("auth", path)?;
        let session_ttl_seconds = env_u64(
            "MODELPORT_ADMIN_SESSION_TTL_SECONDS",
            DEFAULT_SESSION_TTL_SECONDS,
        );
        let cookie_secure = env_flag("MODELPORT_ADMIN_COOKIE_SECURE");
        let file: AuthFile = store.read_or_default(serde_json::json!({ "users": [] }))?;
        let users = file
            .users
            .into_iter()
            .map(|user| (user.id.clone(), user))
            .collect();

        let store = Self {
            store: Some(store),
            inner: Mutex::new(AuthInner {
                users,
                sessions: HashMap::new(),
                attempts: HashMap::new(),
            }),
            persistence_degraded: AtomicBool::new(false),
            session_ttl_seconds,
            cookie_secure,
        };

        store.bootstrap_first_admin(config)?;
        Ok(store)
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            store: None,
            inner: Mutex::new(AuthInner::default()),
            persistence_degraded: AtomicBool::new(false),
            session_ttl_seconds: DEFAULT_SESSION_TTL_SECONDS,
            cookie_secure: false,
        }
    }

    pub fn login(&self, input: LoginInput) -> Result<LoginResult, AppError> {
        let username = normalize_username(&input.username)?;
        if input.password.is_empty() || input.password.len() > MAX_PASSWORD_BYTES {
            return Err(AppError::Auth);
        }

        let now_ms = now_millis();
        let candidate = {
            let mut inner = self.inner.lock().expect("auth lock poisoned");
            self.prune_expired_sessions_locked(&mut inner, now_ms);
            prune_login_attempts(&mut inner.attempts, now_ms);

            if inner
                .attempts
                .get(&username)
                .is_some_and(|attempt| attempt.locked_until_ms > now_ms)
            {
                return Err(AppError::Auth);
            }

            inner
                .users
                .iter()
                .find(|(_, user)| user.username.eq_ignore_ascii_case(&username))
                .map(|(id, user)| (id.clone(), user.clone()))
        };

        // Argon2 is intentionally expensive. Do not hold the auth-state mutex while
        // verifying a password, otherwise one login can stall every session lookup.
        let verified = match candidate.as_ref() {
            Some((_, user)) => {
                let password_valid = verify_password(&input.password, &user.password_hash);
                user.status == "active" && password_valid
            }
            None => {
                // Keep unknown-user attempts in the same expensive class as a
                // real password check so response timing does not enumerate users.
                let _ = hash_password(&input.password);
                false
            }
        };

        let mut inner = self.inner.lock().expect("auth lock poisoned");
        self.prune_expired_sessions_locked(&mut inner, now_ms);
        let Some((user_id, candidate_user)) = candidate else {
            record_failed_login(&mut inner.attempts, &username, now_ms);
            return Err(AppError::Auth);
        };
        let candidate_is_current = inner.users.get(&user_id).is_some_and(|user| {
            user.status == "active"
                && user.username.eq_ignore_ascii_case(&username)
                && user.password_hash == candidate_user.password_hash
        });
        if !verified || !candidate_is_current {
            record_failed_login(&mut inner.attempts, &username, now_ms);
            return Err(AppError::Auth);
        }

        if inner
            .attempts
            .get(&username)
            .is_some_and(|attempt| attempt.locked_until_ms > now_ms)
        {
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

    /// Resolves a verified OIDC identity to a local user and issues the same
    /// first-party session used by password login. The issuer and subject must
    /// already have been cryptographically verified by the OIDC client.
    pub fn login_federated(&self, input: FederatedLoginInput) -> Result<LoginResult, AppError> {
        let issuer = validate_federated_identifier("issuer", &input.issuer, 2_048)?;
        let subject = validate_federated_identifier("subject", &input.subject, 512)?;
        let now_ms = now_millis();
        let mut inner = self.inner.lock().expect("auth lock poisoned");
        self.prune_expired_sessions_locked(&mut inner, now_ms);
        if let Some(user_id) = bound_user_id(&inner, &issuer, &subject)? {
            let previous = inner.clone();
            return self.finish_federated_login_locked(&mut inner, previous, &user_id, now_ms);
        }

        // Email is the only mutable claim permitted for an implicit first
        // binding, and only when the provider asserts that it is verified.
        if !input.email_verified {
            return Err(AppError::Auth);
        }
        let email = input
            .email
            .as_deref()
            .ok_or(AppError::Auth)
            .and_then(|email| validate_email(email).map_err(|_| AppError::Auth))?;
        let email_matches = inner
            .users
            .iter()
            .filter(|(_, user)| user.email.eq_ignore_ascii_case(&email))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        if email_matches.len() > 1 {
            return Err(AppError::Auth);
        }
        if let Some(user_id) = email_matches.into_iter().next() {
            let user = inner.users.get(&user_id).ok_or_else(|| {
                AppError::Config("OIDC email-matched user disappeared".to_owned())
            })?;
            // Existing admins, inactive accounts, and accounts already bound
            // to any identity require an explicit administrative binding.
            if user.role == "admin"
                || user.status != "active"
                || !user.federated_identities.is_empty()
            {
                return Err(AppError::Auth);
            }
            let previous = inner.clone();
            inner
                .users
                .get_mut(&user_id)
                .expect("email-matched OIDC user should still exist")
                .federated_identities
                .push(FederatedIdentity {
                    issuer: issuer.clone(),
                    subject: subject.clone(),
                });
            return self.finish_federated_login_locked(&mut inner, previous, &user_id, now_ms);
        }
        if !input.auto_provision {
            return Err(AppError::Auth);
        }
        let username = input
            .username
            .as_deref()
            .ok_or(AppError::Auth)
            .and_then(|username| normalize_username(username).map_err(|_| AppError::Auth))?;
        if inner.users.values().any(|user| {
            user.username.eq_ignore_ascii_case(&username) || user.email.eq_ignore_ascii_case(&email)
        }) {
            return Err(AppError::Auth);
        }

        // Argon2 is intentionally expensive. Release the global auth mutex
        // before hashing the unusable random local password for a JIT user.
        drop(inner);
        let random_local_secret = format!(
            "{}{}{}",
            new_session_token(),
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let password_hash = hash_password(&random_local_secret)?;

        let mut inner = self.inner.lock().expect("auth lock poisoned");
        self.prune_expired_sessions_locked(&mut inner, now_ms);
        // Resolve races with another callback for the same identity without
        // ever creating a shadow username or email.
        if let Some(user_id) = bound_user_id(&inner, &issuer, &subject)? {
            let previous = inner.clone();
            return self.finish_federated_login_locked(&mut inner, previous, &user_id, now_ms);
        }
        if inner.users.values().any(|user| {
            user.username.eq_ignore_ascii_case(&username) || user.email.eq_ignore_ascii_case(&email)
        }) {
            return Err(AppError::Auth);
        }
        let user = AdminUserRecord {
            id: format!("usr_{}", Uuid::new_v4().simple()),
            username,
            email,
            role: "user".to_owned(),
            status: "active".to_owned(),
            password_hash,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            last_login_at_ms: None,
            federated_identities: vec![FederatedIdentity { issuer, subject }],
        };
        let user_id = user.id.clone();
        let previous = inner.clone();
        inner.users.insert(user_id.clone(), user);
        self.finish_federated_login_locked(&mut inner, previous, &user_id, now_ms)
    }

    fn finish_federated_login_locked(
        &self,
        inner: &mut AuthInner,
        previous: AuthInner,
        user_id: &str,
        now_ms: u64,
    ) -> Result<LoginResult, AppError> {
        let Some(user) = inner.users.get_mut(user_id) else {
            return Err(AppError::Config("OIDC user disappeared".to_owned()));
        };
        if user.status != "active" {
            return Err(AppError::Auth);
        }
        user.last_login_at_ms = Some(now_ms);
        user.updated_at_ms = now_ms;

        let token = new_session_token();
        let expires_at_ms = now_ms.saturating_add(self.session_ttl_seconds.saturating_mul(1_000));
        inner.sessions.insert(
            hash_session_token(&token),
            AdminSession {
                user_id: user_id.to_owned(),
                expires_at_ms,
            },
        );
        if let Err(error) = self.save_locked(inner) {
            *inner = previous;
            return Err(error);
        }
        let user = inner
            .users
            .get(user_id)
            .map(|user| public_user(user, 1, 0))
            .ok_or_else(|| AppError::Config("OIDC user disappeared".to_owned()))?;
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

    pub fn user_by_id(&self, user_id: &str) -> Option<PublicUser> {
        let inner = self.inner.lock().expect("auth lock poisoned");
        inner
            .users
            .get(user_id.trim())
            .map(|user| public_user(user, 0, 0))
    }

    pub fn is_user_active(&self, user_id: &str) -> bool {
        let inner = self.inner.lock().expect("auth lock poisoned");
        inner
            .users
            .get(user_id.trim())
            .is_some_and(|user| user.status == "active")
    }

    pub fn active_admin_count(&self) -> usize {
        let inner = self.inner.lock().expect("auth lock poisoned");
        inner
            .users
            .values()
            .filter(|user| user.role == "admin" && user.status == "active")
            .count()
    }

    pub fn data_path(&self) -> Option<String> {
        self.store.as_ref().map(JsonStore::location)
    }

    pub fn health_check(&self) -> Result<(), AppError> {
        if self.persistence_degraded.load(Ordering::Acquire) {
            return Err(AppError::NotReady(
                "auth persistence is degraded after a failed write".to_owned(),
            ));
        }
        self.store
            .as_ref()
            .map(JsonStore::read_value)
            .transpose()
            .map(|_| ())
    }

    pub fn default_data_path() -> PathBuf {
        auth_store_path()
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
            federated_identities: Vec::new(),
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
        let previous = inner.clone();
        inner.users.insert(user.id.clone(), user);
        self.save_or_restore_locked(&mut inner, previous)?;
        Ok(public)
    }

    pub fn update_user(
        &self,
        user_id: &str,
        current_user_id: &str,
        input: UpdateUserInput,
    ) -> Result<PublicUser, AppError> {
        let email = input.email.as_deref().map(validate_email).transpose()?;
        let role = input.role.as_deref().map(validate_role).transpose()?;
        let status = input.status.as_deref().map(validate_status).transpose()?;
        let password_hash = input
            .password
            .as_deref()
            .filter(|password| !password.is_empty())
            .map(|password| {
                validate_password_strength(password)?;
                hash_password(password)
            })
            .transpose()?;

        let mut inner = self.inner.lock().expect("auth lock poisoned");
        let Some(existing) = inner.users.get(user_id).cloned() else {
            return Err(AppError::InvalidRequest("user not found".to_owned()));
        };

        if user_id == current_user_id
            && (role.as_deref().is_some_and(|next| next != "admin")
                || status.as_deref().is_some_and(|next| next != "active"))
        {
            return Err(AppError::Forbidden(
                "cannot remove admin access from the current signed-in user".to_owned(),
            ));
        }

        let next_role = role.unwrap_or_else(|| existing.role.clone());
        let next_status = status.unwrap_or_else(|| existing.status.clone());
        let active_admins_after = inner
            .users
            .iter()
            .filter(|(id, user)| {
                if id.as_str() == user_id {
                    next_role == "admin" && next_status == "active"
                } else {
                    user.role == "admin" && user.status == "active"
                }
            })
            .count();
        if active_admins_after == 0 {
            return Err(AppError::Forbidden(
                "cannot remove the last active admin".to_owned(),
            ));
        }

        let previous = inner.clone();
        let should_clear_sessions = password_hash.is_some() || next_status != "active";
        let public = {
            let Some(user) = inner.users.get_mut(user_id) else {
                return Err(AppError::InvalidRequest("user not found".to_owned()));
            };
            if let Some(email) = email {
                user.email = email;
            }
            user.role = next_role;
            user.status = next_status;
            if let Some(password_hash) = password_hash {
                user.password_hash = password_hash;
            }
            user.updated_at_ms = now_millis();
            public_user(user, 1, 0)
        };
        if should_clear_sessions {
            inner
                .sessions
                .retain(|_, session| session.user_id.as_str() != user_id);
        }
        self.save_or_restore_locked(&mut inner, previous)?;
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

        let previous = inner.clone();
        inner.users.remove(user_id);
        inner
            .sessions
            .retain(|_, session| session.user_id.as_str() != user_id);
        self.save_or_restore_locked(&mut inner, previous)
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
            federated_identities: Vec::new(),
        };

        inner.users.insert(user.id.clone(), user);
        self.save_locked(&inner)
    }

    fn save_or_restore_locked(
        &self,
        inner: &mut AuthInner,
        previous: AuthInner,
    ) -> Result<(), AppError> {
        if let Err(error) = self.save_locked(inner) {
            *inner = previous;
            return Err(error);
        }
        Ok(())
    }

    fn save_locked(&self, inner: &AuthInner) -> Result<(), AppError> {
        let result = if let Some(store) = &self.store {
            let file = AuthFile {
                users: inner.users.values().cloned().collect(),
            };
            store.write_json(&file)
        } else {
            Ok(())
        };
        self.persistence_degraded
            .store(result.is_err(), Ordering::Release);
        result
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

fn bound_user_id(
    inner: &AuthInner,
    issuer: &str,
    subject: &str,
) -> Result<Option<String>, AppError> {
    let mut matches = inner.users.iter().filter(|(_, user)| {
        user.federated_identities
            .iter()
            .any(|identity| identity.issuer == issuer && identity.subject == subject)
    });
    let first = matches.next().map(|(id, _)| id.clone());
    if matches.next().is_some() {
        return Err(AppError::Config(
            "duplicate federated identity binding in auth store".to_owned(),
        ));
    }
    Ok(first)
}

pub(crate) fn validate_backup_document(value: &Value) -> Result<(), AppError> {
    let file: AuthFile = serde_json::from_value(value.clone()).map_err(|error| {
        AppError::InvalidRequest(format!("backup auth document is invalid: {error}"))
    })?;
    let mut ids = std::collections::BTreeSet::new();
    let mut usernames = std::collections::BTreeSet::new();
    let mut federated_identities = std::collections::BTreeSet::new();
    let mut active_admins = 0usize;

    for user in &file.users {
        if user.id.trim().is_empty() || !ids.insert(user.id.clone()) {
            return Err(AppError::InvalidRequest(
                "backup auth document contains an empty or duplicate user id".to_owned(),
            ));
        }
        let username = normalize_username(&user.username)?;
        if !usernames.insert(username.to_ascii_lowercase()) {
            return Err(AppError::InvalidRequest(
                "backup auth document contains duplicate usernames".to_owned(),
            ));
        }
        validate_email(&user.email)?;
        validate_role(&user.role)?;
        validate_status(&user.status)?;
        let password_hash = PasswordHash::new(&user.password_hash).map_err(|_| {
            AppError::InvalidRequest(
                "backup auth document contains an invalid password hash".to_owned(),
            )
        })?;
        if !matches!(
            password_hash.algorithm.as_str(),
            "argon2id" | "argon2i" | "argon2d"
        ) {
            return Err(AppError::InvalidRequest(
                "backup auth document contains an unsupported password hash".to_owned(),
            ));
        }
        for identity in &user.federated_identities {
            let issuer = validate_federated_identifier("issuer", &identity.issuer, 2_048)?;
            let subject = validate_federated_identifier("subject", &identity.subject, 512)?;
            if !federated_identities.insert((issuer, subject)) {
                return Err(AppError::InvalidRequest(
                    "backup auth document contains a duplicate federated identity".to_owned(),
                ));
            }
        }
        if user.role == "admin" && user.status == "active" {
            active_admins += 1;
        }
    }

    if !file.users.is_empty() && active_admins == 0 {
        return Err(AppError::InvalidRequest(
            "backup auth document must retain at least one active admin".to_owned(),
        ));
    }
    Ok(())
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

fn validate_federated_identifier(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(AppError::InvalidRequest(format!(
            "invalid federated {field}"
        )));
    }
    Ok(value.to_owned())
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
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(AppError::InvalidRequest(format!(
            "password must be at most {MAX_PASSWORD_BYTES} bytes"
        )));
    }
    if password.chars().count() < 12 {
        return Err(AppError::InvalidRequest(
            "password must be at least 12 characters".to_owned(),
        ));
    }

    let normalized = password.trim().to_ascii_lowercase();
    let has_common_prefix = [
        "admin",
        "password",
        "modelport",
        "letmein",
        "qwerty",
        "example-password",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix));
    let is_placeholder = matches!(
        normalized.as_str(),
        "administrator"
            | "password123"
            | "password1234"
            | "modelport123"
            | "modelport-admin"
            | "letmein12345"
            | "change-me-now"
    ) || normalized.starts_with("replace-with-")
        || normalized.starts_with("your-password")
        || normalized.contains("placeholder")
        || normalized.contains("change-me")
        || normalized.contains("changeme");
    let only_digits = normalized
        .chars()
        .all(|character| character.is_ascii_digit());
    let distinct_characters = password
        .chars()
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    if normalized.is_empty()
        || has_common_prefix
        || is_placeholder
        || only_digits
        || distinct_characters < 6
    {
        return Err(AppError::InvalidRequest(
            "password is a common placeholder or is too weak".to_owned(),
        ));
    }

    Ok(())
}

fn validate_bootstrap_password(password: &str) -> Result<(), AppError> {
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
    attempt.last_attempt_ms = now_ms;
    attempt.failed_count = attempt.failed_count.saturating_add(1);
    if attempt.failed_count >= MAX_FAILED_ATTEMPTS {
        attempt.locked_until_ms = now_ms.saturating_add(LOCKOUT_SECONDS.saturating_mul(1_000));
        attempt.failed_count = 0;
    }
}

fn record_failed_login(attempts: &mut HashMap<String, LoginAttempt>, username: &str, now_ms: u64) {
    if !attempts.contains_key(username)
        && attempts.len() >= MAX_LOGIN_ATTEMPT_RECORDS
        && let Some(oldest) = attempts
            .iter()
            .min_by_key(|(_, attempt)| attempt.last_attempt_ms)
            .map(|(username, _)| username.clone())
    {
        attempts.remove(&oldest);
    }
    record_failed_attempt(attempts.entry(username.to_owned()).or_default(), now_ms);
}

fn prune_login_attempts(attempts: &mut HashMap<String, LoginAttempt>, now_ms: u64) {
    attempts.retain(|_, attempt| {
        attempt.locked_until_ms > now_ms
            || attempt
                .last_attempt_ms
                .saturating_add(LOGIN_ATTEMPT_RETENTION_MS)
                > now_ms
    });
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

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failing_store_path(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "modelport-{label}-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

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
            federated_identities: Vec::new(),
        };
        store
            .inner
            .lock()
            .unwrap()
            .users
            .insert(user.id.clone(), user);

        let input = LoginInput {
            username: "admin".to_owned(),
            password: "strong-password-123".to_owned(),
        };
        assert!(!format!("{input:?}").contains("strong-password-123"));
        let login = store.login(input).unwrap();
        assert!(!format!("{login:?}").contains(&login.session_token));
        let cookie = store.session_cookie(&login.session_token);
        let mut headers = HeaderMap::new();
        headers.insert("cookie", cookie.parse().unwrap());

        let current = store.require_session(&headers).unwrap();
        assert_eq!(current.username, "admin");
    }

    #[test]
    fn federated_login_requires_verified_email_and_never_implicitly_binds_admin() {
        let store = AuthStore::for_tests();
        let now = now_millis();
        let user = AdminUserRecord {
            id: "usr_user".to_owned(),
            username: "developer".to_owned(),
            email: "developer@example.com".to_owned(),
            role: "user".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("strong-password-123").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        let admin = AdminUserRecord {
            id: "usr_admin".to_owned(),
            username: "operator".to_owned(),
            email: "operator@example.com".to_owned(),
            role: "admin".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("another-strong-password-123").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        {
            let mut inner = store.inner.lock().unwrap();
            inner.users.insert(user.id.clone(), user);
            inner.users.insert(admin.id.clone(), admin);
        }

        let unverified = store.login_federated(FederatedLoginInput {
            issuer: "https://id.example.com".to_owned(),
            subject: "subject-user".to_owned(),
            username: Some("developer".to_owned()),
            email: Some("developer@example.com".to_owned()),
            email_verified: false,
            auto_provision: false,
        });
        assert!(matches!(unverified, Err(AppError::Auth)));

        let login = store
            .login_federated(FederatedLoginInput {
                issuer: "https://id.example.com".to_owned(),
                subject: "subject-user".to_owned(),
                username: Some("ignored-username".to_owned()),
                email: Some("DEVELOPER@example.com".to_owned()),
                email_verified: true,
                auto_provision: false,
            })
            .unwrap();
        assert_eq!(login.user.id, "usr_user");

        let admin_login = store.login_federated(FederatedLoginInput {
            issuer: "https://id.example.com".to_owned(),
            subject: "subject-admin".to_owned(),
            username: Some("operator".to_owned()),
            email: Some("operator@example.com".to_owned()),
            email_verified: true,
            auto_provision: true,
        });
        assert!(matches!(admin_login, Err(AppError::Auth)));
        let inner = store.inner.lock().unwrap();
        assert!(inner.users["usr_admin"].federated_identities.is_empty());
    }

    #[test]
    fn bound_federated_identity_ignores_changed_or_malformed_optional_claims() {
        let store = AuthStore::for_tests();
        let now = now_millis();
        let user = AdminUserRecord {
            id: "usr_bound".to_owned(),
            username: "bound-user".to_owned(),
            email: "bound@example.com".to_owned(),
            role: "user".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("strong-password-123").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: vec![FederatedIdentity {
                issuer: "https://id.example.com".to_owned(),
                subject: "stable-subject".to_owned(),
            }],
        };
        store
            .inner
            .lock()
            .unwrap()
            .users
            .insert(user.id.clone(), user);

        let login = store
            .login_federated(FederatedLoginInput {
                issuer: "https://id.example.com".to_owned(),
                subject: "stable-subject".to_owned(),
                username: Some("not a valid local username".to_owned()),
                email: Some("not-an-email".to_owned()),
                email_verified: false,
                auto_provision: false,
            })
            .unwrap();
        assert_eq!(login.user.id, "usr_bound");
    }

    #[test]
    fn federated_auto_provision_is_user_only_and_rejects_shadow_username() {
        let store = AuthStore::for_tests();
        store
            .create_user(CreateUserInput {
                username: "existing-user".to_owned(),
                email: "existing@example.com".to_owned(),
                password: "strong-existing-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            })
            .unwrap();
        let collision = store.login_federated(FederatedLoginInput {
            issuer: "https://id.example.com".to_owned(),
            subject: "collision-subject".to_owned(),
            username: Some("EXISTING-USER".to_owned()),
            email: Some("different@example.com".to_owned()),
            email_verified: true,
            auto_provision: true,
        });
        assert!(matches!(collision, Err(AppError::Auth)));

        let login = store
            .login_federated(FederatedLoginInput {
                issuer: "https://id.example.com".to_owned(),
                subject: "new-subject".to_owned(),
                username: Some("new-user".to_owned()),
                email: Some("new@example.com".to_owned()),
                email_verified: true,
                auto_provision: true,
            })
            .unwrap();
        assert_eq!(login.user.role, "user");
        assert_eq!(login.user.status, "active");
        let inner = store.inner.lock().unwrap();
        let record = inner.users.get(&login.user.id).unwrap();
        assert_eq!(record.federated_identities.len(), 1);
        assert!(PasswordHash::new(&record.password_hash).is_ok());
    }

    #[test]
    fn bootstrap_rejects_weak_and_placeholder_passwords() {
        for password in [
            "admin",
            "password1234",
            "123456789012",
            "aaaaaaaaaaaa",
            "replace-with-a-long-random-admin-password",
            "this-is-a-placeholder-password",
        ] {
            assert!(
                validate_bootstrap_password(password).is_err(),
                "password should be rejected: {password}"
            );
        }
    }

    #[test]
    fn backup_validation_rejects_invalid_hashes_and_missing_active_admin() {
        let invalid_hash = serde_json::json!({
            "users": [{
                "id": "usr_admin",
                "username": "operator",
                "email": "operator@example.com",
                "role": "admin",
                "status": "active",
                "password_hash": "not-a-password-hash",
                "created_at_ms": 1,
                "updated_at_ms": 1,
                "last_login_at_ms": null
            }]
        });
        assert!(validate_backup_document(&invalid_hash).is_err());

        let no_active_admin = serde_json::json!({
            "users": [{
                "id": "usr_viewer",
                "username": "viewer",
                "email": "viewer@example.com",
                "role": "viewer",
                "status": "active",
                "password_hash": hash_password("strong-password-123").unwrap(),
                "created_at_ms": 1,
                "updated_at_ms": 1,
                "last_login_at_ms": null
            }]
        });
        assert!(validate_backup_document(&no_active_admin).is_err());

        let shared_identity = serde_json::json!({
            "users": [
                {
                    "id": "usr_admin",
                    "username": "operator",
                    "email": "operator@example.com",
                    "role": "admin",
                    "status": "active",
                    "password_hash": hash_password("strong-password-123").unwrap(),
                    "created_at_ms": 1,
                    "updated_at_ms": 1,
                    "last_login_at_ms": null,
                    "federated_identities": [{
                        "issuer": "https://id.example.com",
                        "subject": "duplicate-subject"
                    }]
                },
                {
                    "id": "usr_user",
                    "username": "developer",
                    "email": "developer@example.com",
                    "role": "user",
                    "status": "active",
                    "password_hash": hash_password("another-strong-password-123").unwrap(),
                    "created_at_ms": 1,
                    "updated_at_ms": 1,
                    "last_login_at_ms": null,
                    "federated_identities": [{
                        "issuer": "https://id.example.com",
                        "subject": "duplicate-subject"
                    }]
                }
            ]
        });
        assert!(validate_backup_document(&shared_identity).is_err());
    }

    #[test]
    fn strong_passwords_are_accepted_and_short_passwords_are_rejected() {
        validate_bootstrap_password("strong-password-123").unwrap();
        assert!(validate_password_strength("admin").is_err());
    }

    #[test]
    fn user_lookup_reports_the_current_status() {
        let store = AuthStore::for_tests();
        let user = store
            .create_user(CreateUserInput {
                username: "disabled-user".to_owned(),
                email: "disabled@example.com".to_owned(),
                password: "strong-disabled-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("disabled".to_owned()),
            })
            .unwrap();

        assert_eq!(
            store.user_by_id(&user.id).map(|user| user.username),
            Some("disabled-user".to_owned())
        );
        assert!(!store.is_user_active(&user.id));
        assert!(store.user_by_id("usr_missing").is_none());
        assert!(!store.is_user_active("usr_missing"));
    }

    #[test]
    fn admin_updates_user_profile_role_and_password() {
        let store = AuthStore::for_tests();
        let now = now_millis();
        let admin = AdminUserRecord {
            id: "usr_admin".to_owned(),
            username: "admin".to_owned(),
            email: "admin@modelport.local".to_owned(),
            role: "admin".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("strong-password-123").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        let user = AdminUserRecord {
            id: "usr_user".to_owned(),
            username: "dev".to_owned(),
            email: "dev@modelport.local".to_owned(),
            role: "user".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("old-password-123").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        {
            let mut inner = store.inner.lock().unwrap();
            inner.users.insert(admin.id.clone(), admin);
            inner.users.insert(user.id.clone(), user);
        }

        let updated = store
            .update_user(
                "usr_user",
                "usr_admin",
                UpdateUserInput {
                    email: Some("devops@modelport.local".to_owned()),
                    password: Some("new-password-123".to_owned()),
                    role: Some("viewer".to_owned()),
                    status: Some("disabled".to_owned()),
                },
            )
            .unwrap();

        assert_eq!(updated.email, "devops@modelport.local");
        assert_eq!(updated.role, "viewer");
        assert_eq!(updated.status, "disabled");
        let inner = store.inner.lock().unwrap();
        let user = inner.users.get("usr_user").unwrap();
        assert!(verify_password("new-password-123", &user.password_hash));
    }

    #[test]
    fn update_user_keeps_current_admin_access() {
        let store = AuthStore::for_tests();
        let now = now_millis();
        let admin = AdminUserRecord {
            id: "usr_admin".to_owned(),
            username: "admin".to_owned(),
            email: "admin@modelport.local".to_owned(),
            role: "admin".to_owned(),
            status: "active".to_owned(),
            password_hash: hash_password("strong-password-123").unwrap(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        store
            .inner
            .lock()
            .unwrap()
            .users
            .insert(admin.id.clone(), admin);

        let err = store
            .update_user(
                "usr_admin",
                "usr_admin",
                UpdateUserInput {
                    email: None,
                    password: None,
                    role: Some("user".to_owned()),
                    status: Some("active".to_owned()),
                },
            )
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn failed_user_writes_restore_memory_and_degrade_health() {
        let path = failing_store_path("auth-write-failure");
        let store = AuthStore {
            store: Some(JsonStore::File(path.clone())),
            inner: Mutex::new(AuthInner::default()),
            persistence_degraded: AtomicBool::new(false),
            session_ttl_seconds: DEFAULT_SESSION_TTL_SECONDS,
            cookie_secure: false,
        };

        assert!(matches!(
            store.create_user(CreateUserInput {
                username: "new-user".to_owned(),
                email: "new-user@example.com".to_owned(),
                password: "strong-new-user-password-123".to_owned(),
                role: Some("user".to_owned()),
                status: Some("active".to_owned()),
            }),
            Err(AppError::Io(_))
        ));
        assert!(store.inner.lock().unwrap().users.is_empty());

        let now = now_millis();
        let admin = AdminUserRecord {
            id: "usr_admin".to_owned(),
            username: "admin".to_owned(),
            email: "admin@example.com".to_owned(),
            role: "admin".to_owned(),
            status: "active".to_owned(),
            password_hash: "unused".to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        let user = AdminUserRecord {
            id: "usr_user".to_owned(),
            username: "user".to_owned(),
            email: "user@example.com".to_owned(),
            role: "user".to_owned(),
            status: "active".to_owned(),
            password_hash: "unused".to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
            last_login_at_ms: None,
            federated_identities: Vec::new(),
        };
        {
            let mut inner = store.inner.lock().unwrap();
            inner.users.insert(admin.id.clone(), admin);
            inner.users.insert(user.id.clone(), user);
            inner.sessions.insert(
                "session-user".to_owned(),
                AdminSession {
                    user_id: "usr_user".to_owned(),
                    expires_at_ms: now.saturating_add(60_000),
                },
            );
        }

        assert!(matches!(
            store.update_user(
                "usr_user",
                "usr_admin",
                UpdateUserInput {
                    email: Some("changed@example.com".to_owned()),
                    password: None,
                    role: Some("viewer".to_owned()),
                    status: Some("disabled".to_owned()),
                },
            ),
            Err(AppError::Io(_))
        ));
        {
            let inner = store.inner.lock().unwrap();
            let user = inner.users.get("usr_user").unwrap();
            assert_eq!(user.email, "user@example.com");
            assert_eq!(user.role, "user");
            assert_eq!(user.status, "active");
            assert!(inner.sessions.contains_key("session-user"));
        }

        assert!(matches!(
            store.delete_user("usr_user", "usr_admin"),
            Err(AppError::Io(_))
        ));
        {
            let inner = store.inner.lock().unwrap();
            assert!(inner.users.contains_key("usr_user"));
            assert!(inner.sessions.contains_key("session-user"));
        }
        assert!(matches!(store.health_check(), Err(AppError::NotReady(_))));

        std::fs::remove_dir_all(path).unwrap();

        let healthy = AuthStore::for_tests();
        healthy.persistence_degraded.store(true, Ordering::Release);
        healthy.save_locked(&AuthInner::default()).unwrap();
        assert!(healthy.health_check().is_ok());
    }
}
