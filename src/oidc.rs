use std::{
    collections::HashMap,
    future::Future,
    net::IpAddr,
    pin::Pin,
    sync::Mutex,
    time::{Duration, Instant},
};

use openidconnect::{
    AccessTokenHash, AdditionalClaims, AsyncHttpClient, AuthorizationCode, Client, ClientId,
    ClientSecret, CsrfToken, EmptyExtraTokenFields, EndpointMaybeSet, EndpointNotSet, EndpointSet,
    HttpClientError, HttpRequest, HttpResponse, IdTokenFields, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope,
    StandardErrorResponse, StandardTokenResponse, TokenResponse,
    core::{
        CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreErrorResponseType,
        CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm,
        CoreJwsSigningAlgorithm, CoreProviderMetadata, CoreRevocableToken,
        CoreRevocationErrorResponse, CoreTokenIntrospectionResponse, CoreTokenType,
    },
    reqwest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    config::{env_flag, env_value, is_placeholder_value},
    error::AppError,
};

pub const OIDC_START_URL: &str = "/admin/auth/oidc/start";
pub const OIDC_FLOW_COOKIE: &str = "modelport_oidc_flow";

const DEFAULT_LABEL: &str = "Single sign-on";
const DEFAULT_USERNAME_CLAIM: &str = "preferred_username";
const DEFAULT_EMAIL_CLAIM: &str = "email";
const PENDING_STATE_TTL: Duration = Duration::from_secs(10 * 60);
const METADATA_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_PENDING_STATES: usize = 1_024;
const MAX_RETURN_TO_BYTES: usize = 2_048;
const MAX_CALLBACK_VALUE_BYTES: usize = 8 * 1_024;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct DynamicClaims(HashMap<String, Value>);

impl AdditionalClaims for DynamicClaims {}

type OidcIdTokenFields = IdTokenFields<
    DynamicClaims,
    EmptyExtraTokenFields,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type OidcTokenResponse = StandardTokenResponse<OidcIdTokenFields, CoreTokenType>;
type OidcClient<
    HasAuthUrl = EndpointNotSet,
    HasDeviceAuthUrl = EndpointNotSet,
    HasIntrospectionUrl = EndpointNotSet,
    HasRevocationUrl = EndpointNotSet,
    HasTokenUrl = EndpointNotSet,
    HasUserInfoUrl = EndpointNotSet,
> = Client<
    DynamicClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    OidcTokenResponse,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
    HasUserInfoUrl,
>;
type ReadyOidcClient = OidcClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
    EndpointMaybeSet,
>;

pub struct OidcService {
    config: Option<OidcConfig>,
    pending: Mutex<HashMap<[u8; 32], PendingAuthorization>>,
    metadata_cache: Mutex<Option<CachedMetadata>>,
    http_client: SecureHttpClient,
    cookie_secure: bool,
}

#[derive(Clone)]
struct OidcConfig {
    issuer: String,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    label: String,
    auto_provision: bool,
    username_claim: String,
    email_claim: String,
    allow_insecure_loopback: bool,
}

struct PendingAuthorization {
    pkce_verifier: PkceCodeVerifier,
    nonce: Nonce,
    browser_flow_hash: [u8; 32],
    return_to: String,
    created_at: Instant,
}

struct CachedMetadata {
    metadata: CoreProviderMetadata,
    fetched_at: Instant,
}

#[derive(Clone)]
struct SecureHttpClient {
    inner: reqwest::Client,
    allow_insecure_loopback: bool,
}

#[derive(Debug, Error)]
enum SecureHttpError {
    #[error("OIDC endpoint URL is not allowed")]
    UrlNotAllowed,
    #[error("OIDC HTTP request failed")]
    Request(#[source] HttpClientError<reqwest::Error>),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationMethods {
    pub password_enabled: bool,
    pub oidc: OidcMethod,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcMethod {
    pub enabled: bool,
    pub label: String,
    pub start_url: &'static str,
}

pub struct CompletedOidcLogin {
    pub issuer: String,
    pub subject: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub email_verified: bool,
    pub auto_provision: bool,
    pub return_to: String,
}

pub struct OidcStart {
    pub authorization_url: String,
    pub flow_cookie: String,
}

impl std::fmt::Debug for CompletedOidcLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompletedOidcLogin")
            .field("issuer", &self.issuer)
            .field("subject", &"[redacted]")
            .field("username_present", &self.username.is_some())
            .field("email_present", &self.email.is_some())
            .field("email_verified", &self.email_verified)
            .field("auto_provision", &self.auto_provision)
            .field("return_to", &self.return_to)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum OidcFlowError {
    #[error("OIDC login is disabled")]
    Disabled,
    #[error("invalid return path")]
    InvalidReturnTo,
    #[error("invalid OIDC callback")]
    InvalidCallback,
    #[error("invalid or expired OIDC state")]
    InvalidState,
    #[error("OIDC provider is unavailable")]
    ProviderUnavailable,
    #[error("OIDC token exchange failed")]
    TokenExchangeFailed,
    #[error("OIDC ID token is invalid")]
    InvalidToken,
}

impl OidcFlowError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::InvalidReturnTo | Self::InvalidCallback => "invalid_callback",
            Self::InvalidState => "invalid_state",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::TokenExchangeFailed => "token_exchange_failed",
            Self::InvalidToken => "token_invalid",
        }
    }
}

impl std::fmt::Debug for OidcService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OidcService")
            .field("enabled", &self.config.is_some())
            .field("label", &self.config.as_ref().map(|config| &config.label))
            .finish_non_exhaustive()
    }
}

impl OidcService {
    pub fn from_env() -> Result<Self, AppError> {
        let config = OidcConfig::from_env()?;
        Self::new(config)
    }

    pub fn validate_configuration() -> Result<(), AppError> {
        OidcConfig::from_env().map(|_| ())
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self::new(None).expect("disabled OIDC service should always initialize")
    }

    fn new(config: Option<OidcConfig>) -> Result<Self, AppError> {
        let allow_insecure_loopback = config
            .as_ref()
            .is_some_and(|config| config.allow_insecure_loopback);
        let inner = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| AppError::Config("failed to initialize OIDC HTTP client".to_owned()))?;
        Ok(Self {
            config,
            pending: Mutex::new(HashMap::new()),
            metadata_cache: Mutex::new(None),
            http_client: SecureHttpClient {
                inner,
                allow_insecure_loopback,
            },
            cookie_secure: env_flag("MODELPORT_ADMIN_COOKIE_SECURE"),
        })
    }

    pub fn methods(&self) -> AuthenticationMethods {
        AuthenticationMethods {
            password_enabled: true,
            oidc: OidcMethod {
                enabled: self.config.is_some(),
                label: self
                    .config
                    .as_ref()
                    .map(|config| config.label.clone())
                    .unwrap_or_else(|| DEFAULT_LABEL.to_owned()),
                start_url: OIDC_START_URL,
            },
        }
    }

    pub async fn start(&self, return_to: Option<&str>) -> Result<OidcStart, OidcFlowError> {
        let config = self.config.as_ref().ok_or(OidcFlowError::Disabled)?;
        let return_to = validate_return_to(return_to.unwrap_or("/"))?;
        let client = self.ready_client(config).await?;
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (authorization_url, csrf_state, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("profile".to_owned()))
            .add_scope(Scope::new("email".to_owned()))
            .set_pkce_challenge(pkce_challenge)
            .url();
        let browser_flow = CsrfToken::new_random();

        self.insert_pending(
            hash_state(csrf_state.secret()),
            PendingAuthorization {
                pkce_verifier,
                nonce,
                browser_flow_hash: hash_state(browser_flow.secret()),
                return_to,
                created_at: Instant::now(),
            },
        );
        Ok(OidcStart {
            authorization_url: authorization_url.to_string(),
            flow_cookie: self.flow_cookie(browser_flow.secret()),
        })
    }

    pub async fn complete(
        &self,
        code: &str,
        state: &str,
        browser_flow: &str,
    ) -> Result<CompletedOidcLogin, OidcFlowError> {
        validate_callback_value(code)?;
        let pending = self.consume_pending(state, browser_flow)?;
        let config = self.config.as_ref().ok_or(OidcFlowError::Disabled)?;
        let client = self.ready_client(config).await?;
        let token_response = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(pending.pkce_verifier)
            .request_async(&self.http_client)
            .await
            .map_err(|_| OidcFlowError::TokenExchangeFailed)?;
        let id_token = token_response
            .id_token()
            .ok_or(OidcFlowError::InvalidToken)?;
        let verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&verifier, &pending.nonce)
            .map_err(|_| OidcFlowError::InvalidToken)?;

        if let Some(expected_hash) = claims.access_token_hash() {
            let actual_hash = AccessTokenHash::from_token(
                token_response.access_token(),
                id_token
                    .signing_alg()
                    .map_err(|_| OidcFlowError::InvalidToken)?,
                id_token
                    .signing_key(&verifier)
                    .map_err(|_| OidcFlowError::InvalidToken)?,
            )
            .map_err(|_| OidcFlowError::InvalidToken)?;
            if actual_hash != *expected_hash {
                return Err(OidcFlowError::InvalidToken);
            }
        }

        let claims_value = serde_json::to_value(claims).map_err(|_| OidcFlowError::InvalidToken)?;
        Ok(CompletedOidcLogin {
            issuer: config.issuer.clone(),
            subject: claims.subject().as_str().to_owned(),
            username: string_claim(&claims_value, &config.username_claim),
            email: string_claim(&claims_value, &config.email_claim),
            // OIDC's `email_verified` assertion applies specifically to the
            // standard `email` claim. A custom email-shaped claim must never
            // inherit that assertion for implicit local-account binding.
            email_verified: config.email_claim == "email"
                && claims.email_verified().unwrap_or(false),
            auto_provision: config.auto_provision,
            return_to: pending.return_to,
        })
    }

    /// Consumes a state for a provider-declared error callback. This prevents
    /// replay and also enforces that the callback returns to the same browser
    /// that initiated the flow.
    pub fn consume_provider_error(
        &self,
        state: &str,
        browser_flow: &str,
    ) -> Result<(), OidcFlowError> {
        self.consume_pending(state, browser_flow).map(|_| ())
    }

    pub fn clear_flow_cookie(&self) -> String {
        let mut cookie = format!(
            "{OIDC_FLOW_COOKIE}=; Path=/admin/auth/oidc/callback; HttpOnly; SameSite=Lax; Max-Age=0"
        );
        if self.cookie_secure {
            cookie.push_str("; Secure");
        }
        cookie
    }

    fn flow_cookie(&self, token: &str) -> String {
        let mut cookie = format!(
            "{OIDC_FLOW_COOKIE}={token}; Path=/admin/auth/oidc/callback; HttpOnly; SameSite=Lax; Max-Age={}",
            PENDING_STATE_TTL.as_secs()
        );
        if self.cookie_secure {
            cookie.push_str("; Secure");
        }
        cookie
    }

    fn consume_pending(
        &self,
        state: &str,
        browser_flow: &str,
    ) -> Result<PendingAuthorization, OidcFlowError> {
        validate_callback_value(state)?;
        let pending = {
            let mut states = self
                .pending
                .lock()
                .expect("OIDC pending-state lock poisoned");
            prune_pending(&mut states);
            states
                .remove(&hash_state(state))
                .ok_or(OidcFlowError::InvalidState)?
        };
        validate_callback_value(browser_flow).map_err(|_| OidcFlowError::InvalidState)?;
        // Compare fixed-size hashes rather than the raw browser-flow secrets.
        if pending.browser_flow_hash != hash_state(browser_flow) {
            return Err(OidcFlowError::InvalidState);
        }
        Ok(pending)
    }

    fn insert_pending(&self, key: [u8; 32], authorization: PendingAuthorization) {
        let mut pending = self
            .pending
            .lock()
            .expect("OIDC pending-state lock poisoned");
        prune_pending(&mut pending);
        if pending.len() >= MAX_PENDING_STATES
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, authorization)| authorization.created_at)
                .map(|(key, _)| *key)
        {
            pending.remove(&oldest);
        }
        pending.insert(key, authorization);
    }

    async fn ready_client(&self, config: &OidcConfig) -> Result<ReadyOidcClient, OidcFlowError> {
        let issuer = IssuerUrl::new(config.issuer.clone())
            .map_err(|_| OidcFlowError::ProviderUnavailable)?;
        // Discovery includes a JWKS fetch. Cache it briefly so the start and
        // callback halves of one login do not both contact the provider, while
        // still allowing routine signing-key rotation to be observed quickly.
        let cached = self
            .metadata_cache
            .lock()
            .expect("OIDC metadata-cache lock poisoned")
            .as_ref()
            .filter(|cached| cached.fetched_at.elapsed() < METADATA_CACHE_TTL)
            .map(|cached| cached.metadata.clone());
        let metadata = if let Some(metadata) = cached {
            metadata
        } else {
            let metadata = CoreProviderMetadata::discover_async(issuer, &self.http_client)
                .await
                .map_err(|_| OidcFlowError::ProviderUnavailable)?;
            validate_provider_metadata(&metadata, config.allow_insecure_loopback)?;
            self.metadata_cache
                .lock()
                .expect("OIDC metadata-cache lock poisoned")
                .replace(CachedMetadata {
                    metadata: metadata.clone(),
                    fetched_at: Instant::now(),
                });
            metadata
        };
        validate_provider_metadata(&metadata, config.allow_insecure_loopback)?;
        let token_endpoint = metadata
            .token_endpoint()
            .cloned()
            .ok_or(OidcFlowError::ProviderUnavailable)?;
        let client = OidcClient::from_provider_metadata(
            metadata,
            ClientId::new(config.client_id.clone()),
            config.client_secret.clone().map(ClientSecret::new),
        )
        .set_redirect_uri(
            RedirectUrl::new(config.redirect_uri.clone())
                .map_err(|_| OidcFlowError::ProviderUnavailable)?,
        )
        .set_token_uri(token_endpoint);
        Ok(client)
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

impl OidcConfig {
    fn from_env() -> Result<Option<Self>, AppError> {
        let issuer = env_optional("MODELPORT_OIDC_ISSUER");
        let client_id = env_optional("MODELPORT_OIDC_CLIENT_ID");
        let client_secret = env_optional("MODELPORT_OIDC_CLIENT_SECRET");
        let redirect_uri = env_optional("MODELPORT_OIDC_REDIRECT_URI");
        let any_configured = issuer.is_some()
            || client_id.is_some()
            || client_secret.is_some()
            || redirect_uri.is_some();
        if !any_configured {
            return Ok(None);
        }

        let issuer = issuer.ok_or_else(|| {
            AppError::Config("MODELPORT_OIDC_ISSUER is required when OIDC is configured".to_owned())
        })?;
        let client_id = client_id.ok_or_else(|| {
            AppError::Config(
                "MODELPORT_OIDC_CLIENT_ID is required when OIDC is configured".to_owned(),
            )
        })?;
        let redirect_uri = redirect_uri.ok_or_else(|| {
            AppError::Config(
                "MODELPORT_OIDC_REDIRECT_URI is required when OIDC is configured".to_owned(),
            )
        })?;
        if client_secret.as_deref().is_some_and(is_placeholder_value) {
            return Err(AppError::Config(
                "MODELPORT_OIDC_CLIENT_SECRET must not be a placeholder".to_owned(),
            ));
        }
        let allow_insecure_loopback = env_flag("MODELPORT_OIDC_ALLOW_INSECURE_HTTP");
        let issuer_url =
            validate_config_url(&issuer, allow_insecure_loopback, "MODELPORT_OIDC_ISSUER")?;
        if issuer_url.query().is_some() || issuer_url.fragment().is_some() {
            return Err(AppError::Config(
                "MODELPORT_OIDC_ISSUER must not contain a query or fragment".to_owned(),
            ));
        }
        let redirect_url = validate_config_url(
            &redirect_uri,
            allow_insecure_loopback,
            "MODELPORT_OIDC_REDIRECT_URI",
        )?;
        if redirect_url.path() != "/admin/auth/oidc/callback"
            || redirect_url.query().is_some()
            || redirect_url.fragment().is_some()
        {
            return Err(AppError::Config(
                "MODELPORT_OIDC_REDIRECT_URI must use path /admin/auth/oidc/callback without a query or fragment"
                    .to_owned(),
            ));
        }
        if redirect_url.scheme() == "https" && !env_flag("MODELPORT_ADMIN_COOKIE_SECURE") {
            return Err(AppError::Config(
                "MODELPORT_ADMIN_COOKIE_SECURE=1 is required for an HTTPS OIDC redirect URI"
                    .to_owned(),
            ));
        }

        let label =
            env_optional("MODELPORT_OIDC_LABEL").unwrap_or_else(|| DEFAULT_LABEL.to_owned());
        if label.len() > 80 || label.chars().any(char::is_control) {
            return Err(AppError::Config(
                "MODELPORT_OIDC_LABEL must be at most 80 characters".to_owned(),
            ));
        }
        let username_claim = env_optional("MODELPORT_OIDC_USERNAME_CLAIM")
            .unwrap_or_else(|| DEFAULT_USERNAME_CLAIM.to_owned());
        let email_claim = env_optional("MODELPORT_OIDC_EMAIL_CLAIM")
            .unwrap_or_else(|| DEFAULT_EMAIL_CLAIM.to_owned());
        validate_claim_name(&username_claim, "MODELPORT_OIDC_USERNAME_CLAIM")?;
        validate_claim_name(&email_claim, "MODELPORT_OIDC_EMAIL_CLAIM")?;

        Ok(Some(Self {
            issuer,
            client_id,
            client_secret,
            redirect_uri,
            label,
            auto_provision: env_flag("MODELPORT_OIDC_AUTO_PROVISION"),
            username_claim,
            email_claim,
            allow_insecure_loopback,
        }))
    }
}

impl<'c> AsyncHttpClient<'c> for SecureHttpClient {
    type Error = SecureHttpError;
    type Future =
        Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + Sync + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let url = openidconnect::url::Url::parse(&request.uri().to_string())
                .map_err(|_| SecureHttpError::UrlNotAllowed)?;
            if !url_is_allowed(&url, self.allow_insecure_loopback) {
                return Err(SecureHttpError::UrlNotAllowed);
            }
            AsyncHttpClient::call(&self.inner, request)
                .await
                .map_err(SecureHttpError::Request)
        })
    }
}

fn validate_provider_metadata(
    metadata: &CoreProviderMetadata,
    allow_insecure_loopback: bool,
) -> Result<(), OidcFlowError> {
    let mut endpoints = vec![
        metadata.authorization_endpoint().as_str(),
        metadata.jwks_uri().as_str(),
    ];
    if let Some(token_endpoint) = metadata.token_endpoint() {
        endpoints.push(token_endpoint.as_str());
    }
    if endpoints.iter().all(|endpoint| {
        openidconnect::url::Url::parse(endpoint)
            .ok()
            .is_some_and(|url| url_is_allowed(&url, allow_insecure_loopback))
    }) {
        Ok(())
    } else {
        Err(OidcFlowError::ProviderUnavailable)
    }
}

fn validate_config_url(
    value: &str,
    allow_insecure_loopback: bool,
    variable: &str,
) -> Result<openidconnect::url::Url, AppError> {
    let url = openidconnect::url::Url::parse(value)
        .map_err(|_| AppError::Config(format!("{variable} must be an absolute URL")))?;
    if !url_is_allowed(&url, allow_insecure_loopback) {
        return Err(AppError::Config(format!(
            "{variable} must use HTTPS; loopback HTTP requires MODELPORT_OIDC_ALLOW_INSECURE_HTTP=1"
        )));
    }
    Ok(url)
}

fn url_is_allowed(url: &openidconnect::url::Url, allow_insecure_loopback: bool) -> bool {
    if url.username() != "" || url.password().is_some() || url.host_str().is_none() {
        return false;
    }
    if url.scheme() == "https" {
        return true;
    }
    url.scheme() == "http"
        && allow_insecure_loopback
        && url.host_str().is_some_and(is_loopback_host)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_claim_name(value: &str, variable: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character))
    {
        return Err(AppError::Config(format!("{variable} is invalid")));
    }
    Ok(())
}

fn validate_return_to(value: &str) -> Result<String, OidcFlowError> {
    if value.is_empty()
        || value.len() > MAX_RETURN_TO_BYTES
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\\')
        || value.contains('#')
        || value.chars().any(char::is_control)
    {
        return Err(OidcFlowError::InvalidReturnTo);
    }
    let uri = value
        .parse::<axum::http::Uri>()
        .map_err(|_| OidcFlowError::InvalidReturnTo)?;
    if uri.scheme().is_some() || uri.authority().is_some() || !uri.path().starts_with('/') {
        return Err(OidcFlowError::InvalidReturnTo);
    }
    if uri.path() == "/login" {
        return Err(OidcFlowError::InvalidReturnTo);
    }
    Ok(value.to_owned())
}

fn validate_callback_value(value: &str) -> Result<(), OidcFlowError> {
    if value.is_empty()
        || value.len() > MAX_CALLBACK_VALUE_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(OidcFlowError::InvalidCallback);
    }
    Ok(())
}

fn prune_pending(pending: &mut HashMap<[u8; 32], PendingAuthorization>) {
    pending.retain(|_, authorization| authorization.created_at.elapsed() < PENDING_STATE_TTL);
}

fn hash_state(state: &str) -> [u8; 32] {
    Sha256::digest(state.as_bytes()).into()
}

fn string_claim(claims: &Value, name: &str) -> Option<String> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn env_optional(name: &str) -> Option<String> {
    env_value(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(flow: &str, created_at: Instant) -> PendingAuthorization {
        let (_, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        PendingAuthorization {
            pkce_verifier,
            nonce: Nonce::new("test-nonce".to_owned()),
            browser_flow_hash: hash_state(flow),
            return_to: "/dashboard".to_owned(),
            created_at,
        }
    }

    #[test]
    fn return_to_only_accepts_local_absolute_paths() {
        for valid in ["/", "/dashboard", "/settings?tab=auth"] {
            assert_eq!(validate_return_to(valid).unwrap(), valid);
        }
        for invalid in [
            "https://evil.example/",
            "//evil.example/",
            "/\\evil.example/",
            "dashboard",
            "/dashboard#fragment",
            "/login",
            "/login?next=/dashboard",
            "/dashboard\r\nlocation:https://evil.example",
        ] {
            assert!(validate_return_to(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn insecure_urls_are_limited_to_explicit_loopback_testing() {
        let production = openidconnect::url::Url::parse("https://id.example.com").unwrap();
        let loopback = openidconnect::url::Url::parse("http://127.0.0.1:8000").unwrap();
        let remote_http = openidconnect::url::Url::parse("http://id.example.com").unwrap();
        assert!(url_is_allowed(&production, false));
        assert!(!url_is_allowed(&loopback, false));
        assert!(url_is_allowed(&loopback, true));
        assert!(!url_is_allowed(&remote_http, true));
    }

    #[test]
    fn disabled_methods_keep_password_login_available() {
        let service = OidcService::disabled();
        let methods = service.methods();
        assert!(methods.password_enabled);
        assert!(!methods.oidc.enabled);
        assert_eq!(methods.oidc.start_url, OIDC_START_URL);
        assert_eq!(service.pending_count(), 0);
    }

    #[test]
    fn pending_state_is_browser_bound_single_use_and_bounded() {
        let service = OidcService::disabled();
        service.insert_pending(
            hash_state("wrong-cookie-state"),
            pending("right-cookie", Instant::now()),
        );
        assert!(matches!(
            service.consume_pending("wrong-cookie-state", "wrong-cookie"),
            Err(OidcFlowError::InvalidState)
        ));
        assert!(matches!(
            service.consume_pending("wrong-cookie-state", "right-cookie"),
            Err(OidcFlowError::InvalidState)
        ));

        service.insert_pending(
            hash_state("valid-state"),
            pending("valid-cookie", Instant::now()),
        );
        assert_eq!(
            service
                .consume_pending("valid-state", "valid-cookie")
                .unwrap()
                .return_to,
            "/dashboard"
        );
        assert!(matches!(
            service.consume_pending("valid-state", "valid-cookie"),
            Err(OidcFlowError::InvalidState)
        ));

        service.insert_pending(
            hash_state("expired-state"),
            pending(
                "expired-cookie",
                Instant::now() - PENDING_STATE_TTL - Duration::from_secs(1),
            ),
        );
        assert!(matches!(
            service.consume_pending("expired-state", "expired-cookie"),
            Err(OidcFlowError::InvalidState)
        ));

        for index in 0..=MAX_PENDING_STATES {
            let value = format!("bounded-{index}");
            service.insert_pending(hash_state(&value), pending(&value, Instant::now()));
        }
        assert_eq!(service.pending_count(), MAX_PENDING_STATES);
    }
}
