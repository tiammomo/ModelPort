use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    net::IpAddr,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{error::AppError, pricing, storage::JsonStore};

const DEFAULT_USAGE_LIMIT: usize = 5_000;
const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug)]
pub struct ControlStore {
    store: Option<JsonStore>,
    inner: Mutex<ControlInner>,
    usage_limit: usize,
}

#[derive(Debug, Default)]
struct ControlInner {
    api_keys: BTreeMap<String, ApiKeyRecord>,
    quotas: BTreeMap<String, QuotaRecord>,
    usage: Vec<UsageRecord>,
    route_config: RouteConfigRecord,
    activities: Vec<ActivityRecord>,
    provider_tests: BTreeMap<String, ProviderTestRecord>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlFile {
    #[serde(default)]
    api_keys: Vec<ApiKeyRecord>,
    #[serde(default)]
    quotas: Vec<QuotaRecord>,
    #[serde(default)]
    usage: Vec<UsageRecord>,
    #[serde(default)]
    route_config: RouteConfigRecord,
    #[serde(default)]
    activities: Vec<ActivityRecord>,
    #[serde(default)]
    provider_tests: Vec<ProviderTestRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteConfigRecord {
    #[serde(default)]
    aliases: BTreeMap<String, String>,
    #[serde(default)]
    deleted_aliases: BTreeSet<String>,
    default_provider: Option<String>,
    provider_order: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityRecord {
    id: String,
    timestamp_ms: u64,
    activity_type: String,
    actor: String,
    target: String,
    message: String,
    severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderTestRecord {
    provider_id: String,
    tested_at_ms: u64,
    success: bool,
    message: String,
    #[serde(default)]
    discovered_models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ActivityInput {
    pub activity_type: String,
    pub actor: String,
    pub target: String,
    pub message: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyRecord {
    id: String,
    user_id: String,
    username: String,
    name: String,
    key_hash: String,
    key_prefix: String,
    key_preview: String,
    group: Option<String>,
    created_at_ms: u64,
    last_used_at_ms: Option<u64>,
    expires_at_ms: Option<u64>,
    status: String,
    #[serde(default)]
    ip_restricted: bool,
    #[serde(default)]
    allowed_ips: Vec<String>,
    #[serde(default)]
    spend_limit_usd: f64,
    #[serde(default)]
    rate_limited: bool,
    #[serde(default)]
    five_hour_limit_usd: f64,
    #[serde(default)]
    daily_limit_usd: f64,
    #[serde(default)]
    weekly_limit_usd: f64,
    #[serde(default)]
    monthly_limit_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuotaRecord {
    id: String,
    user_id: String,
    username: String,
    quota_type: String,
    limit: f64,
    used: f64,
    period: String,
    period_start_ms: u64,
    period_end_ms: u64,
    reset_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageRecord {
    id: String,
    timestamp_ms: u64,
    user_id: String,
    username: String,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    #[serde(default)]
    api_key_group: Option<String>,
    model: String,
    resolved_model: String,
    provider: String,
    #[serde(default = "default_protocol")]
    protocol: String,
    stream: bool,
    status: String,
    status_code: u16,
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
    #[serde(default)]
    cache_read_tokens: u64,
    cost_estimate: f64,
    latency_ms: u64,
    #[serde(default)]
    first_byte_latency_ms: Option<u64>,
    #[serde(default)]
    retry_count: u32,
    #[serde(default)]
    client_ip: Option<String>,
    #[serde(default)]
    request_path: Option<String>,
    error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicApiKey {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub name: String,
    pub key_prefix: String,
    pub key_preview: String,
    pub group: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub status: String,
    pub requests_today: u64,
    pub tokens_today: u64,
    pub ip_restricted: bool,
    pub allowed_ips: Vec<String>,
    pub spend_limit_usd: f64,
    pub rate_limited: bool,
    pub five_hour_limit_usd: f64,
    pub daily_limit_usd: f64,
    pub weekly_limit_usd: f64,
    pub monthly_limit_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedApiKey {
    #[serde(flatten)]
    pub public: PublicApiKey,
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiKeyInput {
    pub user_id: String,
    pub username: Option<String>,
    pub name: String,
    pub group: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiKeyInput {
    pub name: Option<String>,
    pub group: Option<String>,
    pub expires_at: Option<String>,
    pub status: Option<String>,
    pub ip_restricted: Option<bool>,
    pub allowed_ips: Option<Vec<String>>,
    pub spend_limit_usd: Option<f64>,
    pub rate_limited: Option<bool>,
    pub five_hour_limit_usd: Option<f64>,
    pub daily_limit_usd: Option<f64>,
    pub weekly_limit_usd: Option<f64>,
    pub monthly_limit_usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertQuotaInput {
    pub id: Option<String>,
    pub user_id: String,
    pub username: String,
    pub quota_type: String,
    pub limit: f64,
    pub period: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicQuota {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub quota_type: String,
    pub limit: f64,
    pub used: f64,
    pub period: String,
    pub period_start: String,
    pub period_end: String,
    pub reset_at: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UsageEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_estimate: f64,
}

#[derive(Debug, Clone)]
pub struct ClientIdentity {
    pub user_id: String,
    pub username: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub api_key_group: Option<String>,
    pub enforce_quotas: bool,
    pub api_key_policy: ApiKeyPolicy,
}

#[derive(Debug, Clone, Default)]
pub struct ApiKeyPolicy {
    pub ip_restricted: bool,
    pub allowed_ips: Vec<String>,
    pub spend_limit_usd: f64,
    pub rate_limited: bool,
    pub five_hour_limit_usd: f64,
    pub daily_limit_usd: f64,
    pub weekly_limit_usd: f64,
    pub monthly_limit_usd: f64,
}

#[derive(Debug, Clone)]
pub struct UsageEventInput {
    pub identity: ClientIdentity,
    pub model: String,
    pub resolved_model: String,
    pub provider: String,
    pub protocol: String,
    pub stream: bool,
    pub success: bool,
    pub status_code: u16,
    pub estimate: UsageEstimate,
    pub latency: Duration,
    pub first_byte_latency: Option<Duration>,
    pub retry_count: u32,
    pub client_ip: Option<String>,
    pub request_path: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_write_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cost_estimate: f64,
    pub api_keys_total: u64,
    pub api_keys_active: u64,
    pub average_latency_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderUsageStats {
    pub requests_total: u64,
    pub successes_total: u64,
    pub duration_ms_total: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RoutingConfigSnapshot {
    pub default_provider: Option<String>,
    pub provider_order: Option<Vec<String>>,
}

impl ControlStore {
    pub fn load() -> Result<Self, AppError> {
        let path = control_store_path();
        let store = JsonStore::open("control", path)?;
        let usage_limit = env::var("MODELPORT_USAGE_LOG_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_USAGE_LIMIT);
        let file: ControlFile = store.read_or_default(json!({
            "apiKeys": [],
            "quotas": [],
            "usage": [],
            "routeConfig": {},
            "activities": [],
            "providerTests": [],
        }))?;

        Ok(Self {
            store: Some(store),
            inner: Mutex::new(ControlInner {
                api_keys: file
                    .api_keys
                    .into_iter()
                    .map(|record| (record.id.clone(), record))
                    .collect(),
                quotas: file
                    .quotas
                    .into_iter()
                    .map(|record| (record.id.clone(), record))
                    .collect(),
                usage: file.usage,
                route_config: file.route_config,
                activities: file.activities,
                provider_tests: file
                    .provider_tests
                    .into_iter()
                    .map(|record| (record.provider_id.clone(), record))
                    .collect(),
            }),
            usage_limit,
        })
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            store: None,
            inner: Mutex::new(ControlInner::default()),
            usage_limit: DEFAULT_USAGE_LIMIT,
        }
    }

    pub fn routing_config(&self) -> RoutingConfigSnapshot {
        let inner = self.inner.lock().expect("control lock poisoned");
        RoutingConfigSnapshot {
            default_provider: inner.route_config.default_provider.clone(),
            provider_order: inner.route_config.provider_order.clone(),
        }
    }

    pub fn effective_aliases(
        &self,
        base_aliases: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let inner = self.inner.lock().expect("control lock poisoned");
        effective_aliases_locked(base_aliases, &inner.route_config)
    }

    pub fn upsert_alias(&self, alias: String, target: String) -> Result<(), AppError> {
        let alias = alias.trim();
        let target = target.trim();
        if alias.is_empty() || alias.len() > 120 {
            return Err(AppError::InvalidRequest(
                "alias must be 1-120 characters".to_owned(),
            ));
        }
        if alias.contains(':') {
            return Err(AppError::InvalidRequest(
                "alias cannot contain provider selector ':'".to_owned(),
            ));
        }
        if target.is_empty() || target.len() > 240 {
            return Err(AppError::InvalidRequest(
                "alias target must be 1-240 characters".to_owned(),
            ));
        }

        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner
            .route_config
            .aliases
            .insert(alias.to_owned(), target.to_owned());
        inner.route_config.deleted_aliases.remove(alias);
        self.save_locked(&inner)
    }

    pub fn delete_alias(&self, alias: &str, tombstone: bool) -> Result<(), AppError> {
        let alias = alias.trim();
        if alias.is_empty() {
            return Err(AppError::InvalidRequest("alias is required".to_owned()));
        }

        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.route_config.aliases.remove(alias);
        if tombstone {
            inner.route_config.deleted_aliases.insert(alias.to_owned());
        } else {
            inner.route_config.deleted_aliases.remove(alias);
        }
        self.save_locked(&inner)
    }

    pub fn set_default_provider(&self, provider_id: String) -> Result<(), AppError> {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            return Err(AppError::InvalidRequest(
                "default provider is required".to_owned(),
            ));
        }
        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.route_config.default_provider = Some(provider_id.to_owned());
        self.save_locked(&inner)
    }

    pub fn set_provider_order(&self, provider_order: Vec<String>) -> Result<(), AppError> {
        if provider_order.is_empty() {
            return Err(AppError::InvalidRequest(
                "provider order cannot be empty".to_owned(),
            ));
        }
        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.route_config.provider_order = Some(provider_order);
        self.save_locked(&inner)
    }

    pub fn record_activity(&self, input: ActivityInput) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.activities.push(ActivityRecord {
            id: format!("act_{}", Uuid::new_v4().simple()),
            timestamp_ms: now_millis(),
            activity_type: input.activity_type,
            actor: input.actor,
            target: input.target,
            message: input.message,
            severity: input.severity,
        });
        let overflow = inner.activities.len().saturating_sub(500);
        if overflow > 0 {
            inner.activities.drain(0..overflow);
        }
        self.save_locked(&inner)
    }

    pub fn activity_rows(&self, limit: usize) -> Vec<serde_json::Value> {
        let inner = self.inner.lock().expect("control lock poisoned");
        inner
            .activities
            .iter()
            .rev()
            .take(limit)
            .map(|record| {
                json!({
                    "id": record.id,
                    "timestamp": record.timestamp_ms.to_string(),
                    "type": record.activity_type,
                    "actor": record.actor,
                    "target": record.target,
                    "message": record.message,
                    "severity": record.severity,
                })
            })
            .collect()
    }

    pub fn activity_count(&self) -> usize {
        let inner = self.inner.lock().expect("control lock poisoned");
        inner.activities.len()
    }

    pub fn data_path(&self) -> Option<String> {
        self.store.as_ref().map(JsonStore::location)
    }

    pub fn default_data_path() -> PathBuf {
        control_store_path()
    }

    pub fn export_snapshot(&self) -> serde_json::Value {
        let inner = self.inner.lock().expect("control lock poisoned");
        json!({
            "apiKeys": inner
                .api_keys
                .values()
                .map(|record| public_api_key(record, &inner.usage))
                .collect::<Vec<_>>(),
            "quotas": inner.quotas.values().map(public_quota).collect::<Vec<_>>(),
            "usage": &inner.usage,
            "routeConfig": &inner.route_config,
            "activities": &inner.activities,
            "providerTests": inner.provider_tests.values().collect::<Vec<_>>(),
        })
    }

    pub fn record_provider_test(
        &self,
        provider_id: String,
        success: bool,
        message: String,
        discovered_models: Vec<String>,
    ) -> Result<u64, AppError> {
        let tested_at_ms = now_millis();
        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.provider_tests.insert(
            provider_id.clone(),
            ProviderTestRecord {
                provider_id,
                tested_at_ms,
                success,
                message,
                discovered_models,
            },
        );
        self.save_locked(&inner)?;
        Ok(tested_at_ms)
    }

    pub fn provider_test_rows(&self) -> BTreeMap<String, serde_json::Value> {
        let inner = self.inner.lock().expect("control lock poisoned");
        inner
            .provider_tests
            .iter()
            .map(|(provider_id, record)| {
                (
                    provider_id.clone(),
                    json!({
                        "testedAt": record.tested_at_ms.to_string(),
                        "success": record.success,
                        "message": record.message,
                        "models": record.discovered_models,
                        "modelCount": record.discovered_models.len(),
                    }),
                )
            })
            .collect()
    }

    pub fn authenticate_headers(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<ClientIdentity>, AppError> {
        let Some(token) = client_token(headers) else {
            return Ok(None);
        };
        let token_hash = hash_secret(token);
        let now = now_millis();
        let mut inner = self.inner.lock().expect("control lock poisoned");
        reset_expired_quotas_locked(&mut inner, now);

        let Some(record) = inner
            .api_keys
            .values_mut()
            .find(|record| record.key_hash == token_hash)
        else {
            return Ok(None);
        };

        if record.status != "active" {
            return Err(AppError::Auth);
        }
        if record.expires_at_ms.is_some_and(|expires| expires <= now) {
            record.status = "revoked".to_owned();
            self.save_locked(&inner)?;
            return Err(AppError::Auth);
        }

        record.last_used_at_ms = Some(now);
        let identity = ClientIdentity {
            user_id: record.user_id.clone(),
            username: record.username.clone(),
            api_key_id: Some(record.id.clone()),
            api_key_name: Some(record.name.clone()),
            api_key_group: record.group.clone(),
            enforce_quotas: true,
            api_key_policy: ApiKeyPolicy {
                ip_restricted: record.ip_restricted,
                allowed_ips: record.allowed_ips.clone(),
                spend_limit_usd: record.spend_limit_usd,
                rate_limited: record.rate_limited,
                five_hour_limit_usd: record.five_hour_limit_usd,
                daily_limit_usd: record.daily_limit_usd,
                weekly_limit_usd: record.weekly_limit_usd,
                monthly_limit_usd: record.monthly_limit_usd,
            },
        };
        self.save_locked(&inner)?;
        Ok(Some(identity))
    }

    pub fn legacy_identity() -> ClientIdentity {
        ClientIdentity {
            user_id: "usr_local_admin".to_owned(),
            username: "local-admin".to_owned(),
            api_key_id: None,
            api_key_name: Some("MODELPORT_AUTH_TOKEN".to_owned()),
            api_key_group: Some("legacy".to_owned()),
            enforce_quotas: false,
            api_key_policy: ApiKeyPolicy::default(),
        }
    }

    pub fn list_api_keys(&self) -> Vec<PublicApiKey> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        reset_expired_quotas_locked(&mut inner, now_millis());
        inner
            .api_keys
            .values()
            .map(|record| public_api_key(record, &inner.usage))
            .collect()
    }

    pub fn list_user_api_keys(&self, user_id: &str) -> Vec<PublicApiKey> {
        self.list_api_keys()
            .into_iter()
            .filter(|record| record.user_id == user_id)
            .collect()
    }

    pub fn active_api_key_count(&self, user_id: &str) -> u32 {
        let inner = self.inner.lock().expect("control lock poisoned");
        inner
            .api_keys
            .values()
            .filter(|record| record.user_id == user_id && record.status == "active")
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    pub fn api_key_user_id(&self, key_id: &str) -> Result<String, AppError> {
        let inner = self.inner.lock().expect("control lock poisoned");
        inner
            .api_keys
            .get(key_id)
            .map(|record| record.user_id.clone())
            .ok_or_else(|| AppError::InvalidRequest("API key not found".to_owned()))
    }

    pub fn create_api_key(&self, input: CreateApiKeyInput) -> Result<CreatedApiKey, AppError> {
        let name = input.name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err(AppError::InvalidRequest(
                "API key name must be 1-80 characters".to_owned(),
            ));
        }
        let user_id = input.user_id.trim();
        if user_id.is_empty() {
            return Err(AppError::InvalidRequest("userId is required".to_owned()));
        }
        let username = input.username.unwrap_or_else(|| user_id.to_owned());
        let key = new_api_key();
        let now = now_millis();
        let record = ApiKeyRecord {
            id: format!("key_{}", Uuid::new_v4().simple()),
            user_id: user_id.to_owned(),
            username,
            name: name.to_owned(),
            key_hash: hash_secret(&key),
            key_prefix: key.chars().take(12).collect(),
            key_preview: preview_secret(&key),
            group: input.group.filter(|value| !value.trim().is_empty()),
            created_at_ms: now,
            last_used_at_ms: None,
            expires_at_ms: input.expires_at.and_then(|value| value.parse::<u64>().ok()),
            status: "active".to_owned(),
            ip_restricted: false,
            allowed_ips: Vec::new(),
            spend_limit_usd: 0.0,
            rate_limited: false,
            five_hour_limit_usd: 0.0,
            daily_limit_usd: 0.0,
            weekly_limit_usd: 0.0,
            monthly_limit_usd: 0.0,
        };

        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.api_keys.insert(record.id.clone(), record.clone());
        self.save_locked(&inner)?;
        Ok(CreatedApiKey {
            public: public_api_key(&record, &inner.usage),
            key,
        })
    }

    pub fn revoke_api_key(&self, key_id: &str) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        let Some(record) = inner.api_keys.get_mut(key_id) else {
            return Err(AppError::InvalidRequest("API key not found".to_owned()));
        };
        record.status = "revoked".to_owned();
        self.save_locked(&inner)
    }

    pub fn update_api_key(
        &self,
        key_id: &str,
        input: UpdateApiKeyInput,
    ) -> Result<PublicApiKey, AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        let Some(record) = inner.api_keys.get(key_id).cloned() else {
            return Err(AppError::InvalidRequest("API key not found".to_owned()));
        };

        let mut updated = record;
        if let Some(name) = input.name {
            let name = name.trim();
            if name.is_empty() || name.len() > 80 {
                return Err(AppError::InvalidRequest(
                    "API key name must be 1-80 characters".to_owned(),
                ));
            }
            updated.name = name.to_owned();
        }
        if let Some(group) = input.group {
            let group = group.trim();
            updated.group = if group.is_empty() {
                None
            } else {
                Some(group.to_owned())
            };
        }
        if let Some(expires_at) = input.expires_at {
            let expires_at = expires_at.trim();
            updated.expires_at_ms = if expires_at.is_empty() {
                None
            } else {
                Some(expires_at.parse::<u64>().map_err(|_| {
                    AppError::InvalidRequest("expiresAt must be a millisecond timestamp".to_owned())
                })?)
            };
        }
        if let Some(status) = input.status {
            let status = status.trim();
            if !matches!(status, "active" | "revoked") {
                return Err(AppError::InvalidRequest(
                    "invalid API key status".to_owned(),
                ));
            }
            updated.status = status.to_owned();
        }
        if let Some(ip_restricted) = input.ip_restricted {
            updated.ip_restricted = ip_restricted;
        }
        if let Some(allowed_ips) = input.allowed_ips {
            updated.allowed_ips = normalize_ip_rules(allowed_ips)?;
        }
        if let Some(spend_limit_usd) = input.spend_limit_usd {
            updated.spend_limit_usd = validate_usd_limit("spendLimitUsd", spend_limit_usd)?;
        }
        if let Some(rate_limited) = input.rate_limited {
            updated.rate_limited = rate_limited;
        }
        if let Some(five_hour_limit_usd) = input.five_hour_limit_usd {
            updated.five_hour_limit_usd =
                validate_usd_limit("fiveHourLimitUsd", five_hour_limit_usd)?;
        }
        if let Some(daily_limit_usd) = input.daily_limit_usd {
            updated.daily_limit_usd = validate_usd_limit("dailyLimitUsd", daily_limit_usd)?;
        }
        if let Some(weekly_limit_usd) = input.weekly_limit_usd {
            updated.weekly_limit_usd = validate_usd_limit("weeklyLimitUsd", weekly_limit_usd)?;
        }
        if let Some(monthly_limit_usd) = input.monthly_limit_usd {
            updated.monthly_limit_usd = validate_usd_limit("monthlyLimitUsd", monthly_limit_usd)?;
        }

        if updated.status == "active"
            && updated
                .expires_at_ms
                .is_some_and(|expires_at| expires_at <= now_millis())
        {
            return Err(AppError::InvalidRequest(
                "cannot activate an expired API key".to_owned(),
            ));
        }

        inner.api_keys.insert(updated.id.clone(), updated.clone());
        self.save_locked(&inner)?;
        Ok(public_api_key(&updated, &inner.usage))
    }

    pub fn delete_api_key(&self, key_id: &str) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        if inner.api_keys.remove(key_id).is_none() {
            return Err(AppError::InvalidRequest("API key not found".to_owned()));
        }
        self.save_locked(&inner)
    }

    pub fn delete_user_resources(&self, user_id: &str) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        for record in inner.api_keys.values_mut() {
            if record.user_id == user_id {
                record.status = "revoked".to_owned();
            }
        }
        inner.quotas.retain(|_, quota| quota.user_id != user_id);
        self.save_locked(&inner)
    }

    pub fn list_quotas(&self) -> Result<Vec<PublicQuota>, AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        reset_expired_quotas_locked(&mut inner, now_millis());
        self.save_locked(&inner)?;
        Ok(inner.quotas.values().map(public_quota).collect())
    }

    pub fn upsert_quota(&self, input: UpsertQuotaInput) -> Result<PublicQuota, AppError> {
        if input.user_id.trim().is_empty() || input.username.trim().is_empty() {
            return Err(AppError::InvalidRequest(
                "userId and username are required".to_owned(),
            ));
        }
        if input.limit < 0.0 {
            return Err(AppError::InvalidRequest(
                "quota limit must be zero or greater".to_owned(),
            ));
        }
        if !matches!(input.quota_type.as_str(), "tokens" | "requests" | "cost") {
            return Err(AppError::InvalidRequest("invalid quota type".to_owned()));
        }
        if !matches!(input.period.as_str(), "daily" | "weekly" | "monthly") {
            return Err(AppError::InvalidRequest("invalid quota period".to_owned()));
        }

        let now = now_millis();
        let (period_start_ms, period_end_ms) = current_period(&input.period, now);
        let id = input
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("quota_{}", Uuid::new_v4().simple()));
        let mut inner = self.inner.lock().expect("control lock poisoned");
        let used = inner.quotas.get(&id).map(|quota| quota.used).unwrap_or(0.0);
        let quota = QuotaRecord {
            id: id.clone(),
            user_id: input.user_id,
            username: input.username,
            quota_type: input.quota_type,
            limit: input.limit,
            used,
            period: input.period,
            period_start_ms,
            period_end_ms,
            reset_at_ms: period_end_ms,
        };
        inner.quotas.insert(id, quota.clone());
        self.save_locked(&inner)?;
        Ok(public_quota(&quota))
    }

    pub fn delete_quota(&self, quota_id: &str) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        inner.quotas.remove(quota_id);
        self.save_locked(&inner)
    }

    pub fn check_quotas(
        &self,
        identity: &ClientIdentity,
        estimate: UsageEstimate,
        client_ip: Option<&str>,
    ) -> Result<(), AppError> {
        if !identity.enforce_quotas {
            return Ok(());
        }
        let mut inner = self.inner.lock().expect("control lock poisoned");
        let now = now_millis();
        reset_expired_quotas_locked(&mut inner, now);
        if let Some(api_key_id) = &identity.api_key_id {
            enforce_api_key_policy(
                &identity.api_key_policy,
                &inner.usage,
                api_key_id,
                estimate,
                client_ip,
                now,
            )?;
        }
        for quota in inner
            .quotas
            .values()
            .filter(|quota| quota.user_id == identity.user_id)
        {
            let increment = quota_increment(quota, estimate);
            if increment > 0.0 && quota.used + increment > quota.limit {
                return Err(AppError::QuotaExceeded(format!(
                    "{} quota exceeded for user {}",
                    quota.quota_type, identity.username
                )));
            }
        }
        Ok(())
    }

    pub fn record_usage(&self, input: UsageEventInput) -> Result<(), AppError> {
        let mut inner = self.inner.lock().expect("control lock poisoned");
        let now = now_millis();
        reset_expired_quotas_locked(&mut inner, now);
        if input.identity.enforce_quotas {
            for quota in inner
                .quotas
                .values_mut()
                .filter(|quota| quota.user_id == input.identity.user_id)
            {
                quota.used += quota_increment(quota, input.estimate);
            }
        }
        inner.usage.push(UsageRecord {
            id: format!("log_{}", Uuid::new_v4().simple()),
            timestamp_ms: now,
            user_id: input.identity.user_id,
            username: input.identity.username,
            api_key_id: input.identity.api_key_id,
            api_key_name: input.identity.api_key_name,
            api_key_group: input.identity.api_key_group,
            model: input.model,
            resolved_model: input.resolved_model,
            provider: input.provider,
            protocol: input.protocol,
            stream: input.stream,
            status: if input.success { "success" } else { "error" }.to_owned(),
            status_code: input.status_code,
            input_tokens: input.estimate.input_tokens,
            output_tokens: input.estimate.output_tokens,
            cache_write_tokens: input.estimate.cache_write_tokens,
            cache_read_tokens: input.estimate.cache_read_tokens,
            cost_estimate: input.estimate.cost_estimate,
            latency_ms: duration_ms(input.latency),
            first_byte_latency_ms: input.first_byte_latency.map(duration_ms),
            retry_count: input.retry_count,
            client_ip: input.client_ip,
            request_path: Some(input.request_path),
            error_message: input.error_message,
        });
        let overflow = inner.usage.len().saturating_sub(self.usage_limit);
        if overflow > 0 {
            inner.usage.drain(0..overflow);
        }
        self.save_locked(&inner)
    }

    pub fn usage_rows(&self) -> Vec<serde_json::Value> {
        let inner = self.inner.lock().expect("control lock poisoned");
        inner
            .usage
            .iter()
            .rev()
            .map(|record| {
                let pricing = pricing::pricing_for_model(&record.resolved_model);
                let input_cost =
                    pricing::cost_component(record.input_tokens, pricing.input_per_million);
                let output_cost =
                    pricing::cost_component(record.output_tokens, pricing.output_per_million);
                let cache_write_cost = pricing::cost_component(
                    record.cache_write_tokens,
                    pricing.cache_write_per_million,
                );
                let cache_read_cost = pricing::cost_component(
                    record.cache_read_tokens,
                    pricing.cache_read_per_million,
                );
                let billed_input_tokens = record
                    .input_tokens
                    .saturating_add(record.cache_write_tokens)
                    .saturating_add(record.cache_read_tokens);
                let total_tokens = billed_input_tokens.saturating_add(record.output_tokens);
                let cache_total = record
                    .cache_write_tokens
                    .saturating_add(record.cache_read_tokens);
                let cache_hit_rate = if billed_input_tokens == 0 {
                    0.0
                } else {
                    (cache_total as f64 / billed_input_tokens as f64) * 100.0
                };
                let first_byte_latency_ms =
                    record.first_byte_latency_ms.unwrap_or(record.latency_ms);
                let request_path = record
                    .request_path
                    .clone()
                    .unwrap_or_else(|| "/v1/messages".to_owned());
                json!({
                    "id": record.id,
                    "timestamp": record.timestamp_ms.to_string(),
                    "userId": record.user_id,
                    "username": record.username,
                    "apiKeyId": record.api_key_id,
                    "apiKeyName": record.api_key_name,
                    "apiKeyGroup": record.api_key_group,
                    "tokenName": record.api_key_name,
                    "group": record.api_key_group,
                    "channelId": record.provider,
                    "channelName": record.provider,
                    "model": record.model,
                    "resolvedModel": record.resolved_model,
                    "provider": record.provider,
                    "protocol": record.protocol,
                    "requestType": if record.status == "success" { "consume" } else { "error" },
                    "stream": if record.stream { "stream" } else { "non-stream" },
                    "status": record.status,
                    "statusCode": record.status_code,
                    "inputTokens": record.input_tokens,
                    "outputTokens": record.output_tokens,
                    "cacheWriteTokens": record.cache_write_tokens,
                    "cacheReadTokens": record.cache_read_tokens,
                    "billedInputTokens": billed_input_tokens,
                    "totalTokens": total_tokens,
                    "cacheHitRate": cache_hit_rate,
                    "costEstimate": record.cost_estimate,
                    "modelPricing": pricing,
                    "costBreakdown": {
                        "inputCost": input_cost,
                        "outputCost": output_cost,
                        "cacheWriteCost": cache_write_cost,
                        "cacheReadCost": cache_read_cost,
                        "totalCost": record.cost_estimate,
                    },
                    "latencyMs": record.latency_ms,
                    "firstByteLatencyMs": first_byte_latency_ms,
                    "retryCount": record.retry_count,
                    "clientIp": record.client_ip,
                    "requestPath": request_path,
                    "billingMode": "upstream-returned",
                    "detail": format!(
                        "模型: {} · 缓存创建: ${:.6}/1M · 缓存命中: ${:.6}/1M · 分组: {}",
                        record.resolved_model,
                        pricing.cache_write_per_million,
                        pricing.cache_read_per_million,
                        record.api_key_group.as_deref().unwrap_or("default"),
                    ),
                    "errorMessage": record.error_message,
                })
            })
            .collect()
    }

    pub fn usage_time_series_24h(&self) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
        let inner = self.inner.lock().expect("control lock poisoned");
        let now = now_millis();
        let hour_ms = 60 * 60 * 1_000;
        let window_start = now.saturating_sub(23 * hour_ms);
        let mut requests = [0u64; 24];
        let mut errors = [0u64; 24];

        for record in inner
            .usage
            .iter()
            .filter(|record| record.timestamp_ms >= window_start)
        {
            let offset = record.timestamp_ms.saturating_sub(window_start) / hour_ms;
            let index = usize::try_from(offset.min(23)).unwrap_or(23);
            requests[index] = requests[index].saturating_add(1);
            if record.status != "success" {
                errors[index] = errors[index].saturating_add(1);
            }
        }

        let request_series = requests
            .iter()
            .enumerate()
            .map(|(index, value)| {
                json!({
                    "timestamp": window_start.saturating_add(u64::try_from(index).unwrap_or(0) * hour_ms).to_string(),
                    "value": value,
                })
            })
            .collect();
        let error_series = errors
            .iter()
            .enumerate()
            .map(|(index, value)| {
                json!({
                    "timestamp": window_start.saturating_add(u64::try_from(index).unwrap_or(0) * hour_ms).to_string(),
                    "value": value,
                })
            })
            .collect();
        (request_series, error_series)
    }

    pub fn usage_top_models_today(&self, limit: usize) -> Vec<serde_json::Value> {
        let inner = self.inner.lock().expect("control lock poisoned");
        let today_start = day_start(now_millis());
        let mut models: BTreeMap<(String, String), u64> = BTreeMap::new();

        for record in inner
            .usage
            .iter()
            .filter(|record| record.timestamp_ms >= today_start)
        {
            let key = (record.resolved_model.clone(), record.provider.clone());
            let count = models.entry(key).or_insert(0);
            *count = count.saturating_add(1);
        }

        let mut rows = models
            .into_iter()
            .map(|((model, provider), requests)| {
                json!({
                    "model": model,
                    "provider": provider,
                    "requests": requests,
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            let left_count = left
                .get("requests")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let right_count = right
                .get("requests")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            right_count.cmp(&left_count)
        });
        rows.truncate(limit);
        rows
    }

    pub fn provider_usage_today(&self) -> BTreeMap<String, ProviderUsageStats> {
        let inner = self.inner.lock().expect("control lock poisoned");
        let today_start = day_start(now_millis());
        let mut providers = BTreeMap::new();

        for record in inner
            .usage
            .iter()
            .filter(|record| record.timestamp_ms >= today_start)
        {
            let stats = providers
                .entry(record.provider.clone())
                .or_insert_with(ProviderUsageStats::default);
            stats.requests_total = stats.requests_total.saturating_add(1);
            if record.status == "success" {
                stats.successes_total = stats.successes_total.saturating_add(1);
            }
            stats.duration_ms_total = stats.duration_ms_total.saturating_add(record.latency_ms);
        }

        providers
    }

    pub fn usage_summary_today(&self) -> UsageSummary {
        let inner = self.inner.lock().expect("control lock poisoned");
        let today_start = day_start(now_millis());
        let mut summary = UsageSummary {
            total_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_write_tokens: 0,
            total_cache_read_tokens: 0,
            total_cost_estimate: 0.0,
            api_keys_total: inner.api_keys.len().try_into().unwrap_or(u64::MAX),
            api_keys_active: inner
                .api_keys
                .values()
                .filter(|record| record.status == "active")
                .count()
                .try_into()
                .unwrap_or(u64::MAX),
            average_latency_ms: 0,
        };
        let mut total_latency = 0u64;
        for record in inner
            .usage
            .iter()
            .filter(|record| record.timestamp_ms >= today_start)
        {
            summary.total_requests += 1;
            summary.total_input_tokens = summary
                .total_input_tokens
                .saturating_add(record.input_tokens);
            summary.total_output_tokens = summary
                .total_output_tokens
                .saturating_add(record.output_tokens);
            summary.total_cache_write_tokens = summary
                .total_cache_write_tokens
                .saturating_add(record.cache_write_tokens);
            summary.total_cache_read_tokens = summary
                .total_cache_read_tokens
                .saturating_add(record.cache_read_tokens);
            summary.total_cost_estimate += record.cost_estimate;
            total_latency = total_latency.saturating_add(record.latency_ms);
        }
        summary.average_latency_ms = total_latency
            .checked_div(summary.total_requests)
            .unwrap_or(0);
        summary
    }

    fn save_locked(&self, inner: &ControlInner) -> Result<(), AppError> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let file = ControlFile {
            api_keys: inner.api_keys.values().cloned().collect(),
            quotas: inner.quotas.values().cloned().collect(),
            usage: inner.usage.clone(),
            route_config: inner.route_config.clone(),
            activities: inner.activities.clone(),
            provider_tests: inner.provider_tests.values().cloned().collect(),
        };
        store.write_json(&file)
    }
}

fn public_api_key(record: &ApiKeyRecord, usage: &[UsageRecord]) -> PublicApiKey {
    let today_start = day_start(now_millis());
    let mut requests_today = 0u64;
    let mut tokens_today = 0u64;
    for usage in usage.iter().filter(|usage| {
        usage.timestamp_ms >= today_start && usage.api_key_id.as_deref() == Some(&record.id)
    }) {
        requests_today += 1;
        tokens_today = tokens_today
            .saturating_add(usage.input_tokens)
            .saturating_add(usage.output_tokens)
            .saturating_add(usage.cache_write_tokens)
            .saturating_add(usage.cache_read_tokens);
    }

    PublicApiKey {
        id: record.id.clone(),
        user_id: record.user_id.clone(),
        username: record.username.clone(),
        name: record.name.clone(),
        key_prefix: record.key_prefix.clone(),
        key_preview: record.key_preview.clone(),
        group: record.group.clone(),
        created_at: record.created_at_ms.to_string(),
        last_used_at: record.last_used_at_ms.map(|value| value.to_string()),
        expires_at: record.expires_at_ms.map(|value| value.to_string()),
        status: record.status.clone(),
        requests_today,
        tokens_today,
        ip_restricted: record.ip_restricted,
        allowed_ips: record.allowed_ips.clone(),
        spend_limit_usd: record.spend_limit_usd,
        rate_limited: record.rate_limited,
        five_hour_limit_usd: record.five_hour_limit_usd,
        daily_limit_usd: record.daily_limit_usd,
        weekly_limit_usd: record.weekly_limit_usd,
        monthly_limit_usd: record.monthly_limit_usd,
    }
}

fn validate_usd_limit(field: &str, value: f64) -> Result<f64, AppError> {
    if !value.is_finite() || value < 0.0 {
        return Err(AppError::InvalidRequest(format!(
            "{field} must be zero or greater"
        )));
    }
    Ok(value)
}

fn normalize_ip_rules(values: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut seen = BTreeSet::new();
    let mut rules = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        validate_ip_rule(value)?;
        if seen.insert(value.to_owned()) {
            rules.push(value.to_owned());
        }
    }
    Ok(rules)
}

fn validate_ip_rule(value: &str) -> Result<(), AppError> {
    if value.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let Some((addr, prefix)) = value.split_once('/') else {
        return Err(AppError::InvalidRequest(format!(
            "invalid IP allowlist entry: {value}"
        )));
    };
    let addr = addr
        .parse::<IpAddr>()
        .map_err(|_| AppError::InvalidRequest(format!("invalid IP allowlist entry: {value}")))?;
    let prefix = prefix
        .parse::<u8>()
        .map_err(|_| AppError::InvalidRequest(format!("invalid IP allowlist entry: {value}")))?;
    let max_prefix = match addr {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    if prefix > max_prefix {
        return Err(AppError::InvalidRequest(format!(
            "invalid IP allowlist entry: {value}"
        )));
    }
    Ok(())
}

fn enforce_api_key_policy(
    policy: &ApiKeyPolicy,
    usage: &[UsageRecord],
    api_key_id: &str,
    estimate: UsageEstimate,
    client_ip: Option<&str>,
    now: u64,
) -> Result<(), AppError> {
    enforce_ip_policy(policy, client_ip)?;
    enforce_spend_limit(
        "total spend",
        policy.spend_limit_usd,
        usage_cost_for_api_key(usage, api_key_id, None),
        estimate.cost_estimate,
    )?;

    if policy.rate_limited {
        enforce_spend_limit(
            "5 hour spend",
            policy.five_hour_limit_usd,
            usage_cost_for_api_key(
                usage,
                api_key_id,
                Some(now.saturating_sub(5 * 60 * 60 * 1_000)),
            ),
            estimate.cost_estimate,
        )?;
        enforce_spend_limit(
            "daily spend",
            policy.daily_limit_usd,
            usage_cost_for_api_key(usage, api_key_id, Some(now.saturating_sub(DAY_MS))),
            estimate.cost_estimate,
        )?;
        enforce_spend_limit(
            "7 day spend",
            policy.weekly_limit_usd,
            usage_cost_for_api_key(usage, api_key_id, Some(now.saturating_sub(7 * DAY_MS))),
            estimate.cost_estimate,
        )?;
        enforce_spend_limit(
            "monthly spend",
            policy.monthly_limit_usd,
            usage_cost_for_api_key(usage, api_key_id, Some(now.saturating_sub(30 * DAY_MS))),
            estimate.cost_estimate,
        )?;
    }

    Ok(())
}

fn enforce_ip_policy(policy: &ApiKeyPolicy, client_ip: Option<&str>) -> Result<(), AppError> {
    if !policy.ip_restricted {
        return Ok(());
    }
    if policy.allowed_ips.is_empty() {
        return Err(AppError::Forbidden(
            "API key IP restriction has no allowed IPs configured".to_owned(),
        ));
    }
    let Some(client_ip) = client_ip else {
        return Err(AppError::Forbidden(
            "client IP is required for this API key".to_owned(),
        ));
    };
    let ip = parse_client_ip(client_ip)
        .ok_or_else(|| AppError::Forbidden("client IP is invalid for this API key".to_owned()))?;
    if policy
        .allowed_ips
        .iter()
        .any(|rule| ip_rule_matches(rule, ip))
    {
        return Ok(());
    }

    Err(AppError::Forbidden(format!(
        "client IP {ip} is not allowed for this API key"
    )))
}

fn enforce_spend_limit(label: &str, limit: f64, used: f64, incoming: f64) -> Result<(), AppError> {
    if limit > 0.0 && used + incoming > limit {
        return Err(AppError::QuotaExceeded(format!(
            "API key {label} limit exceeded ({:.4} / {:.4} USD)",
            used + incoming,
            limit
        )));
    }
    Ok(())
}

fn usage_cost_for_api_key(usage: &[UsageRecord], api_key_id: &str, since: Option<u64>) -> f64 {
    usage
        .iter()
        .filter(|record| record.api_key_id.as_deref() == Some(api_key_id))
        .filter(|record| since.is_none_or(|since| record.timestamp_ms >= since))
        .map(|record| record.cost_estimate.max(0.0))
        .sum()
}

fn parse_client_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }
    value
        .rsplit_once(':')
        .and_then(|(host, _)| host.parse::<IpAddr>().ok())
}

fn ip_rule_matches(rule: &str, ip: IpAddr) -> bool {
    if let Ok(exact) = rule.parse::<IpAddr>() {
        return exact == ip;
    }
    let Some((base, prefix)) = rule.split_once('/') else {
        return false;
    };
    let Ok(base) = base.parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    match (base, ip) {
        (IpAddr::V4(base), IpAddr::V4(ip)) if prefix <= 32 => {
            cidr_matches(u32::from(base).into(), u32::from(ip).into(), prefix, 32)
        }
        (IpAddr::V6(base), IpAddr::V6(ip)) if prefix <= 128 => {
            cidr_matches(u128::from(base), u128::from(ip), prefix, 128)
        }
        _ => false,
    }
}

fn cidr_matches(base: u128, ip: u128, prefix: u8, bits: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = u32::from(bits - prefix);
    (base >> shift) == (ip >> shift)
}

fn effective_aliases_locked(
    base_aliases: &HashMap<String, String>,
    route_config: &RouteConfigRecord,
) -> HashMap<String, String> {
    let mut aliases = base_aliases.clone();
    for alias in &route_config.deleted_aliases {
        aliases.remove(alias);
    }
    for (alias, target) in &route_config.aliases {
        aliases.insert(alias.clone(), target.clone());
    }
    aliases
}

fn default_protocol() -> String {
    "openai-compat".to_owned()
}

fn public_quota(record: &QuotaRecord) -> PublicQuota {
    PublicQuota {
        id: record.id.clone(),
        user_id: record.user_id.clone(),
        username: record.username.clone(),
        quota_type: record.quota_type.clone(),
        limit: record.limit,
        used: record.used,
        period: record.period.clone(),
        period_start: record.period_start_ms.to_string(),
        period_end: record.period_end_ms.to_string(),
        reset_at: record.reset_at_ms.to_string(),
    }
}

fn reset_expired_quotas_locked(inner: &mut ControlInner, now: u64) {
    for quota in inner.quotas.values_mut() {
        if quota.reset_at_ms > now {
            continue;
        }
        let (start, end) = current_period(&quota.period, now);
        quota.used = 0.0;
        quota.period_start_ms = start;
        quota.period_end_ms = end;
        quota.reset_at_ms = end;
    }
}

fn quota_increment(quota: &QuotaRecord, estimate: UsageEstimate) -> f64 {
    match quota.quota_type.as_str() {
        "requests" => 1.0,
        "tokens" => estimate
            .input_tokens
            .saturating_add(estimate.output_tokens)
            .saturating_add(estimate.cache_write_tokens)
            .saturating_add(estimate.cache_read_tokens) as f64,
        "cost" => estimate.cost_estimate,
        _ => 0.0,
    }
}

fn current_period(period: &str, now: u64) -> (u64, u64) {
    match period {
        "daily" => {
            let start = day_start(now);
            (start, start.saturating_add(DAY_MS))
        }
        "weekly" => {
            let start = (now / (DAY_MS * 7)) * (DAY_MS * 7);
            (start, start.saturating_add(DAY_MS * 7))
        }
        "monthly" => {
            let start = (now / (DAY_MS * 30)) * (DAY_MS * 30);
            (start, start.saturating_add(DAY_MS * 30))
        }
        _ => (now, now.saturating_add(DAY_MS)),
    }
}

fn day_start(now: u64) -> u64 {
    (now / DAY_MS) * DAY_MS
}

fn client_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
        })
}

fn control_store_path() -> PathBuf {
    env::var("MODELPORT_CONTROL_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".modelport").join("control-plane.json"))
}

fn new_api_key() -> String {
    format!("sk-mp-{}", Uuid::new_v4().simple())
}

fn hash_secret(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn preview_secret(value: &str) -> String {
    let start = value.chars().take(8).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn creates_and_authenticates_api_key() {
        let store = ControlStore::for_tests();
        let created = store
            .create_api_key(CreateApiKeyInput {
                user_id: "usr_test".to_owned(),
                username: Some("test-user".to_owned()),
                name: "local".to_owned(),
                group: None,
                expires_at: None,
            })
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(&created.key).unwrap());
        let identity = store.authenticate_headers(&headers).unwrap().unwrap();
        assert_eq!(identity.user_id, "usr_test");
        assert_eq!(store.active_api_key_count("usr_test"), 1);
    }

    #[test]
    fn updates_and_restores_api_key() {
        let store = ControlStore::for_tests();
        let created = store
            .create_api_key(CreateApiKeyInput {
                user_id: "usr_test".to_owned(),
                username: Some("test-user".to_owned()),
                name: "local".to_owned(),
                group: Some("dev".to_owned()),
                expires_at: None,
            })
            .unwrap();

        store.revoke_api_key(&created.public.id).unwrap();
        assert_eq!(store.active_api_key_count("usr_test"), 0);

        let updated = store
            .update_api_key(
                &created.public.id,
                UpdateApiKeyInput {
                    name: Some("local restored".to_owned()),
                    group: Some(String::new()),
                    expires_at: None,
                    status: Some("active".to_owned()),
                    ip_restricted: Some(true),
                    allowed_ips: Some(vec!["127.0.0.1".to_owned(), "10.0.0.0/8".to_owned()]),
                    spend_limit_usd: Some(20.0),
                    rate_limited: Some(true),
                    five_hour_limit_usd: Some(0.0),
                    daily_limit_usd: Some(5.0),
                    weekly_limit_usd: Some(25.0),
                    monthly_limit_usd: Some(100.0),
                },
            )
            .unwrap();

        assert_eq!(updated.name, "local restored");
        assert_eq!(updated.group, None);
        assert_eq!(updated.status, "active");
        assert!(updated.ip_restricted);
        assert_eq!(updated.allowed_ips, vec!["127.0.0.1", "10.0.0.0/8"]);
        assert_eq!(updated.daily_limit_usd, 5.0);
        assert_eq!(store.active_api_key_count("usr_test"), 1);
    }

    #[test]
    fn request_quota_is_enforced() {
        let store = ControlStore::for_tests();
        let identity = ClientIdentity {
            user_id: "usr_test".to_owned(),
            username: "test-user".to_owned(),
            api_key_id: Some("key_test".to_owned()),
            api_key_name: Some("local".to_owned()),
            api_key_group: Some("test".to_owned()),
            enforce_quotas: true,
            api_key_policy: ApiKeyPolicy::default(),
        };
        store
            .upsert_quota(UpsertQuotaInput {
                id: Some("quota_test".to_owned()),
                user_id: identity.user_id.clone(),
                username: identity.username.clone(),
                quota_type: "requests".to_owned(),
                limit: 1.0,
                period: "daily".to_owned(),
            })
            .unwrap();

        store
            .check_quotas(&identity, UsageEstimate::default(), None)
            .unwrap();
        store
            .record_usage(UsageEventInput {
                identity: identity.clone(),
                model: "mimo-v2.5-pro".to_owned(),
                resolved_model: "mimo-v2.5-pro".to_owned(),
                provider: "mimo".to_owned(),
                protocol: "openai-compat".to_owned(),
                stream: false,
                success: true,
                status_code: 200,
                estimate: UsageEstimate::default(),
                latency: Duration::from_millis(10),
                first_byte_latency: Some(Duration::from_millis(10)),
                retry_count: 0,
                client_ip: Some("127.0.0.1".to_owned()),
                request_path: "/v1/messages".to_owned(),
                error_message: None,
            })
            .unwrap();

        assert!(
            store
                .check_quotas(&identity, UsageEstimate::default(), None)
                .is_err()
        );
    }

    #[test]
    fn api_key_ip_allowlist_is_enforced() {
        let store = ControlStore::for_tests();
        let identity = ClientIdentity {
            user_id: "usr_test".to_owned(),
            username: "test-user".to_owned(),
            api_key_id: Some("key_test".to_owned()),
            api_key_name: Some("local".to_owned()),
            api_key_group: Some("test".to_owned()),
            enforce_quotas: true,
            api_key_policy: ApiKeyPolicy {
                ip_restricted: true,
                allowed_ips: vec!["10.0.0.0/8".to_owned(), "127.0.0.1".to_owned()],
                ..ApiKeyPolicy::default()
            },
        };

        store
            .check_quotas(&identity, UsageEstimate::default(), Some("10.1.2.3"))
            .unwrap();
        assert!(
            store
                .check_quotas(&identity, UsageEstimate::default(), Some("192.168.1.10"))
                .is_err()
        );
    }

    #[test]
    fn api_key_spend_limit_is_enforced() {
        let store = ControlStore::for_tests();
        let identity = ClientIdentity {
            user_id: "usr_test".to_owned(),
            username: "test-user".to_owned(),
            api_key_id: Some("key_test".to_owned()),
            api_key_name: Some("local".to_owned()),
            api_key_group: Some("test".to_owned()),
            enforce_quotas: true,
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.02,
                rate_limited: true,
                daily_limit_usd: 0.02,
                ..ApiKeyPolicy::default()
            },
        };

        store
            .record_usage(UsageEventInput {
                identity: identity.clone(),
                model: "mimo-v2.5-pro".to_owned(),
                resolved_model: "mimo-v2.5-pro".to_owned(),
                provider: "mimo".to_owned(),
                protocol: "openai-compat".to_owned(),
                stream: false,
                success: true,
                status_code: 200,
                estimate: UsageEstimate {
                    cost_estimate: 0.015,
                    ..UsageEstimate::default()
                },
                latency: Duration::from_millis(10),
                first_byte_latency: Some(Duration::from_millis(10)),
                retry_count: 0,
                client_ip: Some("127.0.0.1".to_owned()),
                request_path: "/v1/messages".to_owned(),
                error_message: None,
            })
            .unwrap();

        assert!(
            store
                .check_quotas(
                    &identity,
                    UsageEstimate {
                        cost_estimate: 0.01,
                        ..UsageEstimate::default()
                    },
                    None,
                )
                .is_err()
        );
    }

    #[test]
    fn route_alias_overrides_are_persistent_in_control_store() {
        let store = ControlStore::for_tests();
        let base_aliases = HashMap::from([("base".to_owned(), "mimo".to_owned())]);

        store
            .upsert_alias("fast".to_owned(), "mimo:mimo-v2.5-pro".to_owned())
            .unwrap();
        let aliases = store.effective_aliases(&base_aliases);
        assert_eq!(
            aliases.get("fast").map(String::as_str),
            Some("mimo:mimo-v2.5-pro")
        );
        assert_eq!(aliases.get("base").map(String::as_str), Some("mimo"));

        store.delete_alias("base", true).unwrap();
        let aliases = store.effective_aliases(&base_aliases);
        assert!(!aliases.contains_key("base"));
        assert_eq!(
            aliases.get("fast").map(String::as_str),
            Some("mimo:mimo-v2.5-pro")
        );
    }
}
