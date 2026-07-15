use std::{
    collections::{HashMap, HashSet},
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgRow};
use tokio::sync::oneshot;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    AppError,
    control::{UsageEstimate, UsageEventInput},
    database::{
        connect_pool, database_url as control_database_url, enterprise_database_url,
        enterprise_mode_enabled, redact_database_url,
    },
    domain::{AttemptId, RequestContext, TenantScope},
};

const DEFAULT_LEASE_TTL_SECS: u64 = 300;
const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 60;
const MIN_LEASE_TTL_SECS: u64 = 30;
const MIN_RECONCILE_INTERVAL_SECS: u64 = 5;

#[derive(Clone)]
pub(crate) struct EnterpriseLedger {
    backend: Arc<LedgerBackend>,
    location: Arc<str>,
    instance_id: Arc<str>,
    lease_ttl: Duration,
    reconcile_interval: Duration,
}

enum LedgerBackend {
    Memory(Box<Mutex<MemoryLedger>>),
    Postgres(PgPool),
}

#[derive(Debug, Default)]
struct MemoryLedger {
    requests: HashMap<String, MemoryRequestRecord>,
    attempts: HashMap<String, MemoryRecord>,
    budget_accounts: HashMap<TenantKey, MemoryBudgetAccount>,
    budget_reservations: HashMap<String, MemoryBudgetReservation>,
    budget_events: Vec<EnterpriseBudgetEvent>,
}

#[derive(Debug, Clone, Default)]
struct MemoryBudgetAccount {
    limit_microunits: Option<i64>,
    reserved_microunits: i64,
    settled_microunits: i64,
    version: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct MemoryBudgetReservation {
    reservation_id: String,
    tenant: TenantKey,
    request_ledger_id: String,
    attempt_id: String,
    reserved_microunits: i64,
    settled_microunits: i64,
    state: String,
    updated_at_ms: i64,
    terminal_at_ms: Option<i64>,
}

#[derive(Debug)]
struct MemoryRecord {
    tenant: TenantKey,
    request_ledger_id: String,
    terminal: bool,
    lease_owner: String,
    lease_expires_at: Instant,
    lease_expires_at_ms: i64,
    state: String,
    status_code: Option<i32>,
    terminal_reason: Option<String>,
    error_message: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    cost_amount_microunits: i64,
    billing_mode: Option<String>,
    chargeable: bool,
    created_at_ms: i64,
    updated_at_ms: i64,
    completed_at_ms: Option<i64>,
    provider_id: Option<String>,
    resolved_model: Option<String>,
    provider_protocol: Option<String>,
}

#[derive(Debug)]
struct MemoryRequestRecord {
    record: MemoryRecord,
    request_id: String,
    principal_id: String,
    client_protocol: String,
    requested_model: String,
    stream: bool,
    idempotency_key_hash: Option<String>,
    request_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TenantKey {
    organization_id: String,
    project_id: String,
    environment_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LedgerRequest {
    ledger_id: String,
    tenant: TenantKey,
    lease_owner: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LedgerAttempt {
    attempt_id: String,
    request_ledger_id: String,
    reservation_id: String,
    tenant: TenantKey,
    lease_owner: String,
}

pub(crate) struct LedgerLease {
    stop: Option<oneshot::Sender<()>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReconcileResult {
    pub(crate) requests: u64,
    pub(crate) attempts: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseLedgerQuery {
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
    pub(crate) state: Option<String>,
    pub(crate) protocol: Option<String>,
    pub(crate) organization_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) environment_id: Option<String>,
    pub(crate) search: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseLedgerOverview {
    backend: &'static str,
    location: String,
    lease_ttl_secs: u64,
    reconcile_interval_secs: u64,
    total_requests: i64,
    started_requests: i64,
    completed_requests: i64,
    failed_requests: i64,
    cancelled_requests: i64,
    unreconciled_requests: i64,
    idempotent_requests: i64,
    active_leases: i64,
    expired_leases: i64,
    chargeable_requests: i64,
    total_cost_microunits: i64,
    organization_count: i64,
    project_count: i64,
    environment_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseRequestRow {
    ledger_id: String,
    request_id: String,
    organization_id: String,
    project_id: String,
    environment_id: String,
    principal_id: String,
    client_protocol: String,
    requested_model: String,
    stream: bool,
    state: String,
    status_code: Option<i32>,
    terminal_reason: Option<String>,
    error_message: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    cost_amount_microunits: i64,
    currency: String,
    billing_mode: Option<String>,
    chargeable: bool,
    has_idempotency_key: bool,
    lease_owner: String,
    lease_expires_at_ms: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    completed_at_ms: Option<i64>,
    attempt_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseAttemptRow {
    attempt_id: String,
    request_ledger_id: String,
    organization_id: String,
    project_id: String,
    environment_id: String,
    provider_id: String,
    resolved_model: String,
    provider_protocol: String,
    state: String,
    status_code: Option<i32>,
    terminal_reason: Option<String>,
    error_message: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    cost_amount_microunits: i64,
    currency: String,
    billing_mode: Option<String>,
    chargeable: bool,
    lease_owner: String,
    lease_expires_at_ms: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
    completed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseRequestPage {
    requests: Vec<EnterpriseRequestRow>,
    total: i64,
    page: usize,
    page_size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseRequestDetail {
    request: EnterpriseRequestRow,
    attempts: Vec<EnterpriseAttemptRow>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseBudgetScopeQuery {
    pub(crate) organization_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) environment_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseBudgetUpdate {
    organization_id: String,
    project_id: String,
    environment_id: String,
    limit_microunits: Option<i64>,
    #[serde(default)]
    unlimited: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseBudgetAdjustmentInput {
    organization_id: String,
    project_id: String,
    environment_id: String,
    delta_microunits: i64,
    reason: String,
    evidence_reference: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseBudgetAccount {
    organization_id: String,
    project_id: String,
    environment_id: String,
    currency: String,
    limit_microunits: Option<i64>,
    reserved_microunits: i64,
    settled_microunits: i64,
    available_microunits: Option<i64>,
    utilization_basis_points: Option<i64>,
    version: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseBudgetEvent {
    event_id: String,
    organization_id: String,
    project_id: String,
    environment_id: String,
    currency: String,
    reservation_id: Option<String>,
    request_ledger_id: Option<String>,
    attempt_id: Option<String>,
    event_type: String,
    reserved_delta_microunits: i64,
    settled_delta_microunits: i64,
    evidence_source: String,
    billing_mode: Option<String>,
    reason: Option<String>,
    actor_id: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseBudgetView {
    account: EnterpriseBudgetAccount,
    recent_events: Vec<EnterpriseBudgetEvent>,
}

#[derive(Debug, Clone)]
pub(crate) struct LedgerOutcome {
    state: &'static str,
    status_code: u16,
    terminal_reason: String,
    error_message: Option<String>,
    estimate: UsageEstimate,
    billing_mode: String,
    chargeable: bool,
}

impl MemoryRecord {
    fn started(
        tenant: TenantKey,
        request_ledger_id: String,
        lease_owner: String,
        lease_ttl: Duration,
        provider: Option<(&str, &str, &str)>,
    ) -> Self {
        let now = now_millis();
        let (provider_id, resolved_model, provider_protocol) = provider
            .map(|(provider_id, resolved_model, provider_protocol)| {
                (
                    Some(provider_id.to_owned()),
                    Some(resolved_model.to_owned()),
                    Some(provider_protocol.to_owned()),
                )
            })
            .unwrap_or((None, None, None));
        Self {
            tenant,
            request_ledger_id,
            terminal: false,
            lease_owner,
            lease_expires_at: Instant::now() + lease_ttl,
            lease_expires_at_ms: now.saturating_add(duration_millis_i64(lease_ttl)),
            state: "started".to_owned(),
            status_code: None,
            terminal_reason: None,
            error_message: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            cost_amount_microunits: 0,
            billing_mode: None,
            chargeable: false,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
            provider_id,
            resolved_model,
            provider_protocol,
        }
    }

    fn finalize(&mut self, outcome: &LedgerOutcome) {
        let now = now_millis();
        self.terminal = true;
        self.state = outcome.state.to_owned();
        self.status_code = Some(i32::from(outcome.status_code));
        self.terminal_reason = Some(outcome.terminal_reason.clone());
        self.error_message = outcome.error_message.clone();
        self.input_tokens = to_i64(outcome.estimate.input_tokens);
        self.output_tokens = to_i64(outcome.estimate.output_tokens);
        self.cache_write_tokens = to_i64(outcome.estimate.cache_write_tokens);
        self.cache_read_tokens = to_i64(outcome.estimate.cache_read_tokens);
        self.cost_amount_microunits = cost_microunits(outcome.estimate.cost_estimate);
        self.billing_mode = Some(outcome.billing_mode.clone());
        self.chargeable = outcome.chargeable;
        self.updated_at_ms = now;
        self.completed_at_ms = Some(now);
    }

    fn mark_unreconciled(&mut self, provider_attempt: bool) {
        let now = now_millis();
        self.terminal = true;
        self.state = "failed".to_owned();
        self.status_code = Some(500);
        self.terminal_reason = Some("lease_expired_unreconciled".to_owned());
        self.error_message = Some(
            if provider_attempt {
                "ledger lease expired before a terminal Provider outcome was persisted"
            } else {
                "ledger lease expired before a terminal request outcome was persisted"
            }
            .to_owned(),
        );
        self.billing_mode = Some("unreconciled".to_owned());
        self.chargeable = false;
        self.updated_at_ms = now;
        self.completed_at_ms = Some(now);
    }
}

impl EnterpriseLedger {
    pub(crate) fn validate_configuration() -> Result<(), AppError> {
        lease_config().map(|_| ())
    }

    pub(crate) fn memory() -> Self {
        Self {
            backend: Arc::new(LedgerBackend::Memory(Box::new(Mutex::new(
                MemoryLedger::default(),
            )))),
            location: Arc::from("memory://enterprise-ledger"),
            instance_id: Arc::from(format!("ins_{}", Uuid::new_v4().simple())),
            lease_ttl: Duration::from_secs(DEFAULT_LEASE_TTL_SECS),
            reconcile_interval: Duration::from_secs(DEFAULT_RECONCILE_INTERVAL_SECS),
        }
    }

    pub(crate) async fn connect_from_env() -> Result<Self, AppError> {
        let enterprise = enterprise_mode_enabled()?;
        let (lease_ttl, reconcile_interval) = lease_config()?;
        if enterprise && control_database_url().is_none() {
            return Err(AppError::Config(
                "MODELPORT_ENTERPRISE_MODE requires MODELPORT_DATABASE_URL so auth and control state do not fall back to files"
                    .to_owned(),
            ));
        }
        let Some(database_url) = enterprise_database_url() else {
            if enterprise {
                return Err(AppError::Config(
                    "MODELPORT_ENTERPRISE_MODE requires MODELPORT_ENTERPRISE_DATABASE_URL or MODELPORT_DATABASE_URL"
                        .to_owned(),
                ));
            }
            return Ok(Self::memory());
        };

        let pool = connect_pool(&database_url, None).await?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|error| AppError::Database(format!("database migration failed: {error}")))?;

        Ok(Self {
            backend: Arc::new(LedgerBackend::Postgres(pool)),
            location: Arc::from(format!(
                "{}#relational-ledger",
                redact_database_url(&database_url)
            )),
            instance_id: Arc::from(format!("ins_{}", Uuid::new_v4().simple())),
            lease_ttl,
            reconcile_interval,
        })
    }

    pub(crate) fn location(&self) -> &str {
        &self.location
    }

    pub(crate) async fn health_check(&self) -> Result<(), AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(_) => Ok(()),
            LedgerBackend::Postgres(pool) => {
                sqlx::query_scalar::<_, i32>("SELECT 1")
                    .fetch_one(pool)
                    .await?;
                Ok(())
            }
        }
    }

    pub(crate) async fn begin_request(
        &self,
        context: &RequestContext,
        requested_model: &str,
        stream: bool,
        idempotency_key: Option<&str>,
        request_fingerprint: &str,
    ) -> Result<LedgerRequest, AppError> {
        if request_fingerprint.len() != 64 {
            return Err(AppError::Database(
                "request fingerprint must be a SHA-256 hex digest".to_owned(),
            ));
        }
        let idempotency_key_hash = idempotency_key.map(hash_idempotency_key);
        let request = LedgerRequest {
            ledger_id: format!("grq_{}", Uuid::new_v4().simple()),
            tenant: TenantKey::from(&context.tenant),
            lease_owner: self.instance_id.to_string(),
        };

        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                if let Some(key_hash) = &idempotency_key_hash
                    && let Some(existing) = ledger.requests.values().find(|record| {
                        record.record.tenant == request.tenant
                            && record.idempotency_key_hash.as_ref() == Some(key_hash)
                    })
                {
                    return Err(idempotency_conflict(
                        existing.request_fingerprint == request_fingerprint,
                        existing.record.terminal,
                    ));
                }
                let record = MemoryRecord::started(
                    request.tenant.clone(),
                    request.ledger_id.clone(),
                    request.lease_owner.clone(),
                    self.lease_ttl,
                    None,
                );
                ledger.requests.insert(
                    request.ledger_id.clone(),
                    MemoryRequestRecord {
                        record,
                        request_id: context.request_id.to_string(),
                        principal_id: context.principal_id.to_string(),
                        client_protocol: context.protocol.as_str().to_owned(),
                        requested_model: requested_model.to_owned(),
                        stream,
                        idempotency_key_hash,
                        request_fingerprint: request_fingerprint.to_owned(),
                    },
                );
            }
            LedgerBackend::Postgres(pool) => {
                let result = sqlx::query(
                    "INSERT INTO modelport_gateway_requests (
                        ledger_id, request_id,
                        organization_id, project_id, environment_id,
                        principal_id, client_protocol, requested_model, stream,
                        idempotency_key_hash, request_fingerprint,
                        lease_owner, lease_expires_at
                    ) VALUES (
                        $1, $2, $3, $4, $5, $6, $7, $8, $9,
                        $10, $11, $12, now() + ($13 * interval '1 second')
                    )
                    ON CONFLICT (
                        organization_id, project_id, environment_id, idempotency_key_hash
                    ) WHERE idempotency_key_hash IS NOT NULL
                    DO NOTHING",
                )
                .bind(&request.ledger_id)
                .bind(context.request_id.as_str())
                .bind(&request.tenant.organization_id)
                .bind(&request.tenant.project_id)
                .bind(&request.tenant.environment_id)
                .bind(context.principal_id.as_str())
                .bind(context.protocol.as_str())
                .bind(requested_model)
                .bind(stream)
                .bind(idempotency_key_hash.as_deref())
                .bind(request_fingerprint)
                .bind(&request.lease_owner)
                .bind(duration_secs_i32(self.lease_ttl))
                .execute(pool)
                .await?;

                if result.rows_affected() == 0 {
                    let key_hash = idempotency_key_hash.as_deref().ok_or_else(|| {
                        AppError::Database(
                            "request insertion conflicted without an idempotency key".to_owned(),
                        )
                    })?;
                    let existing = sqlx::query_as::<_, (String, String)>(
                        "SELECT request_fingerprint, state
                         FROM modelport_gateway_requests
                         WHERE organization_id = $1
                           AND project_id = $2
                           AND environment_id = $3
                           AND idempotency_key_hash = $4",
                    )
                    .bind(&request.tenant.organization_id)
                    .bind(&request.tenant.project_id)
                    .bind(&request.tenant.environment_id)
                    .bind(key_hash)
                    .fetch_one(pool)
                    .await?;
                    return Err(idempotency_conflict(
                        existing.0 == request_fingerprint,
                        existing.1 != "started",
                    ));
                }
            }
        }
        Ok(request)
    }

    pub(crate) async fn begin_attempt(
        &self,
        request: &LedgerRequest,
        attempt_id: &AttemptId,
        provider_id: &str,
        resolved_model: &str,
        provider_protocol: &str,
        estimate: UsageEstimate,
    ) -> Result<LedgerAttempt, AppError> {
        let reservation_id = format!("brs_{}", Uuid::new_v4().simple());
        let reserved_microunits = cost_microunits(estimate.cost_estimate);
        let attempt = LedgerAttempt {
            attempt_id: attempt_id.to_string(),
            request_ledger_id: request.ledger_id.clone(),
            reservation_id,
            tenant: request.tenant.clone(),
            lease_owner: request.lease_owner.clone(),
        };

        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let parent = ledger.requests.get(&request.ledger_id);
                if !parent.is_some_and(|record| {
                    record.record.tenant == request.tenant
                        && record.record.lease_owner == request.lease_owner
                        && !record.record.terminal
                }) {
                    return Err(AppError::Database(
                        "request ledger scope is invalid or already terminal".to_owned(),
                    ));
                }
                if ledger.attempts.contains_key(&attempt.attempt_id)
                    || ledger.budget_reservations.contains_key(&attempt.attempt_id)
                {
                    return Err(AppError::Database(
                        "Provider Attempt already exists in enterprise ledger".to_owned(),
                    ));
                }
                let account = ledger
                    .budget_accounts
                    .entry(attempt.tenant.clone())
                    .or_insert_with(|| MemoryBudgetAccount {
                        updated_at_ms: now_millis(),
                        ..MemoryBudgetAccount::default()
                    });
                if account.limit_microunits.is_some_and(|limit| {
                    account
                        .settled_microunits
                        .saturating_add(account.reserved_microunits)
                        .saturating_add(reserved_microunits)
                        > limit
                }) {
                    return Err(budget_exceeded(account, reserved_microunits));
                }
                let now = now_millis();
                account.reserved_microunits = account
                    .reserved_microunits
                    .saturating_add(reserved_microunits);
                account.version = account.version.saturating_add(1);
                account.updated_at_ms = now;
                ledger.attempts.insert(
                    attempt.attempt_id.clone(),
                    MemoryRecord::started(
                        attempt.tenant.clone(),
                        attempt.request_ledger_id.clone(),
                        attempt.lease_owner.clone(),
                        self.lease_ttl,
                        Some((provider_id, resolved_model, provider_protocol)),
                    ),
                );
                ledger.budget_reservations.insert(
                    attempt.attempt_id.clone(),
                    MemoryBudgetReservation {
                        reservation_id: attempt.reservation_id.clone(),
                        tenant: attempt.tenant.clone(),
                        request_ledger_id: attempt.request_ledger_id.clone(),
                        attempt_id: attempt.attempt_id.clone(),
                        reserved_microunits,
                        settled_microunits: 0,
                        state: "reserved".to_owned(),
                        updated_at_ms: now,
                        terminal_at_ms: None,
                    },
                );
                ledger.budget_events.push(budget_event(
                    &attempt,
                    "reservation_created",
                    reserved_microunits,
                    0,
                    "local-estimate",
                    None,
                    Some("Provider Attempt budget reservation"),
                    None,
                    estimate,
                ));
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                sqlx::query(
                    "INSERT INTO modelport_provider_attempts (
                        attempt_id, request_ledger_id,
                        organization_id, project_id, environment_id,
                        provider_id, resolved_model, provider_protocol,
                        lease_owner, lease_expires_at
                    )
                    SELECT $1, ledger_id, organization_id, project_id, environment_id,
                           $6, $7, $8, $9, now() + ($10 * interval '1 second')
                    FROM modelport_gateway_requests
                    WHERE ledger_id = $2
                      AND organization_id = $3
                      AND project_id = $4
                      AND environment_id = $5
                      AND lease_owner = $9
                      AND state = 'started'",
                )
                .bind(&attempt.attempt_id)
                .bind(&attempt.request_ledger_id)
                .bind(&attempt.tenant.organization_id)
                .bind(&attempt.tenant.project_id)
                .bind(&attempt.tenant.environment_id)
                .bind(provider_id)
                .bind(resolved_model)
                .bind(provider_protocol)
                .bind(&attempt.lease_owner)
                .bind(duration_secs_i32(self.lease_ttl))
                .execute(&mut *transaction)
                .await
                .and_then(|result| {
                    if result.rows_affected() == 1 {
                        Ok(result)
                    } else {
                        Err(sqlx::Error::RowNotFound)
                    }
                })?;
                sqlx::query(
                    "INSERT INTO modelport_budget_accounts (
                        organization_id, project_id, environment_id, currency
                     ) VALUES ($1, $2, $3, 'USD')
                     ON CONFLICT (organization_id, project_id, environment_id, currency)
                     DO NOTHING",
                )
                .bind(&attempt.tenant.organization_id)
                .bind(&attempt.tenant.project_id)
                .bind(&attempt.tenant.environment_id)
                .execute(&mut *transaction)
                .await?;
                let reserved = sqlx::query_as::<_, (Option<i64>, i64, i64)>(
                    "UPDATE modelport_budget_accounts
                     SET reserved_microunits = reserved_microunits + $1,
                         version = version + 1,
                         updated_at = now()
                     WHERE organization_id = $2
                       AND project_id = $3
                       AND environment_id = $4
                       AND currency = 'USD'
                       AND (
                           limit_microunits IS NULL
                           OR settled_microunits + reserved_microunits + $1 <= limit_microunits
                       )
                     RETURNING limit_microunits, reserved_microunits, settled_microunits",
                )
                .bind(reserved_microunits)
                .bind(&attempt.tenant.organization_id)
                .bind(&attempt.tenant.project_id)
                .bind(&attempt.tenant.environment_id)
                .fetch_optional(&mut *transaction)
                .await?;
                if reserved.is_none() {
                    return Err(AppError::QuotaExceeded(format!(
                        "enterprise budget has insufficient available balance for a {} microunit reservation",
                        reserved_microunits
                    )));
                }
                sqlx::query(
                    "INSERT INTO modelport_budget_reservations (
                        reservation_id,
                        organization_id, project_id, environment_id, currency,
                        request_ledger_id, attempt_id, reserved_microunits
                     ) VALUES ($1, $2, $3, $4, 'USD', $5, $6, $7)",
                )
                .bind(&attempt.reservation_id)
                .bind(&attempt.tenant.organization_id)
                .bind(&attempt.tenant.project_id)
                .bind(&attempt.tenant.environment_id)
                .bind(&attempt.request_ledger_id)
                .bind(&attempt.attempt_id)
                .bind(reserved_microunits)
                .execute(&mut *transaction)
                .await?;
                insert_budget_event_pg(
                    &mut transaction,
                    &attempt,
                    "reservation_created",
                    reserved_microunits,
                    0,
                    "local-estimate",
                    None,
                    Some("Provider Attempt budget reservation"),
                    None,
                    estimate,
                )
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(attempt)
    }

    pub(crate) async fn finalize_attempt(
        &self,
        attempt: &LedgerAttempt,
        outcome: &LedgerOutcome,
    ) -> Result<(), AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let Some(record) = ledger
                    .attempts
                    .get_mut(&attempt.attempt_id)
                    .filter(|record| {
                        record.tenant == attempt.tenant && record.lease_owner == attempt.lease_owner
                    })
                else {
                    return Err(missing_scoped_record());
                };
                if record.terminal {
                    return Ok(());
                }
                record.finalize(outcome);
                settle_memory_budget(&mut ledger, attempt, outcome)?;
                Ok(())
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let updated = update_terminal_record_pg(
                    &mut transaction,
                    false,
                    &attempt.attempt_id,
                    &attempt.tenant,
                    &attempt.lease_owner,
                    outcome,
                )
                .await?;
                if !updated {
                    let state = sqlx::query_scalar::<_, String>(
                        "SELECT state FROM modelport_provider_attempts
                         WHERE attempt_id = $1
                           AND organization_id = $2
                           AND project_id = $3
                           AND environment_id = $4
                           AND lease_owner = $5",
                    )
                    .bind(&attempt.attempt_id)
                    .bind(&attempt.tenant.organization_id)
                    .bind(&attempt.tenant.project_id)
                    .bind(&attempt.tenant.environment_id)
                    .bind(&attempt.lease_owner)
                    .fetch_optional(&mut *transaction)
                    .await?;
                    if state.is_some_and(|state| state != "started") {
                        transaction.commit().await?;
                        return Ok(());
                    }
                    return Err(missing_scoped_record());
                }
                settle_budget_pg(&mut transaction, attempt, outcome).await?;
                transaction.commit().await?;
                Ok(())
            }
        }
    }

    pub(crate) async fn finalize_request(
        &self,
        request: &LedgerRequest,
        outcome: &LedgerOutcome,
    ) -> Result<(), AppError> {
        self.finalize_request_record(
            &request.ledger_id,
            &request.tenant,
            &request.lease_owner,
            outcome,
        )
        .await
    }

    async fn finalize_request_record(
        &self,
        id: &str,
        tenant: &TenantKey,
        lease_owner: &str,
        outcome: &LedgerOutcome,
    ) -> Result<(), AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let Some(record) = ledger.requests.get_mut(id).filter(|record| {
                    record.record.tenant == *tenant && record.record.lease_owner == lease_owner
                }) else {
                    return Err(missing_scoped_record());
                };
                record.record.finalize(outcome);
                Ok(())
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                update_terminal_record_pg(&mut transaction, true, id, tenant, lease_owner, outcome)
                    .await?;
                transaction.commit().await?;
                Ok(())
            }
        }
    }

    pub(crate) fn maintain_lease(&self, request: &LedgerRequest) -> LedgerLease {
        if matches!(self.backend.as_ref(), LedgerBackend::Memory(_)) {
            return LedgerLease { stop: None };
        }

        let (stop, mut stopped) = oneshot::channel();
        let ledger = self.clone();
        let request = request.clone();
        let heartbeat_interval = self.lease_ttl.div_f32(3.0).max(Duration::from_secs(1));
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(heartbeat_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if let Err(err) = ledger.renew_lease(&request).await {
                            warn!(
                                error = %err,
                                ledger_id = request.ledger_id.as_str(),
                                "failed to renew inference ledger lease"
                            );
                        }
                    }
                    _ = &mut stopped => break,
                }
            }
        });
        LedgerLease { stop: Some(stop) }
    }

    async fn renew_lease(&self, request: &LedgerRequest) -> Result<(), AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let expires_at = Instant::now() + self.lease_ttl;
                let expires_at_ms =
                    now_millis().saturating_add(duration_millis_i64(self.lease_ttl));
                if let Some(record) = ledger
                    .requests
                    .get_mut(&request.ledger_id)
                    .filter(|record| {
                        record.record.tenant == request.tenant
                            && record.record.lease_owner == request.lease_owner
                            && !record.record.terminal
                    })
                {
                    record.record.lease_expires_at = expires_at;
                    record.record.lease_expires_at_ms = expires_at_ms;
                    record.record.updated_at_ms = now_millis();
                }
                for record in ledger.attempts.values_mut().filter(|record| {
                    record.tenant == request.tenant
                        && record.request_ledger_id == request.ledger_id
                        && record.lease_owner == request.lease_owner
                        && !record.terminal
                }) {
                    record.lease_expires_at = expires_at;
                    record.lease_expires_at_ms = expires_at_ms;
                    record.updated_at_ms = now_millis();
                }
                Ok(())
            }
            LedgerBackend::Postgres(pool) => {
                let lease_ttl = duration_secs_i32(self.lease_ttl);
                let mut transaction = pool.begin().await?;
                sqlx::query(
                    "UPDATE modelport_gateway_requests
                     SET lease_expires_at = now() + ($1 * interval '1 second'),
                         updated_at = now()
                     WHERE ledger_id = $2
                       AND organization_id = $3
                       AND project_id = $4
                       AND environment_id = $5
                       AND lease_owner = $6
                       AND state = 'started'",
                )
                .bind(lease_ttl)
                .bind(&request.ledger_id)
                .bind(&request.tenant.organization_id)
                .bind(&request.tenant.project_id)
                .bind(&request.tenant.environment_id)
                .bind(&request.lease_owner)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "UPDATE modelport_provider_attempts
                     SET lease_expires_at = now() + ($1 * interval '1 second'),
                         updated_at = now()
                     WHERE request_ledger_id = $2
                       AND organization_id = $3
                       AND project_id = $4
                       AND environment_id = $5
                       AND lease_owner = $6
                       AND state = 'started'",
                )
                .bind(lease_ttl)
                .bind(&request.ledger_id)
                .bind(&request.tenant.organization_id)
                .bind(&request.tenant.project_id)
                .bind(&request.tenant.environment_id)
                .bind(&request.lease_owner)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(())
            }
        }
    }

    pub(crate) async fn reconcile_expired(&self) -> Result<ReconcileResult, AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let now = Instant::now();
                let mut result = ReconcileResult::default();
                let expired_attempt_ids = ledger
                    .attempts
                    .iter()
                    .filter(|(_, record)| !record.terminal && record.lease_expires_at <= now)
                    .map(|(attempt_id, _)| attempt_id.clone())
                    .collect::<Vec<_>>();
                for attempt_id in expired_attempt_ids {
                    if let Some(record) = ledger.attempts.get_mut(&attempt_id) {
                        record.mark_unreconciled(true);
                    }
                    release_memory_budget(&mut ledger, &attempt_id)?;
                    result.attempts += 1;
                }
                for record in ledger.requests.values_mut().filter(|record| {
                    !record.record.terminal && record.record.lease_expires_at <= now
                }) {
                    record.record.mark_unreconciled(false);
                    result.requests += 1;
                }
                Ok(result)
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let expired_attempts = sqlx::query(
                    "UPDATE modelport_provider_attempts
                     SET state = 'failed',
                         status_code = 500,
                         terminal_reason = 'lease_expired_unreconciled',
                         error_message = 'ledger lease expired before a terminal Provider outcome was persisted',
                         billing_mode = 'unreconciled',
                         chargeable = false,
                         updated_at = now(),
                         completed_at = now()
                     WHERE state = 'started'
                       AND lease_expires_at <= now()
                     RETURNING attempt_id, organization_id, project_id, environment_id",
                )
                .fetch_all(&mut *transaction)
                .await?;
                for row in &expired_attempts {
                    release_budget_pg(
                        &mut transaction,
                        row.try_get("attempt_id")?,
                        &TenantKey {
                            organization_id: row.try_get("organization_id")?,
                            project_id: row.try_get("project_id")?,
                            environment_id: row.try_get("environment_id")?,
                        },
                    )
                    .await?;
                }
                let requests = sqlx::query(
                    "UPDATE modelport_gateway_requests
                     SET state = 'failed',
                         status_code = 500,
                         terminal_reason = 'lease_expired_unreconciled',
                         error_message = 'ledger lease expired before a terminal request outcome was persisted',
                         billing_mode = 'unreconciled',
                         chargeable = false,
                         updated_at = now(),
                         completed_at = now()
                     WHERE state = 'started'
                       AND lease_expires_at <= now()",
                )
                .execute(&mut *transaction)
                .await?
                .rows_affected();
                transaction.commit().await?;
                Ok(ReconcileResult {
                    requests,
                    attempts: usize_to_u64(expired_attempts.len()),
                })
            }
        }
    }

    pub(crate) async fn overview(&self) -> Result<EnterpriseLedgerOverview, AppError> {
        let mut overview = EnterpriseLedgerOverview {
            backend: self.backend_name(),
            location: self.location().to_owned(),
            lease_ttl_secs: self.lease_ttl.as_secs(),
            reconcile_interval_secs: self.reconcile_interval.as_secs(),
            total_requests: 0,
            started_requests: 0,
            completed_requests: 0,
            failed_requests: 0,
            cancelled_requests: 0,
            unreconciled_requests: 0,
            idempotent_requests: 0,
            active_leases: 0,
            expired_leases: 0,
            chargeable_requests: 0,
            total_cost_microunits: 0,
            organization_count: 0,
            project_count: 0,
            environment_count: 0,
        };

        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let now = Instant::now();
                let mut organizations = HashSet::new();
                let mut projects = HashSet::new();
                let mut environments = HashSet::new();
                for request in ledger.requests.values() {
                    overview.total_requests += 1;
                    match request.record.state.as_str() {
                        "started" => overview.started_requests += 1,
                        "completed" => overview.completed_requests += 1,
                        "failed" => overview.failed_requests += 1,
                        "cancelled" => overview.cancelled_requests += 1,
                        _ => {}
                    }
                    if request.record.terminal_reason.as_deref()
                        == Some("lease_expired_unreconciled")
                    {
                        overview.unreconciled_requests += 1;
                    }
                    if request.idempotency_key_hash.is_some() {
                        overview.idempotent_requests += 1;
                    }
                    if !request.record.terminal {
                        if request.record.lease_expires_at > now {
                            overview.active_leases += 1;
                        } else {
                            overview.expired_leases += 1;
                        }
                    }
                    if request.record.chargeable {
                        overview.chargeable_requests += 1;
                    }
                    overview.total_cost_microunits = overview
                        .total_cost_microunits
                        .saturating_add(request.record.cost_amount_microunits);
                    organizations.insert(request.record.tenant.organization_id.clone());
                    projects.insert((
                        request.record.tenant.organization_id.clone(),
                        request.record.tenant.project_id.clone(),
                    ));
                    environments.insert((
                        request.record.tenant.organization_id.clone(),
                        request.record.tenant.project_id.clone(),
                        request.record.tenant.environment_id.clone(),
                    ));
                }
                overview.organization_count = usize_to_i64(organizations.len());
                overview.project_count = usize_to_i64(projects.len());
                overview.environment_count = usize_to_i64(environments.len());
            }
            LedgerBackend::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT
                        count(*)::bigint AS total_requests,
                        count(*) FILTER (WHERE state = 'started')::bigint AS started_requests,
                        count(*) FILTER (WHERE state = 'completed')::bigint AS completed_requests,
                        count(*) FILTER (WHERE state = 'failed')::bigint AS failed_requests,
                        count(*) FILTER (WHERE state = 'cancelled')::bigint AS cancelled_requests,
                        count(*) FILTER (WHERE terminal_reason = 'lease_expired_unreconciled')::bigint AS unreconciled_requests,
                        count(*) FILTER (WHERE idempotency_key_hash IS NOT NULL)::bigint AS idempotent_requests,
                        count(*) FILTER (WHERE state = 'started' AND lease_expires_at > now())::bigint AS active_leases,
                        count(*) FILTER (WHERE state = 'started' AND lease_expires_at <= now())::bigint AS expired_leases,
                        count(*) FILTER (WHERE chargeable)::bigint AS chargeable_requests,
                        COALESCE(sum(cost_amount_microunits), 0)::bigint AS total_cost_microunits,
                        count(DISTINCT organization_id)::bigint AS organization_count,
                        count(DISTINCT (organization_id, project_id))::bigint AS project_count,
                        count(DISTINCT (organization_id, project_id, environment_id))::bigint AS environment_count
                     FROM modelport_gateway_requests",
                )
                .fetch_one(pool)
                .await?;
                overview.total_requests = row.try_get("total_requests")?;
                overview.started_requests = row.try_get("started_requests")?;
                overview.completed_requests = row.try_get("completed_requests")?;
                overview.failed_requests = row.try_get("failed_requests")?;
                overview.cancelled_requests = row.try_get("cancelled_requests")?;
                overview.unreconciled_requests = row.try_get("unreconciled_requests")?;
                overview.idempotent_requests = row.try_get("idempotent_requests")?;
                overview.active_leases = row.try_get("active_leases")?;
                overview.expired_leases = row.try_get("expired_leases")?;
                overview.chargeable_requests = row.try_get("chargeable_requests")?;
                overview.total_cost_microunits = row.try_get("total_cost_microunits")?;
                overview.organization_count = row.try_get("organization_count")?;
                overview.project_count = row.try_get("project_count")?;
                overview.environment_count = row.try_get("environment_count")?;
            }
        }
        Ok(overview)
    }

    pub(crate) async fn list_requests(
        &self,
        query: &EnterpriseLedgerQuery,
    ) -> Result<EnterpriseRequestPage, AppError> {
        let query = query.normalized()?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let mut requests = ledger
                    .requests
                    .iter()
                    .filter(|(_, request)| query.matches_memory(request))
                    .map(|(ledger_id, request)| {
                        memory_request_row(
                            ledger_id,
                            request,
                            usize_to_i64(
                                ledger
                                    .attempts
                                    .values()
                                    .filter(|attempt| attempt.request_ledger_id == *ledger_id)
                                    .count(),
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                requests.sort_by(|left, right| {
                    right
                        .created_at_ms
                        .cmp(&left.created_at_ms)
                        .then_with(|| right.ledger_id.cmp(&left.ledger_id))
                });
                let total = usize_to_i64(requests.len());
                let start = query.offset().min(requests.len());
                let end = start.saturating_add(query.page_size).min(requests.len());
                Ok(EnterpriseRequestPage {
                    requests: requests[start..end].to_vec(),
                    total,
                    page: query.page,
                    page_size: query.page_size,
                })
            }
            LedgerBackend::Postgres(pool) => {
                let count = sqlx::query_scalar::<_, i64>(REQUEST_COUNT_SQL)
                    .bind(query.state.as_deref())
                    .bind(query.protocol.as_deref())
                    .bind(query.organization_id.as_deref())
                    .bind(query.project_id.as_deref())
                    .bind(query.environment_id.as_deref())
                    .bind(query.search.as_deref())
                    .fetch_one(pool)
                    .await?;
                let rows = sqlx::query(REQUEST_LIST_SQL)
                    .bind(query.state.as_deref())
                    .bind(query.protocol.as_deref())
                    .bind(query.organization_id.as_deref())
                    .bind(query.project_id.as_deref())
                    .bind(query.environment_id.as_deref())
                    .bind(query.search.as_deref())
                    .bind(usize_to_i64(query.page_size))
                    .bind(usize_to_i64(query.offset()))
                    .fetch_all(pool)
                    .await?;
                Ok(EnterpriseRequestPage {
                    requests: rows
                        .iter()
                        .map(request_row_from_pg)
                        .collect::<Result<_, _>>()?,
                    total: count,
                    page: query.page,
                    page_size: query.page_size,
                })
            }
        }
    }

    pub(crate) async fn request_detail(
        &self,
        ledger_id: &str,
    ) -> Result<Option<EnterpriseRequestDetail>, AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let Some(request) = ledger.requests.get(ledger_id) else {
                    return Ok(None);
                };
                let mut attempts = ledger
                    .attempts
                    .iter()
                    .filter(|(_, attempt)| attempt.request_ledger_id == ledger_id)
                    .map(|(attempt_id, attempt)| memory_attempt_row(attempt_id, attempt))
                    .collect::<Vec<_>>();
                attempts.sort_by_key(|attempt| attempt.created_at_ms);
                Ok(Some(EnterpriseRequestDetail {
                    request: memory_request_row(ledger_id, request, usize_to_i64(attempts.len())),
                    attempts,
                }))
            }
            LedgerBackend::Postgres(pool) => {
                let Some(row) = sqlx::query(REQUEST_DETAIL_SQL)
                    .bind(ledger_id)
                    .fetch_optional(pool)
                    .await?
                else {
                    return Ok(None);
                };
                let request = request_row_from_pg(&row)?;
                let attempt_rows = sqlx::query(ATTEMPT_LIST_SQL)
                    .bind(ledger_id)
                    .fetch_all(pool)
                    .await?;
                Ok(Some(EnterpriseRequestDetail {
                    request,
                    attempts: attempt_rows
                        .iter()
                        .map(attempt_row_from_pg)
                        .collect::<Result<_, _>>()?,
                }))
            }
        }
    }

    pub(crate) async fn budget_view(
        &self,
        scope: &EnterpriseBudgetScopeQuery,
    ) -> Result<EnterpriseBudgetView, AppError> {
        let tenant = scope.tenant()?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let account = ledger
                    .budget_accounts
                    .entry(tenant.clone())
                    .or_insert_with(|| MemoryBudgetAccount {
                        updated_at_ms: now_millis(),
                        ..MemoryBudgetAccount::default()
                    })
                    .clone();
                let recent_events = ledger
                    .budget_events
                    .iter()
                    .rev()
                    .filter(|event| event_matches_tenant(event, &tenant))
                    .take(50)
                    .cloned()
                    .collect();
                Ok(EnterpriseBudgetView {
                    account: memory_budget_account(&tenant, &account),
                    recent_events,
                })
            }
            LedgerBackend::Postgres(pool) => {
                let account = sqlx::query(BUDGET_ACCOUNT_SQL)
                    .bind(&tenant.organization_id)
                    .bind(&tenant.project_id)
                    .bind(&tenant.environment_id)
                    .fetch_optional(pool)
                    .await?
                    .map(|row| budget_account_from_pg(&row))
                    .transpose()?
                    .unwrap_or_else(|| empty_budget_account(&tenant));
                let events = sqlx::query(BUDGET_EVENTS_SQL)
                    .bind(&tenant.organization_id)
                    .bind(&tenant.project_id)
                    .bind(&tenant.environment_id)
                    .fetch_all(pool)
                    .await?;
                Ok(EnterpriseBudgetView {
                    account,
                    recent_events: events
                        .iter()
                        .map(budget_event_from_pg)
                        .collect::<Result<_, _>>()?,
                })
            }
        }
    }

    pub(crate) async fn update_budget(
        &self,
        input: &EnterpriseBudgetUpdate,
    ) -> Result<EnterpriseBudgetView, AppError> {
        let tenant = input.tenant()?;
        let limit = input.validated_limit()?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let account = ledger
                    .budget_accounts
                    .entry(tenant.clone())
                    .or_insert_with(|| MemoryBudgetAccount {
                        updated_at_ms: now_millis(),
                        ..MemoryBudgetAccount::default()
                    });
                account.limit_microunits = limit;
                account.version = account.version.saturating_add(1);
                account.updated_at_ms = now_millis();
            }
            LedgerBackend::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO modelport_budget_accounts (
                        organization_id, project_id, environment_id, currency, limit_microunits
                     ) VALUES ($1, $2, $3, 'USD', $4)
                     ON CONFLICT (organization_id, project_id, environment_id, currency)
                     DO UPDATE SET
                         limit_microunits = EXCLUDED.limit_microunits,
                         version = modelport_budget_accounts.version + 1,
                         updated_at = now()",
                )
                .bind(&tenant.organization_id)
                .bind(&tenant.project_id)
                .bind(&tenant.environment_id)
                .bind(limit)
                .execute(pool)
                .await?;
            }
        }
        self.budget_view(&EnterpriseBudgetScopeQuery::from(&tenant))
            .await
    }

    pub(crate) async fn adjust_budget(
        &self,
        input: &EnterpriseBudgetAdjustmentInput,
        actor_id: &str,
    ) -> Result<EnterpriseBudgetView, AppError> {
        let tenant = input.tenant()?;
        input.validate()?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let account = ledger
                    .budget_accounts
                    .entry(tenant.clone())
                    .or_insert_with(|| MemoryBudgetAccount {
                        updated_at_ms: now_millis(),
                        ..MemoryBudgetAccount::default()
                    });
                account.settled_microunits = account
                    .settled_microunits
                    .checked_add(input.delta_microunits)
                    .filter(|value| *value >= 0)
                    .ok_or_else(|| {
                        AppError::InvalidRequest(
                            "budget adjustment cannot make settled spend negative".to_owned(),
                        )
                    })?;
                account.version = account.version.saturating_add(1);
                account.updated_at_ms = now_millis();
                ledger
                    .budget_events
                    .push(adjustment_event(&tenant, input, actor_id));
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                sqlx::query(
                    "INSERT INTO modelport_budget_accounts (
                        organization_id, project_id, environment_id, currency
                     ) VALUES ($1, $2, $3, 'USD')
                     ON CONFLICT (organization_id, project_id, environment_id, currency)
                     DO NOTHING",
                )
                .bind(&tenant.organization_id)
                .bind(&tenant.project_id)
                .bind(&tenant.environment_id)
                .execute(&mut *transaction)
                .await?;
                let updated = sqlx::query(
                    "UPDATE modelport_budget_accounts
                     SET settled_microunits = settled_microunits + $1,
                         version = version + 1,
                         updated_at = now()
                     WHERE organization_id = $2
                       AND project_id = $3
                       AND environment_id = $4
                       AND currency = 'USD'
                       AND settled_microunits + $1 >= 0",
                )
                .bind(input.delta_microunits)
                .bind(&tenant.organization_id)
                .bind(&tenant.project_id)
                .bind(&tenant.environment_id)
                .execute(&mut *transaction)
                .await?;
                if updated.rows_affected() != 1 {
                    return Err(AppError::InvalidRequest(
                        "budget adjustment cannot make settled spend negative".to_owned(),
                    ));
                }
                sqlx::query(
                    "INSERT INTO modelport_budget_events (
                        event_id,
                        organization_id, project_id, environment_id, currency,
                        event_type, reserved_delta_microunits, settled_delta_microunits,
                        evidence_source, reason, actor_id
                     ) VALUES ($1, $2, $3, $4, 'USD', 'adjustment', 0, $5, $6, $7, $8)",
                )
                .bind(format!("bev_{}", Uuid::new_v4().simple()))
                .bind(&tenant.organization_id)
                .bind(&tenant.project_id)
                .bind(&tenant.environment_id)
                .bind(input.delta_microunits)
                .bind(&input.evidence_reference)
                .bind(&input.reason)
                .bind(actor_id)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
            }
        }
        self.budget_view(&EnterpriseBudgetScopeQuery::from(&tenant))
            .await
    }

    fn backend_name(&self) -> &'static str {
        match self.backend.as_ref() {
            LedgerBackend::Memory(_) => "memory",
            LedgerBackend::Postgres(_) => "postgres",
        }
    }

    pub(crate) fn spawn_reconciler(&self) {
        if matches!(self.backend.as_ref(), LedgerBackend::Memory(_)) {
            return;
        }
        let ledger = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(ledger.reconcile_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                match ledger.reconcile_expired().await {
                    Ok(result) if result.requests > 0 || result.attempts > 0 => info!(
                        requests = result.requests,
                        attempts = result.attempts,
                        "reconciled expired inference ledger leases"
                    ),
                    Ok(_) => {}
                    Err(err) => error!(error = %err, "failed to reconcile expired ledger leases"),
                }
            }
        });
    }

    #[cfg(test)]
    pub(crate) async fn incomplete_requests(&self, tenant: &TenantScope) -> usize {
        let LedgerBackend::Memory(ledger) = self.backend.as_ref() else {
            return 0;
        };
        let tenant = TenantKey::from(tenant);
        ledger
            .lock()
            .expect("enterprise ledger lock poisoned")
            .requests
            .values()
            .filter(|record| record.record.tenant == tenant && !record.record.terminal)
            .count()
    }
}

async fn update_terminal_record_pg(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_record: bool,
    id: &str,
    tenant: &TenantKey,
    lease_owner: &str,
    outcome: &LedgerOutcome,
) -> Result<bool, AppError> {
    let table = if request_record {
        "modelport_gateway_requests"
    } else {
        "modelport_provider_attempts"
    };
    let id_column = if request_record {
        "ledger_id"
    } else {
        "attempt_id"
    };
    let query = format!(
        "UPDATE {table}
         SET state = $1,
             status_code = $2,
             terminal_reason = $3,
             error_message = $4,
             input_tokens = $5,
             output_tokens = $6,
             cache_write_tokens = $7,
             cache_read_tokens = $8,
             cost_amount_microunits = $9,
             billing_mode = $10,
             chargeable = $11,
             updated_at = now(),
             completed_at = now()
         WHERE {id_column} = $12
           AND organization_id = $13
           AND project_id = $14
           AND environment_id = $15
           AND lease_owner = $16
           AND state = 'started'"
    );
    let result = sqlx::query(&query)
        .bind(outcome.state)
        .bind(i32::from(outcome.status_code))
        .bind(&outcome.terminal_reason)
        .bind(&outcome.error_message)
        .bind(to_i64(outcome.estimate.input_tokens))
        .bind(to_i64(outcome.estimate.output_tokens))
        .bind(to_i64(outcome.estimate.cache_write_tokens))
        .bind(to_i64(outcome.estimate.cache_read_tokens))
        .bind(cost_microunits(outcome.estimate.cost_estimate))
        .bind(&outcome.billing_mode)
        .bind(outcome.chargeable)
        .bind(id)
        .bind(&tenant.organization_id)
        .bind(&tenant.project_id)
        .bind(&tenant.environment_id)
        .bind(lease_owner)
        .execute(&mut **transaction)
        .await?;
    Ok(result.rows_affected() == 1)
}

fn settle_memory_budget(
    ledger: &mut MemoryLedger,
    attempt: &LedgerAttempt,
    outcome: &LedgerOutcome,
) -> Result<(), AppError> {
    let settled_microunits = cost_microunits(outcome.estimate.cost_estimate);
    let now = now_millis();
    let reserved_microunits = {
        let reservation = ledger
            .budget_reservations
            .get_mut(&attempt.attempt_id)
            .ok_or_else(|| AppError::Database("budget reservation is missing".to_owned()))?;
        if reservation.state != "reserved" {
            return Ok(());
        }
        reservation.state = "settled".to_owned();
        reservation.settled_microunits = settled_microunits;
        reservation.updated_at_ms = now;
        reservation.terminal_at_ms = Some(now);
        reservation.reserved_microunits
    };
    let account = ledger
        .budget_accounts
        .get_mut(&attempt.tenant)
        .ok_or_else(|| AppError::Database("budget account is missing".to_owned()))?;
    account.reserved_microunits = account
        .reserved_microunits
        .checked_sub(reserved_microunits)
        .ok_or_else(|| AppError::Database("budget reserved balance underflow".to_owned()))?;
    account.settled_microunits = account
        .settled_microunits
        .checked_add(settled_microunits)
        .ok_or_else(|| AppError::Database("budget settled balance overflow".to_owned()))?;
    account.version = account.version.saturating_add(1);
    account.updated_at_ms = now;
    ledger.budget_events.push(budget_event(
        attempt,
        "settled",
        -reserved_microunits,
        settled_microunits,
        outcome_evidence_source(outcome),
        Some(&outcome.billing_mode),
        Some(&outcome.terminal_reason),
        None,
        outcome.estimate,
    ));
    Ok(())
}

fn release_memory_budget(ledger: &mut MemoryLedger, attempt_id: &str) -> Result<(), AppError> {
    let now = now_millis();
    let (attempt, reserved_microunits) = {
        let reservation = ledger
            .budget_reservations
            .get_mut(attempt_id)
            .ok_or_else(|| AppError::Database("budget reservation is missing".to_owned()))?;
        if reservation.state != "reserved" {
            return Ok(());
        }
        reservation.state = "released".to_owned();
        reservation.updated_at_ms = now;
        reservation.terminal_at_ms = Some(now);
        (
            LedgerAttempt {
                attempt_id: reservation.attempt_id.clone(),
                request_ledger_id: reservation.request_ledger_id.clone(),
                reservation_id: reservation.reservation_id.clone(),
                tenant: reservation.tenant.clone(),
                lease_owner: String::new(),
            },
            reservation.reserved_microunits,
        )
    };
    let account = ledger
        .budget_accounts
        .get_mut(&attempt.tenant)
        .ok_or_else(|| AppError::Database("budget account is missing".to_owned()))?;
    account.reserved_microunits = account
        .reserved_microunits
        .checked_sub(reserved_microunits)
        .ok_or_else(|| AppError::Database("budget reserved balance underflow".to_owned()))?;
    account.version = account.version.saturating_add(1);
    account.updated_at_ms = now;
    ledger.budget_events.push(budget_event(
        &attempt,
        "released",
        -reserved_microunits,
        0,
        "lease-expired",
        Some("unreconciled"),
        Some("expired Provider Attempt lease released its budget reservation"),
        None,
        UsageEstimate::default(),
    ));
    Ok(())
}

async fn settle_budget_pg(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attempt: &LedgerAttempt,
    outcome: &LedgerOutcome,
) -> Result<(), AppError> {
    let settled_microunits = cost_microunits(outcome.estimate.cost_estimate);
    let reservation = sqlx::query_as::<_, (String, i64)>(
        "UPDATE modelport_budget_reservations
         SET state = 'settled',
             settled_microunits = $1,
             evidence_source = $2,
             billing_mode = $3,
             updated_at = now(),
             terminal_at = now()
         WHERE attempt_id = $4
           AND organization_id = $5
           AND project_id = $6
           AND environment_id = $7
           AND state = 'reserved'
         RETURNING reservation_id, reserved_microunits",
    )
    .bind(settled_microunits)
    .bind(outcome_evidence_source(outcome))
    .bind(&outcome.billing_mode)
    .bind(&attempt.attempt_id)
    .bind(&attempt.tenant.organization_id)
    .bind(&attempt.tenant.project_id)
    .bind(&attempt.tenant.environment_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::Database("open budget reservation is missing".to_owned()))?;
    let account = sqlx::query(
        "UPDATE modelport_budget_accounts
         SET reserved_microunits = reserved_microunits - $1,
             settled_microunits = settled_microunits + $2,
             version = version + 1,
             updated_at = now()
         WHERE organization_id = $3
           AND project_id = $4
           AND environment_id = $5
           AND currency = 'USD'
           AND reserved_microunits >= $1",
    )
    .bind(reservation.1)
    .bind(settled_microunits)
    .bind(&attempt.tenant.organization_id)
    .bind(&attempt.tenant.project_id)
    .bind(&attempt.tenant.environment_id)
    .execute(&mut **transaction)
    .await?;
    if account.rows_affected() != 1 {
        return Err(AppError::Database(
            "budget account reserved balance invariant failed".to_owned(),
        ));
    }
    insert_budget_event_pg(
        transaction,
        attempt,
        "settled",
        -reservation.1,
        settled_microunits,
        outcome_evidence_source(outcome),
        Some(&outcome.billing_mode),
        Some(&outcome.terminal_reason),
        None,
        outcome.estimate,
    )
    .await
}

async fn release_budget_pg(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attempt_id: &str,
    tenant: &TenantKey,
) -> Result<(), AppError> {
    let reservation = sqlx::query_as::<_, (String, String, i64)>(
        "UPDATE modelport_budget_reservations
         SET state = 'released',
             evidence_source = 'lease-expired',
             billing_mode = 'unreconciled',
             updated_at = now(),
             terminal_at = now()
         WHERE attempt_id = $1
           AND organization_id = $2
           AND project_id = $3
           AND environment_id = $4
           AND state = 'reserved'
         RETURNING reservation_id, request_ledger_id, reserved_microunits",
    )
    .bind(attempt_id)
    .bind(&tenant.organization_id)
    .bind(&tenant.project_id)
    .bind(&tenant.environment_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some((reservation_id, request_ledger_id, reserved_microunits)) = reservation else {
        return Ok(());
    };
    let account = sqlx::query(
        "UPDATE modelport_budget_accounts
         SET reserved_microunits = reserved_microunits - $1,
             version = version + 1,
             updated_at = now()
         WHERE organization_id = $2
           AND project_id = $3
           AND environment_id = $4
           AND currency = 'USD'
           AND reserved_microunits >= $1",
    )
    .bind(reserved_microunits)
    .bind(&tenant.organization_id)
    .bind(&tenant.project_id)
    .bind(&tenant.environment_id)
    .execute(&mut **transaction)
    .await?;
    if account.rows_affected() != 1 {
        return Err(AppError::Database(
            "budget account reserved balance invariant failed during release".to_owned(),
        ));
    }
    insert_budget_event_pg(
        transaction,
        &LedgerAttempt {
            attempt_id: attempt_id.to_owned(),
            request_ledger_id,
            reservation_id,
            tenant: tenant.clone(),
            lease_owner: String::new(),
        },
        "released",
        -reserved_microunits,
        0,
        "lease-expired",
        Some("unreconciled"),
        Some("expired Provider Attempt lease released its budget reservation"),
        None,
        UsageEstimate::default(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_budget_event_pg(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attempt: &LedgerAttempt,
    event_type: &str,
    reserved_delta_microunits: i64,
    settled_delta_microunits: i64,
    evidence_source: &str,
    billing_mode: Option<&str>,
    reason: Option<&str>,
    actor_id: Option<&str>,
    estimate: UsageEstimate,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO modelport_budget_events (
            event_id,
            organization_id, project_id, environment_id, currency,
            reservation_id, request_ledger_id, attempt_id,
            event_type, reserved_delta_microunits, settled_delta_microunits,
            evidence_source, billing_mode, reason, actor_id,
            input_tokens, output_tokens, cache_write_tokens, cache_read_tokens
         ) VALUES (
            $1, $2, $3, $4, 'USD', $5, $6, $7,
            $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18
         )",
    )
    .bind(format!("bev_{}", Uuid::new_v4().simple()))
    .bind(&attempt.tenant.organization_id)
    .bind(&attempt.tenant.project_id)
    .bind(&attempt.tenant.environment_id)
    .bind(&attempt.reservation_id)
    .bind(&attempt.request_ledger_id)
    .bind(&attempt.attempt_id)
    .bind(event_type)
    .bind(reserved_delta_microunits)
    .bind(settled_delta_microunits)
    .bind(evidence_source)
    .bind(billing_mode)
    .bind(reason)
    .bind(actor_id)
    .bind(to_i64(estimate.input_tokens))
    .bind(to_i64(estimate.output_tokens))
    .bind(to_i64(estimate.cache_write_tokens))
    .bind(to_i64(estimate.cache_read_tokens))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn budget_event(
    attempt: &LedgerAttempt,
    event_type: &str,
    reserved_delta_microunits: i64,
    settled_delta_microunits: i64,
    evidence_source: &str,
    billing_mode: Option<&str>,
    reason: Option<&str>,
    actor_id: Option<&str>,
    estimate: UsageEstimate,
) -> EnterpriseBudgetEvent {
    EnterpriseBudgetEvent {
        event_id: format!("bev_{}", Uuid::new_v4().simple()),
        organization_id: attempt.tenant.organization_id.clone(),
        project_id: attempt.tenant.project_id.clone(),
        environment_id: attempt.tenant.environment_id.clone(),
        currency: "USD".to_owned(),
        reservation_id: Some(attempt.reservation_id.clone()),
        request_ledger_id: Some(attempt.request_ledger_id.clone()),
        attempt_id: Some(attempt.attempt_id.clone()),
        event_type: event_type.to_owned(),
        reserved_delta_microunits,
        settled_delta_microunits,
        evidence_source: evidence_source.to_owned(),
        billing_mode: billing_mode.map(str::to_owned),
        reason: reason.map(str::to_owned),
        actor_id: actor_id.map(str::to_owned),
        input_tokens: to_i64(estimate.input_tokens),
        output_tokens: to_i64(estimate.output_tokens),
        cache_write_tokens: to_i64(estimate.cache_write_tokens),
        cache_read_tokens: to_i64(estimate.cache_read_tokens),
        created_at_ms: now_millis(),
    }
}

fn outcome_evidence_source(outcome: &LedgerOutcome) -> &'static str {
    if outcome.billing_mode == "upstream-returned" {
        "provider-usage"
    } else {
        "local-estimate"
    }
}

fn budget_exceeded(account: &MemoryBudgetAccount, requested: i64) -> AppError {
    let available = account
        .limit_microunits
        .unwrap_or(i64::MAX)
        .saturating_sub(account.settled_microunits)
        .saturating_sub(account.reserved_microunits);
    AppError::QuotaExceeded(format!(
        "enterprise budget has {available} microunits available; reservation requires {requested}"
    ))
}

const REQUEST_COUNT_SQL: &str = "SELECT count(*)::bigint
    FROM modelport_gateway_requests r
    WHERE
        ($1::text IS NULL OR r.state = $1)
        AND ($2::text IS NULL OR r.client_protocol = $2)
        AND ($3::text IS NULL OR r.organization_id = $3)
        AND ($4::text IS NULL OR r.project_id = $4)
        AND ($5::text IS NULL OR r.environment_id = $5)
        AND (
            $6::text IS NULL
            OR r.ledger_id ILIKE '%' || $6 || '%'
            OR r.request_id ILIKE '%' || $6 || '%'
            OR r.principal_id ILIKE '%' || $6 || '%'
            OR r.requested_model ILIKE '%' || $6 || '%'
            OR r.organization_id ILIKE '%' || $6 || '%'
            OR r.project_id ILIKE '%' || $6 || '%'
            OR r.environment_id ILIKE '%' || $6 || '%'
            OR COALESCE(r.terminal_reason, '') ILIKE '%' || $6 || '%'
            OR COALESCE(r.error_message, '') ILIKE '%' || $6 || '%'
        )";

const REQUEST_LIST_SQL: &str = "SELECT
        r.ledger_id, r.request_id,
        r.organization_id, r.project_id, r.environment_id,
        r.principal_id, r.client_protocol, r.requested_model, r.stream,
        r.state, r.status_code, r.terminal_reason, r.error_message,
        r.input_tokens, r.output_tokens, r.cache_write_tokens, r.cache_read_tokens,
        r.cost_amount_microunits, r.currency, r.billing_mode, r.chargeable,
        (r.idempotency_key_hash IS NOT NULL) AS has_idempotency_key,
        r.lease_owner,
        (EXTRACT(EPOCH FROM r.lease_expires_at) * 1000)::bigint AS lease_expires_at_ms,
        (EXTRACT(EPOCH FROM r.created_at) * 1000)::bigint AS created_at_ms,
        (EXTRACT(EPOCH FROM r.updated_at) * 1000)::bigint AS updated_at_ms,
        (EXTRACT(EPOCH FROM r.completed_at) * 1000)::bigint AS completed_at_ms,
        (SELECT count(*) FROM modelport_provider_attempts a
         WHERE a.request_ledger_id = r.ledger_id
           AND a.organization_id = r.organization_id
           AND a.project_id = r.project_id
           AND a.environment_id = r.environment_id)::bigint AS attempt_count
    FROM modelport_gateway_requests r
    WHERE
        ($1::text IS NULL OR r.state = $1)
        AND ($2::text IS NULL OR r.client_protocol = $2)
        AND ($3::text IS NULL OR r.organization_id = $3)
        AND ($4::text IS NULL OR r.project_id = $4)
        AND ($5::text IS NULL OR r.environment_id = $5)
        AND (
            $6::text IS NULL
            OR r.ledger_id ILIKE '%' || $6 || '%'
            OR r.request_id ILIKE '%' || $6 || '%'
            OR r.principal_id ILIKE '%' || $6 || '%'
            OR r.requested_model ILIKE '%' || $6 || '%'
            OR r.organization_id ILIKE '%' || $6 || '%'
            OR r.project_id ILIKE '%' || $6 || '%'
            OR r.environment_id ILIKE '%' || $6 || '%'
            OR COALESCE(r.terminal_reason, '') ILIKE '%' || $6 || '%'
            OR COALESCE(r.error_message, '') ILIKE '%' || $6 || '%'
        )
    ORDER BY r.created_at DESC, r.ledger_id DESC
    LIMIT $7 OFFSET $8";

const REQUEST_DETAIL_SQL: &str = "SELECT
        r.ledger_id, r.request_id,
        r.organization_id, r.project_id, r.environment_id,
        r.principal_id, r.client_protocol, r.requested_model, r.stream,
        r.state, r.status_code, r.terminal_reason, r.error_message,
        r.input_tokens, r.output_tokens, r.cache_write_tokens, r.cache_read_tokens,
        r.cost_amount_microunits, r.currency, r.billing_mode, r.chargeable,
        (r.idempotency_key_hash IS NOT NULL) AS has_idempotency_key,
        r.lease_owner,
        (EXTRACT(EPOCH FROM r.lease_expires_at) * 1000)::bigint AS lease_expires_at_ms,
        (EXTRACT(EPOCH FROM r.created_at) * 1000)::bigint AS created_at_ms,
        (EXTRACT(EPOCH FROM r.updated_at) * 1000)::bigint AS updated_at_ms,
        (EXTRACT(EPOCH FROM r.completed_at) * 1000)::bigint AS completed_at_ms,
        (SELECT count(*) FROM modelport_provider_attempts a
         WHERE a.request_ledger_id = r.ledger_id
           AND a.organization_id = r.organization_id
           AND a.project_id = r.project_id
           AND a.environment_id = r.environment_id)::bigint AS attempt_count
    FROM modelport_gateway_requests r
    WHERE r.ledger_id = $1";

const ATTEMPT_LIST_SQL: &str = "SELECT
        attempt_id, request_ledger_id,
        organization_id, project_id, environment_id,
        provider_id, resolved_model, provider_protocol,
        state, status_code, terminal_reason, error_message,
        input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
        cost_amount_microunits, currency, billing_mode, chargeable,
        lease_owner,
        (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::bigint AS lease_expires_at_ms,
        (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_ms,
        (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_ms,
        (EXTRACT(EPOCH FROM completed_at) * 1000)::bigint AS completed_at_ms
    FROM modelport_provider_attempts
    WHERE request_ledger_id = $1
    ORDER BY created_at, attempt_id";

const BUDGET_ACCOUNT_SQL: &str = "SELECT
        organization_id, project_id, environment_id, currency,
        limit_microunits, reserved_microunits, settled_microunits, version,
        (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_ms
    FROM modelport_budget_accounts
    WHERE organization_id = $1
      AND project_id = $2
      AND environment_id = $3
      AND currency = 'USD'";

const BUDGET_EVENTS_SQL: &str = "SELECT
        event_id, organization_id, project_id, environment_id, currency,
        reservation_id, request_ledger_id, attempt_id, event_type,
        reserved_delta_microunits, settled_delta_microunits,
        evidence_source, billing_mode, reason, actor_id,
        input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
        (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_ms
    FROM modelport_budget_events
    WHERE organization_id = $1
      AND project_id = $2
      AND environment_id = $3
      AND currency = 'USD'
    ORDER BY created_at DESC, event_id DESC
    LIMIT 50";

impl EnterpriseBudgetScopeQuery {
    fn tenant(&self) -> Result<TenantKey, AppError> {
        match (
            self.organization_id.as_deref(),
            self.project_id.as_deref(),
            self.environment_id.as_deref(),
        ) {
            (None, None, None) => Ok(TenantKey::local()),
            (Some(organization_id), Some(project_id), Some(environment_id)) => {
                tenant_from_parts(organization_id, project_id, environment_id)
            }
            _ => Err(AppError::InvalidRequest(
                "organizationId, projectId, and environmentId must be supplied together".to_owned(),
            )),
        }
    }
}

impl EnterpriseBudgetUpdate {
    fn tenant(&self) -> Result<TenantKey, AppError> {
        tenant_from_parts(
            &self.organization_id,
            &self.project_id,
            &self.environment_id,
        )
    }

    fn validated_limit(&self) -> Result<Option<i64>, AppError> {
        match (self.unlimited, self.limit_microunits) {
            (true, None) => Ok(None),
            (true, Some(_)) => Err(AppError::InvalidRequest(
                "unlimited budget cannot also provide limitMicrounits".to_owned(),
            )),
            (false, Some(limit)) if limit >= 0 => Ok(Some(limit)),
            (false, _) => Err(AppError::InvalidRequest(
                "a non-negative limitMicrounits value is required unless unlimited is true"
                    .to_owned(),
            )),
        }
    }
}

impl EnterpriseBudgetAdjustmentInput {
    fn tenant(&self) -> Result<TenantKey, AppError> {
        tenant_from_parts(
            &self.organization_id,
            &self.project_id,
            &self.environment_id,
        )
    }

    fn validate(&self) -> Result<(), AppError> {
        if self.delta_microunits == 0 {
            return Err(AppError::InvalidRequest(
                "budget adjustment deltaMicrounits must not be zero".to_owned(),
            ));
        }
        validate_evidence_text("reason", &self.reason, 500)?;
        validate_evidence_text("evidenceReference", &self.evidence_reference, 500)
    }
}

impl From<&TenantKey> for EnterpriseBudgetScopeQuery {
    fn from(tenant: &TenantKey) -> Self {
        Self {
            organization_id: Some(tenant.organization_id.clone()),
            project_id: Some(tenant.project_id.clone()),
            environment_id: Some(tenant.environment_id.clone()),
        }
    }
}

impl TenantKey {
    fn local() -> Self {
        Self {
            organization_id: "org_local".to_owned(),
            project_id: "prj_default".to_owned(),
            environment_id: "env_default".to_owned(),
        }
    }
}

fn tenant_from_parts(
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<TenantKey, AppError> {
    Ok(TenantKey {
        organization_id: validated_tenant_id("organizationId", organization_id)?,
        project_id: validated_tenant_id("projectId", project_id)?,
        environment_id: validated_tenant_id("environmentId", environment_id)?,
    })
}

fn validated_tenant_id(field: &str, value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(AppError::InvalidRequest(format!(
            "budget {field} must contain 1-128 non-control bytes"
        )));
    }
    Ok(value.to_owned())
}

fn validate_evidence_text(field: &str, value: &str, max_len: usize) -> Result<(), AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(AppError::InvalidRequest(format!(
            "budget adjustment {field} must contain 1-{max_len} non-control bytes"
        )));
    }
    Ok(())
}

fn memory_budget_account(
    tenant: &TenantKey,
    account: &MemoryBudgetAccount,
) -> EnterpriseBudgetAccount {
    budget_account(
        tenant,
        account.limit_microunits,
        account.reserved_microunits,
        account.settled_microunits,
        account.version,
        account.updated_at_ms,
    )
}

fn empty_budget_account(tenant: &TenantKey) -> EnterpriseBudgetAccount {
    budget_account(tenant, None, 0, 0, 0, now_millis())
}

fn budget_account(
    tenant: &TenantKey,
    limit_microunits: Option<i64>,
    reserved_microunits: i64,
    settled_microunits: i64,
    version: i64,
    updated_at_ms: i64,
) -> EnterpriseBudgetAccount {
    let consumed = reserved_microunits.saturating_add(settled_microunits);
    EnterpriseBudgetAccount {
        organization_id: tenant.organization_id.clone(),
        project_id: tenant.project_id.clone(),
        environment_id: tenant.environment_id.clone(),
        currency: "USD".to_owned(),
        limit_microunits,
        reserved_microunits,
        settled_microunits,
        available_microunits: limit_microunits.map(|limit| limit.saturating_sub(consumed)),
        utilization_basis_points: limit_microunits.map(|limit| utilization_bps(consumed, limit)),
        version,
        updated_at_ms,
    }
}

fn utilization_bps(consumed: i64, limit: i64) -> i64 {
    if limit == 0 {
        return if consumed == 0 { 0 } else { i64::MAX };
    }
    i64::try_from((i128::from(consumed) * 10_000) / i128::from(limit)).unwrap_or(i64::MAX)
}

fn budget_account_from_pg(row: &PgRow) -> Result<EnterpriseBudgetAccount, sqlx::Error> {
    let tenant = TenantKey {
        organization_id: row.try_get("organization_id")?,
        project_id: row.try_get("project_id")?,
        environment_id: row.try_get("environment_id")?,
    };
    Ok(budget_account(
        &tenant,
        row.try_get("limit_microunits")?,
        row.try_get("reserved_microunits")?,
        row.try_get("settled_microunits")?,
        row.try_get("version")?,
        row.try_get("updated_at_ms")?,
    ))
}

fn budget_event_from_pg(row: &PgRow) -> Result<EnterpriseBudgetEvent, sqlx::Error> {
    Ok(EnterpriseBudgetEvent {
        event_id: row.try_get("event_id")?,
        organization_id: row.try_get("organization_id")?,
        project_id: row.try_get("project_id")?,
        environment_id: row.try_get("environment_id")?,
        currency: row.try_get("currency")?,
        reservation_id: row.try_get("reservation_id")?,
        request_ledger_id: row.try_get("request_ledger_id")?,
        attempt_id: row.try_get("attempt_id")?,
        event_type: row.try_get("event_type")?,
        reserved_delta_microunits: row.try_get("reserved_delta_microunits")?,
        settled_delta_microunits: row.try_get("settled_delta_microunits")?,
        evidence_source: row.try_get("evidence_source")?,
        billing_mode: row.try_get("billing_mode")?,
        reason: row.try_get("reason")?,
        actor_id: row.try_get("actor_id")?,
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        cache_write_tokens: row.try_get("cache_write_tokens")?,
        cache_read_tokens: row.try_get("cache_read_tokens")?,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

fn event_matches_tenant(event: &EnterpriseBudgetEvent, tenant: &TenantKey) -> bool {
    event.organization_id == tenant.organization_id
        && event.project_id == tenant.project_id
        && event.environment_id == tenant.environment_id
}

fn adjustment_event(
    tenant: &TenantKey,
    input: &EnterpriseBudgetAdjustmentInput,
    actor_id: &str,
) -> EnterpriseBudgetEvent {
    EnterpriseBudgetEvent {
        event_id: format!("bev_{}", Uuid::new_v4().simple()),
        organization_id: tenant.organization_id.clone(),
        project_id: tenant.project_id.clone(),
        environment_id: tenant.environment_id.clone(),
        currency: "USD".to_owned(),
        reservation_id: None,
        request_ledger_id: None,
        attempt_id: None,
        event_type: "adjustment".to_owned(),
        reserved_delta_microunits: 0,
        settled_delta_microunits: input.delta_microunits,
        evidence_source: input.evidence_reference.trim().to_owned(),
        billing_mode: None,
        reason: Some(input.reason.trim().to_owned()),
        actor_id: Some(actor_id.to_owned()),
        input_tokens: 0,
        output_tokens: 0,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        created_at_ms: now_millis(),
    }
}

#[derive(Debug)]
struct NormalizedLedgerQuery {
    page: usize,
    page_size: usize,
    state: Option<String>,
    protocol: Option<String>,
    organization_id: Option<String>,
    project_id: Option<String>,
    environment_id: Option<String>,
    search: Option<String>,
}

impl EnterpriseLedgerQuery {
    fn normalized(&self) -> Result<NormalizedLedgerQuery, AppError> {
        let page = self.page.unwrap_or(1);
        if page == 0 || page > 1_000_000 {
            return Err(AppError::InvalidRequest(
                "enterprise ledger page must be between 1 and 1000000".to_owned(),
            ));
        }
        let page_size = self.page_size.unwrap_or(25);
        if !(1..=100).contains(&page_size) {
            return Err(AppError::InvalidRequest(
                "enterprise ledger pageSize must be between 1 and 100".to_owned(),
            ));
        }
        let state = normalized_filter(self.state.as_deref(), "state", 32)?;
        if state
            .as_deref()
            .is_some_and(|value| !matches!(value, "started" | "completed" | "failed" | "cancelled"))
        {
            return Err(AppError::InvalidRequest(
                "enterprise ledger state must be started, completed, failed, or cancelled"
                    .to_owned(),
            ));
        }
        let protocol = normalized_filter(self.protocol.as_deref(), "protocol", 64)?;
        if protocol
            .as_deref()
            .is_some_and(|value| !matches!(value, "anthropic-messages" | "openai-chat-completions"))
        {
            return Err(AppError::InvalidRequest(
                "enterprise ledger protocol must be anthropic-messages or openai-chat-completions"
                    .to_owned(),
            ));
        }
        Ok(NormalizedLedgerQuery {
            page,
            page_size,
            state,
            protocol,
            organization_id: normalized_filter(
                self.organization_id.as_deref(),
                "organizationId",
                128,
            )?,
            project_id: normalized_filter(self.project_id.as_deref(), "projectId", 128)?,
            environment_id: normalized_filter(
                self.environment_id.as_deref(),
                "environmentId",
                128,
            )?,
            search: normalized_filter(self.search.as_deref(), "search", 200)?,
        })
    }
}

impl NormalizedLedgerQuery {
    fn offset(&self) -> usize {
        self.page.saturating_sub(1).saturating_mul(self.page_size)
    }

    fn matches_memory(&self, request: &MemoryRequestRecord) -> bool {
        let record = &request.record;
        if self
            .state
            .as_deref()
            .is_some_and(|value| record.state != value)
            || self
                .protocol
                .as_deref()
                .is_some_and(|value| request.client_protocol != value)
            || self
                .organization_id
                .as_deref()
                .is_some_and(|value| record.tenant.organization_id != value)
            || self
                .project_id
                .as_deref()
                .is_some_and(|value| record.tenant.project_id != value)
            || self
                .environment_id
                .as_deref()
                .is_some_and(|value| record.tenant.environment_id != value)
        {
            return false;
        }
        self.search.as_deref().is_none_or(|search| {
            let search = search.to_lowercase();
            [
                record.request_ledger_id.as_str(),
                request.request_id.as_str(),
                request.principal_id.as_str(),
                request.requested_model.as_str(),
                record.tenant.organization_id.as_str(),
                record.tenant.project_id.as_str(),
                record.tenant.environment_id.as_str(),
                record.terminal_reason.as_deref().unwrap_or_default(),
                record.error_message.as_deref().unwrap_or_default(),
            ]
            .iter()
            .any(|value| value.to_lowercase().contains(&search))
        })
    }
}

fn normalized_filter(
    value: Option<&str>,
    field: &str,
    max_len: usize,
) -> Result<Option<String>, AppError> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    if value.is_some_and(|value| value.len() > max_len || value.chars().any(char::is_control)) {
        return Err(AppError::InvalidRequest(format!(
            "enterprise ledger {field} is invalid or exceeds {max_len} bytes"
        )));
    }
    Ok(value.map(str::to_owned))
}

fn memory_request_row(
    ledger_id: &str,
    request: &MemoryRequestRecord,
    attempt_count: i64,
) -> EnterpriseRequestRow {
    let record = &request.record;
    EnterpriseRequestRow {
        ledger_id: ledger_id.to_owned(),
        request_id: request.request_id.clone(),
        organization_id: record.tenant.organization_id.clone(),
        project_id: record.tenant.project_id.clone(),
        environment_id: record.tenant.environment_id.clone(),
        principal_id: request.principal_id.clone(),
        client_protocol: request.client_protocol.clone(),
        requested_model: request.requested_model.clone(),
        stream: request.stream,
        state: record.state.clone(),
        status_code: record.status_code,
        terminal_reason: record.terminal_reason.clone(),
        error_message: record.error_message.clone(),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_write_tokens: record.cache_write_tokens,
        cache_read_tokens: record.cache_read_tokens,
        cost_amount_microunits: record.cost_amount_microunits,
        currency: "USD".to_owned(),
        billing_mode: record.billing_mode.clone(),
        chargeable: record.chargeable,
        has_idempotency_key: request.idempotency_key_hash.is_some(),
        lease_owner: record.lease_owner.clone(),
        lease_expires_at_ms: record.lease_expires_at_ms,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
        completed_at_ms: record.completed_at_ms,
        attempt_count,
    }
}

fn memory_attempt_row(attempt_id: &str, record: &MemoryRecord) -> EnterpriseAttemptRow {
    EnterpriseAttemptRow {
        attempt_id: attempt_id.to_owned(),
        request_ledger_id: record.request_ledger_id.clone(),
        organization_id: record.tenant.organization_id.clone(),
        project_id: record.tenant.project_id.clone(),
        environment_id: record.tenant.environment_id.clone(),
        provider_id: record.provider_id.clone().unwrap_or_default(),
        resolved_model: record.resolved_model.clone().unwrap_or_default(),
        provider_protocol: record.provider_protocol.clone().unwrap_or_default(),
        state: record.state.clone(),
        status_code: record.status_code,
        terminal_reason: record.terminal_reason.clone(),
        error_message: record.error_message.clone(),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_write_tokens: record.cache_write_tokens,
        cache_read_tokens: record.cache_read_tokens,
        cost_amount_microunits: record.cost_amount_microunits,
        currency: "USD".to_owned(),
        billing_mode: record.billing_mode.clone(),
        chargeable: record.chargeable,
        lease_owner: record.lease_owner.clone(),
        lease_expires_at_ms: record.lease_expires_at_ms,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
        completed_at_ms: record.completed_at_ms,
    }
}

fn request_row_from_pg(row: &PgRow) -> Result<EnterpriseRequestRow, sqlx::Error> {
    Ok(EnterpriseRequestRow {
        ledger_id: row.try_get("ledger_id")?,
        request_id: row.try_get("request_id")?,
        organization_id: row.try_get("organization_id")?,
        project_id: row.try_get("project_id")?,
        environment_id: row.try_get("environment_id")?,
        principal_id: row.try_get("principal_id")?,
        client_protocol: row.try_get("client_protocol")?,
        requested_model: row.try_get("requested_model")?,
        stream: row.try_get("stream")?,
        state: row.try_get("state")?,
        status_code: row.try_get("status_code")?,
        terminal_reason: row.try_get("terminal_reason")?,
        error_message: row.try_get("error_message")?,
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        cache_write_tokens: row.try_get("cache_write_tokens")?,
        cache_read_tokens: row.try_get("cache_read_tokens")?,
        cost_amount_microunits: row.try_get("cost_amount_microunits")?,
        currency: row.try_get("currency")?,
        billing_mode: row.try_get("billing_mode")?,
        chargeable: row.try_get("chargeable")?,
        has_idempotency_key: row.try_get("has_idempotency_key")?,
        lease_owner: row.try_get("lease_owner")?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
        completed_at_ms: row.try_get("completed_at_ms")?,
        attempt_count: row.try_get("attempt_count")?,
    })
}

fn attempt_row_from_pg(row: &PgRow) -> Result<EnterpriseAttemptRow, sqlx::Error> {
    Ok(EnterpriseAttemptRow {
        attempt_id: row.try_get("attempt_id")?,
        request_ledger_id: row.try_get("request_ledger_id")?,
        organization_id: row.try_get("organization_id")?,
        project_id: row.try_get("project_id")?,
        environment_id: row.try_get("environment_id")?,
        provider_id: row.try_get("provider_id")?,
        resolved_model: row.try_get("resolved_model")?,
        provider_protocol: row.try_get("provider_protocol")?,
        state: row.try_get("state")?,
        status_code: row.try_get("status_code")?,
        terminal_reason: row.try_get("terminal_reason")?,
        error_message: row.try_get("error_message")?,
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        cache_write_tokens: row.try_get("cache_write_tokens")?,
        cache_read_tokens: row.try_get("cache_read_tokens")?,
        cost_amount_microunits: row.try_get("cost_amount_microunits")?,
        currency: row.try_get("currency")?,
        billing_mode: row.try_get("billing_mode")?,
        chargeable: row.try_get("chargeable")?,
        lease_owner: row.try_get("lease_owner")?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
        completed_at_ms: row.try_get("completed_at_ms")?,
    })
}

impl Drop for LedgerLease {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl LedgerOutcome {
    pub(crate) fn provider_attempt(
        success: bool,
        status_code: u16,
        error_message: Option<String>,
        estimate: UsageEstimate,
    ) -> Self {
        Self {
            state: if success { "completed" } else { "failed" },
            status_code,
            terminal_reason: if success {
                "completed"
            } else {
                "failed_before_response"
            }
            .to_owned(),
            error_message,
            estimate,
            billing_mode: "local-estimate".to_owned(),
            chargeable: true,
        }
    }

    pub(crate) fn from_usage(usage: &UsageEventInput) -> Self {
        let state = if usage.success {
            "completed"
        } else if usage.terminal_reason.contains("cancel") {
            "cancelled"
        } else {
            "failed"
        };
        Self {
            state,
            status_code: usage.status_code,
            terminal_reason: usage.terminal_reason.clone(),
            error_message: usage.error_message.clone(),
            estimate: usage.estimate,
            billing_mode: usage.billing_mode.clone(),
            chargeable: usage.chargeable,
        }
    }
}

impl From<&TenantScope> for TenantKey {
    fn from(tenant: &TenantScope) -> Self {
        Self {
            organization_id: tenant.organization_id.to_string(),
            project_id: tenant.project_id.to_string(),
            environment_id: tenant.environment_id.to_string(),
        }
    }
}

fn hash_idempotency_key(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn idempotency_conflict(same_request: bool, terminal: bool) -> AppError {
    let message = match (same_request, terminal) {
        (true, false) => "the original request is still in progress",
        (true, true) => {
            "the original request is terminal; response replay is not available in this release"
        }
        (false, _) => "the key was already used with a different request body",
    };
    AppError::IdempotencyConflict(message.to_owned())
}

fn missing_scoped_record() -> AppError {
    AppError::Database(
        "ledger record does not exist in the supplied tenant and lease scope".to_owned(),
    )
}

fn lease_config() -> Result<(Duration, Duration), AppError> {
    let lease_ttl = env_seconds(
        "MODELPORT_LEDGER_LEASE_TTL_SECS",
        DEFAULT_LEASE_TTL_SECS,
        MIN_LEASE_TTL_SECS,
    )?;
    let reconcile_interval = env_seconds(
        "MODELPORT_LEDGER_RECONCILE_INTERVAL_SECS",
        DEFAULT_RECONCILE_INTERVAL_SECS,
        MIN_RECONCILE_INTERVAL_SECS,
    )?;
    validate_lease_durations(lease_ttl, reconcile_interval)?;
    Ok((lease_ttl, reconcile_interval))
}

fn validate_lease_durations(
    lease_ttl: Duration,
    reconcile_interval: Duration,
) -> Result<(), AppError> {
    if reconcile_interval >= lease_ttl {
        return Err(AppError::Config(
            "MODELPORT_LEDGER_RECONCILE_INTERVAL_SECS must be smaller than MODELPORT_LEDGER_LEASE_TTL_SECS"
                .to_owned(),
        ));
    }
    Ok(())
}

fn env_seconds(name: &str, default: u64, minimum: u64) -> Result<Duration, AppError> {
    let seconds = match env::var(name) {
        Ok(value) => value.trim().parse::<u64>().map_err(|_| {
            AppError::Config(format!("{name} must be an integer number of seconds"))
        })?,
        Err(_) => default,
    };
    if seconds < minimum || seconds > i32::MAX as u64 {
        return Err(AppError::Config(format!(
            "{name} must be between {minimum} and {} seconds",
            i32::MAX
        )));
    }
    Ok(Duration::from_secs(seconds))
}

fn duration_secs_i32(duration: Duration) -> i32 {
    i32::try_from(duration.as_secs()).unwrap_or(i32::MAX)
}

fn duration_millis_i64(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn cost_microunits(value: f64) -> i64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1_000_000.0).round().min(i64::MAX as f64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ClientProtocol, RequestId};

    const TEST_FINGERPRINT: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn context() -> RequestContext {
        RequestContext::legacy(
            RequestId::from_string("req_ledger_test"),
            "usr_test",
            ClientProtocol::OpenAiChatCompletions,
        )
    }

    fn estimate(cost_estimate: f64) -> UsageEstimate {
        UsageEstimate {
            input_tokens: 100,
            output_tokens: 20,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            cost_estimate,
        }
    }

    async fn set_local_budget(ledger: &EnterpriseLedger, limit_microunits: i64) {
        ledger
            .update_budget(&EnterpriseBudgetUpdate {
                organization_id: "org_local".to_owned(),
                project_id: "prj_default".to_owned(),
                environment_id: "env_default".to_owned(),
                limit_microunits: Some(limit_microunits),
                unlimited: false,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn memory_ledger_tracks_request_and_attempt_lifecycle() {
        let ledger = EnterpriseLedger::memory();
        let context = context();
        let request = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        assert_eq!(ledger.incomplete_requests(&context.tenant).await, 1);

        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_test"),
                "openai",
                "gpt-test",
                "openai-compatible",
                UsageEstimate::default(),
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome::provider_attempt(true, 200, None, UsageEstimate::default());
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_request(&request, &outcome).await.unwrap();

        assert_eq!(ledger.incomplete_requests(&context.tenant).await, 0);
    }

    #[tokio::test]
    async fn memory_ledger_admin_views_expose_lifecycle_without_sensitive_hashes() {
        let ledger = EnterpriseLedger::memory();
        let context = context();
        let request = ledger
            .begin_request(
                &context,
                "gpt-test",
                true,
                Some("admin-view-key"),
                TEST_FINGERPRINT,
            )
            .await
            .unwrap();
        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_admin_view"),
                "openai",
                "gpt-test",
                "openai-compatible",
                UsageEstimate::default(),
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome::provider_attempt(true, 200, None, UsageEstimate::default());
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_request(&request, &outcome).await.unwrap();

        let overview = ledger.overview().await.unwrap();
        assert_eq!(overview.backend, "memory");
        assert_eq!(overview.total_requests, 1);
        assert_eq!(overview.completed_requests, 1);
        assert_eq!(overview.idempotent_requests, 1);

        let page = ledger
            .list_requests(&EnterpriseLedgerQuery {
                protocol: Some("openai-chat-completions".to_owned()),
                ..EnterpriseLedgerQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.requests[0].attempt_count, 1);
        assert!(page.requests[0].has_idempotency_key);

        let detail = ledger
            .request_detail(&request.ledger_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(detail.attempts[0].provider_id, "openai");
        let serialized = serde_json::to_string(&detail).unwrap();
        assert!(!serialized.contains("admin-view-key"));
        assert!(!serialized.contains(TEST_FINGERPRINT));
        assert!(!serialized.contains("idempotencyKeyHash"));
        assert!(serialized.contains("hasIdempotencyKey"));
    }

    #[test]
    fn enterprise_ledger_query_rejects_unbounded_or_unknown_filters() {
        assert!(
            EnterpriseLedgerQuery {
                page_size: Some(101),
                ..EnterpriseLedgerQuery::default()
            }
            .normalized()
            .is_err()
        );
        assert!(
            EnterpriseLedgerQuery {
                state: Some("unknown".to_owned()),
                ..EnterpriseLedgerQuery::default()
            }
            .normalized()
            .is_err()
        );
    }

    #[tokio::test]
    async fn memory_ledger_rejects_cross_tenant_parent_scope() {
        let ledger = EnterpriseLedger::memory();
        let context = context();
        let mut request = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        request.tenant.organization_id = "org_other".to_owned();

        let result = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_cross_tenant"),
                "openai",
                "gpt-test",
                "openai-compatible",
                UsageEstimate::default(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn memory_ledger_rejects_reused_idempotency_keys() {
        let ledger = EnterpriseLedger::memory();
        let context = context();
        let request = ledger
            .begin_request(
                &context,
                "gpt-test",
                false,
                Some("retry-key-1"),
                TEST_FINGERPRINT,
            )
            .await
            .unwrap();

        let in_progress = ledger
            .begin_request(
                &context,
                "gpt-test",
                false,
                Some("retry-key-1"),
                TEST_FINGERPRINT,
            )
            .await;
        assert!(matches!(
            in_progress,
            Err(AppError::IdempotencyConflict(message)) if message.contains("in progress")
        ));

        let outcome = LedgerOutcome::provider_attempt(true, 200, None, UsageEstimate::default());
        ledger.finalize_request(&request, &outcome).await.unwrap();
        let terminal = ledger
            .begin_request(
                &context,
                "gpt-test",
                false,
                Some("retry-key-1"),
                TEST_FINGERPRINT,
            )
            .await;
        assert!(matches!(
            terminal,
            Err(AppError::IdempotencyConflict(message)) if message.contains("replay")
        ));

        let different = ledger
            .begin_request(
                &context,
                "gpt-test",
                false,
                Some("retry-key-1"),
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .await;
        assert!(matches!(
            different,
            Err(AppError::IdempotencyConflict(message)) if message.contains("different")
        ));
    }

    #[tokio::test]
    async fn memory_ledger_reconciles_only_expired_records() {
        let mut ledger = EnterpriseLedger::memory();
        ledger.lease_ttl = Duration::from_millis(1);
        let context = context();
        let request = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_expired"),
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.75),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(5)).await;
        let result = ledger.reconcile_expired().await.unwrap();
        assert_eq!(
            result,
            ReconcileResult {
                requests: 1,
                attempts: 1
            }
        );
        assert_eq!(ledger.incomplete_requests(&context.tenant).await, 0);
        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.reserved_microunits, 0);
        assert_eq!(budget.account.settled_microunits, 0);
        assert_eq!(budget.recent_events[0].event_type, "released");
        assert_eq!(budget.recent_events[0].reserved_delta_microunits, -750_000);
    }

    #[tokio::test]
    async fn memory_budget_allows_only_one_competing_reservation_within_hard_limit() {
        let ledger = EnterpriseLedger::memory();
        set_local_budget(&ledger, 1_000_000).await;
        let context = context();
        let request = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();

        let first_attempt_id = AttemptId::from_string("att_budget_race_one");
        let second_attempt_id = AttemptId::from_string("att_budget_race_two");
        let first = ledger.begin_attempt(
            &request,
            &first_attempt_id,
            "openai",
            "gpt-test",
            "openai-compatible",
            estimate(0.75),
        );
        let second = ledger.begin_attempt(
            &request,
            &second_attempt_id,
            "openai",
            "gpt-test",
            "openai-compatible",
            estimate(0.75),
        );
        let (first, second) = tokio::join!(first, second);

        assert_ne!(first.is_ok(), second.is_ok());
        let rejected = if first.is_err() { first } else { second };
        assert!(matches!(rejected, Err(AppError::QuotaExceeded(_))));
        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.reserved_microunits, 750_000);
        assert_eq!(budget.account.settled_microunits, 0);
        assert_eq!(budget.recent_events.len(), 1);
    }

    #[tokio::test]
    async fn memory_budget_settlement_is_exact_and_idempotent() {
        let ledger = EnterpriseLedger::memory();
        set_local_budget(&ledger, 2_000_000).await;
        let context = context();
        let request = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_budget_settle"),
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.75),
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome::provider_attempt(true, 200, None, estimate(0.625_123));

        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();

        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.reserved_microunits, 0);
        assert_eq!(budget.account.settled_microunits, 625_123);
        assert_eq!(budget.account.available_microunits, Some(1_374_877));
        assert_eq!(budget.recent_events.len(), 2);
        assert_eq!(budget.recent_events[0].event_type, "settled");
        assert_eq!(budget.recent_events[0].reserved_delta_microunits, -750_000);
        assert_eq!(budget.recent_events[0].settled_delta_microunits, 625_123);
    }

    #[tokio::test]
    async fn memory_budget_adjustments_require_evidence_and_never_rewrite_history() {
        let ledger = EnterpriseLedger::memory();
        let input = EnterpriseBudgetAdjustmentInput {
            organization_id: "org_local".to_owned(),
            project_id: "prj_default".to_owned(),
            environment_id: "env_default".to_owned(),
            delta_microunits: 500_000,
            reason: "provider invoice reconciliation".to_owned(),
            evidence_reference: "invoice://2026-07/acme-42".to_owned(),
        };
        ledger.adjust_budget(&input, "usr_admin").await.unwrap();
        let invalid_reversal = EnterpriseBudgetAdjustmentInput {
            delta_microunits: -500_001,
            reason: "invalid excessive reversal".to_owned(),
            evidence_reference: "ticket://invalid".to_owned(),
            ..input.clone()
        };
        assert!(
            ledger
                .adjust_budget(&invalid_reversal, "usr_admin")
                .await
                .is_err()
        );

        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.settled_microunits, 500_000);
        assert_eq!(budget.recent_events.len(), 1);
        assert_eq!(budget.recent_events[0].event_type, "adjustment");
        assert_eq!(
            budget.recent_events[0].actor_id.as_deref(),
            Some("usr_admin")
        );
        assert_eq!(
            budget.recent_events[0].evidence_source,
            "invoice://2026-07/acme-42"
        );
    }

    #[test]
    fn cost_conversion_is_exact_at_micro_unit_boundary() {
        assert_eq!(cost_microunits(0.000_001), 1);
        assert_eq!(cost_microunits(1.25), 1_250_000);
        assert_eq!(cost_microunits(f64::NAN), 0);
    }

    #[test]
    fn lease_reconciliation_interval_must_be_shorter_than_ttl() {
        assert!(validate_lease_durations(Duration::from_secs(30), Duration::from_secs(29)).is_ok());
        assert!(
            validate_lease_durations(Duration::from_secs(30), Duration::from_secs(30)).is_err()
        );
    }
}
