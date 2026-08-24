use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, QueryBuilder, Row, postgres::PgRow};
use tokio::sync::oneshot;
use tracing::{error, info, warn};
use uuid::Uuid;

use modelport_ops_protocol::{
    OpsAgentSummary, OpsHeartbeat, OpsIncidentDetail, OpsIncidentEvidence,
    OpsIncidentFeedbackInput, OpsIncidentList, OpsIncidentStatus, OpsIncidentStatusUpdate,
    OpsIncidentSummary, OpsIncidentTimelineEntry, OpsLedgerHealth, OpsObservation,
    OpsRequestWindow, OpsSeverity,
};

use crate::{
    AppError,
    control::{
        ProviderUsageStats, UsageEstimate, UsageEventInput, UsagePolicySnapshot, UsageQuotaLimit,
        UsageSummary, quota_subject_for_seed,
    },
    database::{
        connect_pool, database_url as control_database_url, enterprise_database_url,
        redact_database_url,
    },
    domain::{AttemptId, RequestContext, TenantScope},
    metrics::Metrics,
    policy::enforce_spend_limit,
    pricing::{self, ModelPricing},
    smart_router::RoutingDecisionEvidence,
    usage::{current_period, quota_increment},
};

pub(crate) mod compute_inventory;

const DEFAULT_LEASE_TTL_SECS: u64 = 300;
const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 60;
const MIN_LEASE_TTL_SECS: u64 = 30;
const MIN_RECONCILE_INTERVAL_SECS: u64 = 5;
const RETAINED_REQUEST_FINGERPRINT_PREFIX: &str = "modelport-retained-request-fingerprint-v1:";

#[derive(Clone)]
pub(crate) struct EnterpriseLedger {
    backend: Arc<LedgerBackend>,
    location: Arc<str>,
    instance_id: Arc<str>,
    lease_ttl: Duration,
    reconcile_interval: Duration,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DatabasePoolSnapshot {
    pub(crate) size: u32,
    pub(crate) idle: u32,
    pub(crate) max: u32,
}

enum LedgerBackend {
    #[allow(dead_code)]
    Memory(Box<Mutex<MemoryLedger>>),
    Postgres(PgPool),
}

#[derive(Debug, Default)]
struct MemoryLedger {
    requests: HashMap<String, MemoryRequestRecord>,
    attempts: HashMap<String, MemoryRecord>,
    budget_accounts: HashMap<TenantKey, MemoryBudgetAccount>,
    budget_reservations: HashMap<String, MemoryBudgetReservation>,
    usage_reservations: HashMap<String, MemoryUsageReservation>,
    budget_events: Vec<EnterpriseBudgetEvent>,
    audit_events: Vec<EnterpriseAuditEvent>,
    #[allow(dead_code)] // Used by the staged collection integration after this storage seam.
    runtime_compute_snapshots: HashMap<(String, String), compute_inventory::MemoryComputeSnapshot>,
    ops_incidents: BTreeMap<String, OpsIncidentDetail>,
    ops_event_index: HashMap<String, String>,
    ops_heartbeats: BTreeMap<String, OpsHeartbeat>,
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

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct MemoryUsageReservation {
    reservation_id: String,
    quota_subject_id: Option<String>,
    team_id: Option<String>,
    user_id: String,
    reserved_requests: u64,
    reserved_tokens: u64,
    reserved_cost_microunits: i64,
    actual_requests: u64,
    actual_tokens: u64,
    actual_cost_microunits: i64,
    state: String,
    evidence_source: Option<String>,
    billing_mode: Option<String>,
    created_at_ms: i64,
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
    actual_cost_microunits: Option<i64>,
    billable_cost_microunits: Option<i64>,
    pricing_evidence: Option<serde_json::Value>,
    billing_mode: Option<String>,
    chargeable: bool,
    latency_ms: i64,
    first_byte_latency_ms: Option<i64>,
    tool_outcome: String,
    tool_repair_attempted: bool,
    tool_repair_recovered: bool,
    retry_count: i32,
    fallback_from_provider: Option<String>,
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
    username: String,
    api_key_id: Option<String>,
    quota_subject_id: Option<String>,
    api_key_name: Option<String>,
    api_key_group: Option<String>,
    team_id: Option<String>,
    team_name: Option<String>,
    client_ip: Option<String>,
    client_protocol: String,
    requested_model: String,
    request_path: String,
    traffic_class: String,
    tool_use_requested: bool,
    provider_id: Option<String>,
    resolved_model: Option<String>,
    provider_protocol: Option<String>,
    last_attempt_id: Option<String>,
    model_pricing: Option<serde_json::Value>,
    stream: bool,
    routing_decision: Option<RoutingDecisionEvidence>,
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
pub(crate) struct LedgerRequestMetadata {
    pub(crate) request_path: String,
    pub(crate) traffic_class: String,
    pub(crate) tool_use_requested: bool,
    pub(crate) username: String,
    pub(crate) api_key_id: Option<String>,
    pub(crate) quota_subject_id: Option<String>,
    pub(crate) api_key_name: Option<String>,
    pub(crate) api_key_group: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) team_name: Option<String>,
    pub(crate) client_ip: Option<String>,
    pub(crate) routing_decision: Option<RoutingDecisionEvidence>,
}

#[derive(Debug, Clone)]
pub(crate) struct AuditEventInput {
    pub(crate) activity_type: String,
    pub(crate) actor_id: String,
    pub(crate) actor_name: String,
    pub(crate) target: String,
    pub(crate) message: String,
    pub(crate) severity: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ApiKeyUsageStats {
    pub(crate) requests_today: u64,
    pub(crate) tokens_today: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TeamUsageStats {
    pub(crate) requests_today: u64,
    pub(crate) daily_spend_usd: f64,
    pub(crate) monthly_spend_usd: f64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ManagementUsageStats {
    pub(crate) api_keys: HashMap<String, ApiKeyUsageStats>,
    pub(crate) teams: HashMap<String, TeamUsageStats>,
    pub(crate) users_24h: HashMap<String, u64>,
}

impl Default for LedgerRequestMetadata {
    fn default() -> Self {
        Self {
            request_path: "/v1/messages".to_owned(),
            traffic_class: "business".to_owned(),
            tool_use_requested: false,
            username: "local-admin".to_owned(),
            api_key_id: None,
            quota_subject_id: None,
            api_key_name: None,
            api_key_group: None,
            team_id: None,
            team_name: None,
            client_ip: None,
            routing_decision: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LedgerAttempt {
    attempt_id: String,
    request_ledger_id: String,
    reservation_id: String,
    tenant: TenantKey,
    lease_owner: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AttemptPricingEvidence<'a> {
    pub(crate) estimate: UsageEstimate,
    pub(crate) verified: bool,
    pub(crate) usage_policy: &'a UsagePolicySnapshot,
}

pub(crate) struct LedgerLease {
    stop: Option<oneshot::Sender<()>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReconcileResult {
    pub(crate) requests: u64,
    pub(crate) attempts: u64,
}

const DEFAULT_REQUEST_DETAIL_RETENTION_DAYS: u64 = 30;
const DEFAULT_USER_USAGE_RETENTION_DAYS: u64 = 90;
const DEFAULT_AUDIT_RETENTION_DAYS: u64 = 395;
const MILLIS_PER_DAY: u64 = 24 * 60 * 60 * 1_000;
const RETAINED_REQUEST_ID_PREFIX: &str = "retained:";
const RETAINED_PRINCIPAL_ID: &str = "retained-aggregate";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionPolicy {
    pub(crate) request_detail_days: u64,
    pub(crate) user_usage_days: u64,
    pub(crate) audit_days: u64,
    pub(crate) legal_hold: bool,
    /// ModelPort never persists prompts, responses, or tool arguments in its
    /// operational ledger. This explicit flag makes that invariant visible to
    /// retention previews and diagnostics.
    pub(crate) content_persistence: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionCounts {
    pub(crate) request_details_redacted: u64,
    pub(crate) provider_attempts_redacted: u64,
    pub(crate) routing_decisions_deleted: u64,
    pub(crate) user_usage_rows_deidentified: u64,
    pub(crate) audit_events_deleted: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionResult {
    pub(crate) dry_run: bool,
    pub(crate) applied: bool,
    pub(crate) skipped_reason: Option<&'static str>,
    pub(crate) evaluated_at_ms: u64,
    pub(crate) request_detail_cutoff_ms: u64,
    pub(crate) user_usage_cutoff_ms: u64,
    pub(crate) audit_cutoff_ms: u64,
    pub(crate) policy: RetentionPolicy,
    pub(crate) counts: RetentionCounts,
    pub(crate) immutable_budget_events_retained: bool,
}

impl RetentionPolicy {
    pub(crate) fn from_env() -> Result<Self, AppError> {
        let request_detail_days = retention_days_from_env(
            "MODELPORT_REQUEST_DETAIL_RETENTION_DAYS",
            DEFAULT_REQUEST_DETAIL_RETENTION_DAYS,
        )?;
        let user_usage_days = retention_days_from_env(
            "MODELPORT_USER_USAGE_RETENTION_DAYS",
            DEFAULT_USER_USAGE_RETENTION_DAYS,
        )?;
        let audit_days = retention_days_from_env(
            "MODELPORT_AUDIT_RETENTION_DAYS",
            DEFAULT_AUDIT_RETENTION_DAYS,
        )?;
        if request_detail_days > user_usage_days || user_usage_days > audit_days {
            return Err(AppError::Config(
                "retention days must satisfy request detail <= user usage <= audit".to_owned(),
            ));
        }
        Ok(Self {
            request_detail_days,
            user_usage_days,
            audit_days,
            legal_hold: retention_flag_from_env("MODELPORT_RETENTION_LEGAL_HOLD")?,
            content_persistence: false,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseLedgerQuery {
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
    pub(crate) state: Option<String>,
    pub(crate) protocol: Option<String>,
    pub(crate) traffic_class: Option<String>,
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
    estimate_only_requests: i64,
    total_cost_microunits: i64,
    total_billable_cost_microunits: i64,
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
    username: String,
    api_key_id: Option<String>,
    api_key_name: Option<String>,
    api_key_group: Option<String>,
    team_id: Option<String>,
    team_name: Option<String>,
    client_ip: Option<String>,
    client_protocol: String,
    requested_model: String,
    request_path: String,
    traffic_class: String,
    tool_use_requested: bool,
    provider_id: Option<String>,
    resolved_model: Option<String>,
    provider_protocol: Option<String>,
    last_attempt_id: Option<String>,
    model_pricing: Option<serde_json::Value>,
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
    actual_cost_microunits: Option<i64>,
    billable_cost_microunits: Option<i64>,
    pricing_evidence: Option<serde_json::Value>,
    currency: String,
    billing_mode: Option<String>,
    chargeable: bool,
    latency_ms: i64,
    first_byte_latency_ms: Option<i64>,
    tool_outcome: String,
    tool_repair_attempted: bool,
    tool_repair_recovered: bool,
    retry_count: i32,
    fallback_from_provider: Option<String>,
    routing_decision: Option<RoutingDecisionEvidence>,
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
    actual_cost_microunits: Option<i64>,
    billable_cost_microunits: Option<i64>,
    pricing_evidence: Option<serde_json::Value>,
    currency: String,
    billing_mode: Option<String>,
    chargeable: bool,
    latency_ms: i64,
    first_byte_latency_ms: Option<i64>,
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

#[derive(Debug, Clone, Default)]
pub(crate) struct OperationalLogQuery {
    pub(crate) page: usize,
    pub(crate) page_size: usize,
    pub(crate) status: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) api_key_id: Option<String>,
    pub(crate) date_from: Option<u64>,
    pub(crate) date_to: Option<u64>,
    pub(crate) search: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) stream: Option<bool>,
    pub(crate) tool_use_requested: Option<bool>,
    pub(crate) traffic_class: Option<String>,
}

#[derive(Debug)]
pub(crate) struct OperationalLogPage {
    pub(crate) logs: Vec<Value>,
    pub(crate) total: i64,
    pub(crate) summary: Value,
}

#[derive(Debug)]
pub(crate) struct DashboardLedgerSnapshot {
    pub(crate) usage_summary: UsageSummary,
    pub(crate) provider_usage: BTreeMap<String, ProviderUsageStats>,
    pub(crate) matched_requests: u64,
    pub(crate) request_time_series: Vec<Value>,
    pub(crate) error_time_series: Vec<Value>,
    pub(crate) token_time_series: Vec<Value>,
    pub(crate) model_usage: Vec<Value>,
    pub(crate) summary: Value,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    warning_threshold_reached: bool,
    hard_limit_reached: bool,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnterpriseAuditEvent {
    id: String,
    timestamp: String,
    #[serde(rename = "type")]
    activity_type: String,
    actor_id: String,
    actor: String,
    target: String,
    message: String,
    severity: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct UsageSpendTotals {
    api_key_all_time: f64,
    api_key_five_hours: f64,
    api_key_day: f64,
    api_key_week: f64,
    api_key_month: f64,
    team_day: f64,
    team_month: f64,
}

#[derive(Debug, Clone, Copy)]
struct UsageReservationIncrement {
    requests: u64,
    tokens: u64,
    cost_microunits: i64,
}

impl UsageReservationIncrement {
    fn for_attempt(estimate: UsageEstimate, first_attempt: bool) -> Self {
        Self {
            requests: u64::from(first_attempt),
            tokens: estimate_total_tokens(estimate),
            cost_microunits: cost_microunits(estimate.cost_estimate),
        }
    }

    fn quota_value(self, quota_type: &str) -> f64 {
        quota_value_from_totals(quota_type, self.requests, self.tokens, self.cost_microunits)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LedgerOutcome {
    state: &'static str,
    status_code: u16,
    terminal_reason: String,
    error_message: Option<String>,
    estimate: UsageEstimate,
    pricing_evidence: Option<serde_json::Value>,
    billing_mode: String,
    chargeable: bool,
    latency_ms: i64,
    first_byte_latency_ms: Option<i64>,
    tool_outcome: String,
    tool_repair_attempted: bool,
    tool_repair_recovered: bool,
    retry_count: i32,
    fallback_from_provider: Option<String>,
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
            actual_cost_microunits: None,
            billable_cost_microunits: None,
            pricing_evidence: None,
            billing_mode: None,
            chargeable: false,
            latency_ms: 0,
            first_byte_latency_ms: None,
            tool_outcome: "not_requested".to_owned(),
            tool_repair_attempted: false,
            tool_repair_recovered: false,
            retry_count: 0,
            fallback_from_provider: None,
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
        self.actual_cost_microunits = outcome.estimate.actual_cost.map(cost_microunits);
        self.billable_cost_microunits = outcome.estimate.billable_cost.map(cost_microunits);
        self.pricing_evidence.clone_from(&outcome.pricing_evidence);
        self.billing_mode = Some(outcome.billing_mode.clone());
        self.chargeable = outcome.chargeable;
        self.latency_ms = outcome.latency_ms;
        self.first_byte_latency_ms = outcome.first_byte_latency_ms;
        self.tool_outcome.clone_from(&outcome.tool_outcome);
        self.tool_repair_attempted = outcome.tool_repair_attempted;
        self.tool_repair_recovered = outcome.tool_repair_recovered;
        self.retry_count = outcome.retry_count;
        self.fallback_from_provider
            .clone_from(&outcome.fallback_from_provider);
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
        self.latency_ms = now.saturating_sub(self.created_at_ms);
        self.updated_at_ms = now;
        self.completed_at_ms = Some(now);
    }
}

impl EnterpriseLedger {
    pub(crate) fn database_pool_snapshot(&self) -> Option<DatabasePoolSnapshot> {
        let LedgerBackend::Postgres(pool) = self.backend.as_ref() else {
            return None;
        };
        Some(DatabasePoolSnapshot {
            size: pool.size(),
            idle: u32::try_from(pool.num_idle()).unwrap_or(u32::MAX),
            max: pool.options().get_max_connections(),
        })
    }

    /// Stable onboarding evidence derived from the retained request ledger,
    /// independent of today's dashboard window or the five most recent rows.
    pub(crate) async fn onboarding_milestones(&self) -> Result<(bool, bool), AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let has_request = !ledger.requests.is_empty();
                let has_successful_request = ledger.requests.values().any(|request| {
                    request
                        .record
                        .status_code
                        .is_some_and(|status| (200..300).contains(&status))
                });
                Ok((has_request, has_successful_request))
            }
            LedgerBackend::Postgres(pool) => {
                let row = sqlx::query(
                    r#"
                    SELECT
                        EXISTS (SELECT 1 FROM modelport_gateway_requests) AS has_request,
                        EXISTS (
                            SELECT 1
                            FROM modelport_gateway_requests
                            WHERE status_code >= 200 AND status_code < 300
                        ) AS has_successful_request
                    "#,
                )
                .fetch_one(pool)
                .await?;
                Ok((
                    row.try_get("has_request")?,
                    row.try_get("has_successful_request")?,
                ))
            }
        }
    }

    pub(crate) fn validate_configuration() -> Result<(), AppError> {
        lease_config().map(|_| ())
    }

    #[cfg(test)]
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

    #[cfg(test)]
    async fn postgres_for_tests(database_url: &str) -> Result<Self, AppError> {
        let pool = connect_pool(database_url, Some(4)).await?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|error| AppError::Database(format!("database migration failed: {error}")))?;
        Ok(Self {
            backend: Arc::new(LedgerBackend::Postgres(pool)),
            location: Arc::from("postgres://test#relational-ledger"),
            instance_id: Arc::from(format!("ins_{}", Uuid::new_v4().simple())),
            lease_ttl: Duration::from_secs(DEFAULT_LEASE_TTL_SECS),
            reconcile_interval: Duration::from_secs(DEFAULT_RECONCILE_INTERVAL_SECS),
        })
    }

    pub(crate) async fn connect_from_env() -> Result<Self, AppError> {
        let (lease_ttl, reconcile_interval) = lease_config()?;
        if control_database_url().is_none() {
            return Err(AppError::Config(
                "MODELPORT_DATABASE_URL is required; current ModelPort releases use PostgreSQL as the only runtime request ledger"
                    .to_owned(),
            ));
        }
        let Some(database_url) = enterprise_database_url() else {
            return Err(AppError::Config(
                "MODELPORT_ENTERPRISE_DATABASE_URL or MODELPORT_DATABASE_URL is required"
                    .to_owned(),
            ));
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

    #[cfg(test)]
    pub(crate) async fn begin_request(
        &self,
        context: &RequestContext,
        requested_model: &str,
        stream: bool,
        idempotency_key: Option<&str>,
        request_fingerprint: &str,
    ) -> Result<LedgerRequest, AppError> {
        self.begin_request_with_metadata(
            context,
            requested_model,
            stream,
            idempotency_key,
            request_fingerprint,
            &LedgerRequestMetadata::default(),
        )
        .await
    }

    pub(crate) async fn begin_request_with_metadata(
        &self,
        context: &RequestContext,
        requested_model: &str,
        stream: bool,
        idempotency_key: Option<&str>,
        request_fingerprint: &str,
        metadata: &LedgerRequestMetadata,
    ) -> Result<LedgerRequest, AppError> {
        if !matches!(
            metadata.request_path.as_str(),
            "/v1/messages" | "/v1/chat/completions"
        ) {
            return Err(AppError::InvalidRequest(
                "unsupported request path for enterprise ledger".to_owned(),
            ));
        }
        if !matches!(
            metadata.traffic_class.as_str(),
            "business" | "synthetic" | "diagnostic"
        ) {
            return Err(AppError::InvalidRequest(
                "unsupported traffic class for enterprise ledger".to_owned(),
            ));
        }
        validate_request_metadata(metadata)?;
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
                        username: metadata.username.clone(),
                        api_key_id: metadata.api_key_id.clone(),
                        quota_subject_id: metadata.quota_subject_id.clone(),
                        api_key_name: metadata.api_key_name.clone(),
                        api_key_group: metadata.api_key_group.clone(),
                        team_id: metadata.team_id.clone(),
                        team_name: metadata.team_name.clone(),
                        client_ip: metadata.client_ip.clone(),
                        client_protocol: context.protocol.as_str().to_owned(),
                        requested_model: requested_model.to_owned(),
                        request_path: metadata.request_path.clone(),
                        traffic_class: metadata.traffic_class.clone(),
                        tool_use_requested: metadata.tool_use_requested,
                        provider_id: None,
                        resolved_model: None,
                        provider_protocol: None,
                        last_attempt_id: None,
                        model_pricing: None,
                        stream,
                        routing_decision: metadata.routing_decision.clone(),
                        idempotency_key_hash,
                        request_fingerprint: request_fingerprint.to_owned(),
                    },
                );
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                ensure_tenant_catalog(&mut transaction, &request.tenant).await?;
                let result = sqlx::query(
                    "INSERT INTO modelport_gateway_requests (
                        ledger_id, request_id,
                        organization_id, project_id, environment_id,
                        principal_id, username,
                        api_key_id, quota_subject_id, api_key_name, api_key_group,
                        team_id, team_name, client_ip,
                        client_protocol, requested_model, stream,
                        request_path, traffic_class, tool_use_requested,
                        idempotency_key_hash, request_fingerprint,
                        lease_owner, lease_expires_at
                    ) VALUES (
                        $1, $2, $3, $4, $5, $6, $7, $8, $9,
                        $10, $11, $12, $13, $14::inet, $15, $16, $17,
                        $18, $19, $20, $21, $22, $23,
                        now() + ($24 * interval '1 second')
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
                .bind(&metadata.username)
                .bind(metadata.api_key_id.as_deref())
                .bind(metadata.quota_subject_id.as_deref())
                .bind(metadata.api_key_name.as_deref())
                .bind(metadata.api_key_group.as_deref())
                .bind(metadata.team_id.as_deref())
                .bind(metadata.team_name.as_deref())
                .bind(metadata.client_ip.as_deref())
                .bind(context.protocol.as_str())
                .bind(requested_model)
                .bind(stream)
                .bind(&metadata.request_path)
                .bind(&metadata.traffic_class)
                .bind(metadata.tool_use_requested)
                .bind(idempotency_key_hash.as_deref())
                .bind(request_fingerprint)
                .bind(&request.lease_owner)
                .bind(duration_secs_i32(self.lease_ttl))
                .execute(&mut *transaction)
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
                    .fetch_one(&mut *transaction)
                    .await?;
                    return Err(idempotency_conflict(
                        existing.0 == request_fingerprint,
                        existing.1 != "started",
                    ));
                }
                if let Some(decision) = &metadata.routing_decision {
                    sqlx::query(
                        "INSERT INTO modelport_routing_decisions (
                            decision_id, request_ledger_id,
                            organization_id, project_id, environment_id,
                            route_group_id, routing_profile, routing_mode, policy_version,
                            selected_provider_id, selected_model,
                            recommended_provider_id, recommended_model,
                            candidate_count, selected_score, recommended_score, reason_codes,
                            session_affinity, shadow_disagreement
                        ) VALUES (
                            $1, $2, $3, $4, $5, $6, $7, $8,
                            $9, $10, $11, $12, $13, $14, $15, $16,
                            $17, $18, $19
                        )",
                    )
                    .bind(&decision.decision_id)
                    .bind(&request.ledger_id)
                    .bind(&request.tenant.organization_id)
                    .bind(&request.tenant.project_id)
                    .bind(&request.tenant.environment_id)
                    .bind(decision.group_id.as_deref())
                    .bind(&decision.profile)
                    .bind(&decision.mode)
                    .bind(&decision.policy_version)
                    .bind(&decision.selected_provider)
                    .bind(&decision.selected_model)
                    .bind(&decision.recommended_provider)
                    .bind(&decision.recommended_model)
                    .bind(i32::try_from(decision.candidate_count).unwrap_or(i32::MAX))
                    .bind(decision.selected_score)
                    .bind(decision.recommended_score)
                    .bind(&decision.reason_codes)
                    .bind(decision.session_affinity)
                    .bind(decision.shadow_disagreement)
                    .execute(&mut *transaction)
                    .await?;
                }
                transaction.commit().await?;
            }
        }
        Ok(request)
    }

    pub(crate) async fn begin_attempt_with_pricing(
        &self,
        request: &LedgerRequest,
        attempt_id: &AttemptId,
        provider_id: &str,
        resolved_model: &str,
        provider_protocol: &str,
        pricing: AttemptPricingEvidence<'_>,
    ) -> Result<LedgerAttempt, AppError> {
        let AttemptPricingEvidence {
            estimate,
            verified: pricing_verified,
            usage_policy,
        } = pricing;
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
                validate_usage_reservation_pricing(usage_policy, pricing_verified)?;
                let usage_increment = check_memory_usage_admission(
                    &ledger,
                    &request.ledger_id,
                    usage_policy,
                    estimate,
                )?;
                let hard_budget_enabled = ledger
                    .budget_accounts
                    .get(&attempt.tenant)
                    .is_some_and(|account| account.limit_microunits.is_some());
                if hard_budget_enabled && !pricing_verified {
                    return Err(AppError::PricingUnverified(
                        "enterprise amount budget requires configured model pricing".to_owned(),
                    ));
                }
                {
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
                }
                let now = now_millis();
                reserve_memory_usage(
                    &mut ledger,
                    &request.ledger_id,
                    usage_policy,
                    usage_increment,
                    now,
                );
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
                validate_usage_reservation_pricing(usage_policy, pricing_verified)?;
                reserve_usage_capacity_pg(&mut transaction, request, usage_policy, estimate)
                    .await?;
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
                let hard_budget_enabled = sqlx::query_scalar::<_, Option<i64>>(
                    "SELECT limit_microunits
                     FROM modelport_budget_accounts
                     WHERE organization_id = $1
                       AND project_id = $2
                       AND environment_id = $3
                       AND currency = 'USD'",
                )
                .bind(&attempt.tenant.organization_id)
                .bind(&attempt.tenant.project_id)
                .bind(&attempt.tenant.environment_id)
                .fetch_one(&mut *transaction)
                .await?
                .is_some();
                if hard_budget_enabled && !pricing_verified {
                    return Err(AppError::PricingUnverified(
                        "enterprise amount budget requires configured model pricing".to_owned(),
                    ));
                }
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

    #[cfg(test)]
    pub(crate) async fn begin_attempt(
        &self,
        request: &LedgerRequest,
        attempt_id: &AttemptId,
        provider_id: &str,
        resolved_model: &str,
        provider_protocol: &str,
        estimate: UsageEstimate,
    ) -> Result<LedgerAttempt, AppError> {
        let usage_policy = UsagePolicySnapshot::default();
        self.begin_attempt_with_pricing(
            request,
            attempt_id,
            provider_id,
            resolved_model,
            provider_protocol,
            AttemptPricingEvidence {
                estimate,
                verified: true,
                usage_policy: &usage_policy,
            },
        )
        .await
    }

    pub(crate) async fn finalize_attempt(
        &self,
        attempt: &LedgerAttempt,
        outcome: &LedgerOutcome,
    ) -> Result<(), AppError> {
        validate_billing_outcome(outcome)?;
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
                if outcome.chargeable && outcome.estimate.billable_cost.is_some() {
                    settle_memory_budget(&mut ledger, attempt, outcome)?;
                } else {
                    release_memory_budget(
                        &mut ledger,
                        &attempt.attempt_id,
                        outcome_evidence_source(outcome),
                        &outcome.billing_mode,
                        &outcome.terminal_reason,
                    )?;
                }
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
                if outcome.chargeable && outcome.estimate.billable_cost.is_some() {
                    settle_budget_pg(&mut transaction, attempt, outcome).await?;
                } else {
                    release_budget_pg(
                        &mut transaction,
                        &attempt.attempt_id,
                        &attempt.tenant,
                        outcome_evidence_source(outcome),
                        &outcome.billing_mode,
                        &outcome.terminal_reason,
                    )
                    .await?;
                }
                transaction.commit().await?;
                Ok(())
            }
        }
    }

    #[cfg(test)]
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

    pub(crate) async fn finalize_request_usage(
        &self,
        request: &LedgerRequest,
        usage: &UsageEventInput,
    ) -> Result<(), AppError> {
        let outcome = LedgerOutcome::from_usage(usage);
        validate_billing_outcome(&outcome)?;
        let provider_snapshot = usage.attempt_id.as_ref().map(|attempt_id| {
            (
                usage.provider.as_str(),
                usage.resolved_model.as_str(),
                usage.protocol.as_str(),
                attempt_id.as_str(),
            )
        });
        let model_pricing = usage.model_pricing.map(serde_json::to_value).transpose()?;

        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                {
                    let Some(record) =
                        ledger
                            .requests
                            .get_mut(&request.ledger_id)
                            .filter(|record| {
                                record.record.tenant == request.tenant
                                    && record.record.lease_owner == request.lease_owner
                            })
                    else {
                        return Err(missing_scoped_record());
                    };
                    record.record.finalize(&outcome);
                    if let Some((provider_id, resolved_model, provider_protocol, attempt_id)) =
                        provider_snapshot
                    {
                        record.provider_id = Some(provider_id.to_owned());
                        record.resolved_model = Some(resolved_model.to_owned());
                        record.provider_protocol = Some(provider_protocol.to_owned());
                        record.last_attempt_id = Some(attempt_id.to_owned());
                    }
                    record.model_pricing = model_pricing;
                }
                settle_memory_usage(&mut ledger, &request.ledger_id, &outcome);
                Ok(())
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let updated = update_terminal_record_pg(
                    &mut transaction,
                    true,
                    &request.ledger_id,
                    &request.tenant,
                    &request.lease_owner,
                    &outcome,
                )
                .await?;
                if !updated {
                    return Err(missing_scoped_record());
                }
                let (provider_id, resolved_model, provider_protocol, attempt_id) =
                    provider_snapshot
                        .map(|snapshot| {
                            (
                                Some(snapshot.0),
                                Some(snapshot.1),
                                Some(snapshot.2),
                                Some(snapshot.3),
                            )
                        })
                        .unwrap_or((None, None, None, None));
                sqlx::query(
                    "UPDATE modelport_gateway_requests
                     SET provider_id = $1,
                         resolved_model = $2,
                         provider_protocol = $3,
                         last_attempt_id = $4,
                         model_pricing = $5
                     WHERE ledger_id = $6
                       AND organization_id = $7
                       AND project_id = $8
                       AND environment_id = $9
                       AND lease_owner = $10",
                )
                .bind(provider_id)
                .bind(resolved_model)
                .bind(provider_protocol)
                .bind(attempt_id)
                .bind(model_pricing)
                .bind(&request.ledger_id)
                .bind(&request.tenant.organization_id)
                .bind(&request.tenant.project_id)
                .bind(&request.tenant.environment_id)
                .bind(&request.lease_owner)
                .execute(&mut *transaction)
                .await?;
                settle_usage_reservation_pg(&mut transaction, request, &outcome).await?;
                transaction.commit().await?;
                Ok(())
            }
        }
    }

    #[cfg(test)]
    async fn finalize_request_record(
        &self,
        id: &str,
        tenant: &TenantKey,
        lease_owner: &str,
        outcome: &LedgerOutcome,
    ) -> Result<(), AppError> {
        validate_billing_outcome(outcome)?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                {
                    let Some(record) = ledger.requests.get_mut(id).filter(|record| {
                        record.record.tenant == *tenant && record.record.lease_owner == lease_owner
                    }) else {
                        return Err(missing_scoped_record());
                    };
                    record.record.finalize(outcome);
                }
                settle_memory_usage(&mut ledger, id, outcome);
                Ok(())
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                update_terminal_record_pg(&mut transaction, true, id, tenant, lease_owner, outcome)
                    .await?;
                settle_usage_reservation_by_id_pg(&mut transaction, id, tenant, outcome).await?;
                transaction.commit().await?;
                Ok(())
            }
        }
    }

    pub(crate) fn maintain_lease(
        &self,
        request: &LedgerRequest,
        metrics: Arc<Metrics>,
    ) -> LedgerLease {
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
                        let renewal = ledger.renew_lease(&request).await;
                        metrics.record_ledger_operation("lease_renewal", renewal.is_ok());
                        if let Err(err) = renewal {
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
                    release_memory_budget(
                        &mut ledger,
                        &attempt_id,
                        "lease-expired",
                        "unreconciled",
                        "expired Provider Attempt lease released its budget reservation",
                    )?;
                    result.attempts += 1;
                }
                let expired_request_ids = ledger
                    .requests
                    .iter()
                    .filter(|(_, record)| {
                        !record.record.terminal && record.record.lease_expires_at <= now
                    })
                    .map(|(ledger_id, _)| ledger_id.clone())
                    .collect::<Vec<_>>();
                for ledger_id in expired_request_ids {
                    if let Some(record) = ledger.requests.get_mut(&ledger_id) {
                        record.record.mark_unreconciled(false);
                        if record.tool_use_requested {
                            record.record.tool_outcome = "upstream_or_delivery_error".to_owned();
                        } else {
                            record.record.tool_outcome = "not_requested".to_owned();
                        }
                    }
                    release_memory_usage(&mut ledger, &ledger_id);
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
                         latency_ms = GREATEST(
                             0,
                             (EXTRACT(EPOCH FROM (now() - created_at)) * 1000)::bigint
                         ),
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
                        "lease-expired",
                        "unreconciled",
                        "expired Provider Attempt lease released its budget reservation",
                    )
                    .await?;
                }
                let expired_requests = sqlx::query(
                    "UPDATE modelport_gateway_requests
                     SET state = 'failed',
                         status_code = 500,
                         terminal_reason = 'lease_expired_unreconciled',
                         error_message = 'ledger lease expired before a terminal request outcome was persisted',
                         billing_mode = 'unreconciled',
                         chargeable = false,
                         latency_ms = GREATEST(
                             0,
                             (EXTRACT(EPOCH FROM (now() - created_at)) * 1000)::bigint
                         ),
                         tool_outcome = CASE
                             WHEN tool_use_requested THEN 'upstream_or_delivery_error'
                             ELSE 'not_requested'
                         END,
                         updated_at = now(),
                         completed_at = now()
                     WHERE state = 'started'
                       AND lease_expires_at <= now()
                     RETURNING ledger_id, organization_id, project_id, environment_id",
                )
                .fetch_all(&mut *transaction)
                .await?;
                for row in &expired_requests {
                    release_usage_reservation_pg(
                        &mut transaction,
                        row.try_get("ledger_id")?,
                        &TenantKey {
                            organization_id: row.try_get("organization_id")?,
                            project_id: row.try_get("project_id")?,
                            environment_id: row.try_get("environment_id")?,
                        },
                    )
                    .await?;
                }
                transaction.commit().await?;
                Ok(ReconcileResult {
                    requests: usize_to_u64(expired_requests.len()),
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
            estimate_only_requests: 0,
            total_cost_microunits: 0,
            total_billable_cost_microunits: 0,
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
                        if request.record.terminal
                            && request.record.billable_cost_microunits.is_none()
                        {
                            overview.estimate_only_requests += 1;
                        }
                    }
                    overview.total_cost_microunits = overview
                        .total_cost_microunits
                        .saturating_add(request.record.cost_amount_microunits);
                    overview.total_billable_cost_microunits = overview
                        .total_billable_cost_microunits
                        .saturating_add(request.record.billable_cost_microunits.unwrap_or(0));
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
                        count(*) FILTER (
                            WHERE state <> 'started' AND chargeable
                              AND billable_cost_microunits IS NULL
                        )::bigint AS estimate_only_requests,
                        COALESCE(sum(cost_amount_microunits), 0)::bigint AS total_cost_microunits,
                        COALESCE(sum(billable_cost_microunits), 0)::bigint AS total_billable_cost_microunits,
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
                overview.estimate_only_requests = row.try_get("estimate_only_requests")?;
                overview.total_cost_microunits = row.try_get("total_cost_microunits")?;
                overview.total_billable_cost_microunits =
                    row.try_get("total_billable_cost_microunits")?;
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
                    .bind(query.traffic_class.as_deref())
                    .fetch_one(pool)
                    .await?;
                let rows = sqlx::query(REQUEST_LIST_SQL)
                    .bind(query.state.as_deref())
                    .bind(query.protocol.as_deref())
                    .bind(query.organization_id.as_deref())
                    .bind(query.project_id.as_deref())
                    .bind(query.environment_id.as_deref())
                    .bind(query.search.as_deref())
                    .bind(query.traffic_class.as_deref())
                    .bind(usize_to_i64(query.page_size))
                    .bind(usize_to_i64(query.offset()))
                    .bind(None::<i64>)
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

    #[cfg(test)]
    pub(crate) async fn usage_rows(&self) -> Result<Vec<Value>, AppError> {
        self.usage_rows_since(None).await
    }

    pub(crate) async fn usage_rows_since(
        &self,
        since_ms: Option<u64>,
    ) -> Result<Vec<Value>, AppError> {
        let since_ms_i64 = since_ms.map(|value| i64::try_from(value).unwrap_or(i64::MAX));
        let mut requests = match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                ledger
                    .requests
                    .iter()
                    .filter(|(_, request)| {
                        since_ms_i64.is_none_or(|since| request.record.created_at_ms >= since)
                    })
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
                    .collect::<Vec<_>>()
            }
            LedgerBackend::Postgres(pool) => sqlx::query(REQUEST_LIST_SQL)
                .bind(None::<&str>)
                .bind(None::<&str>)
                .bind(None::<&str>)
                .bind(None::<&str>)
                .bind(None::<&str>)
                .bind(None::<&str>)
                .bind(None::<&str>)
                .bind(i64::MAX)
                .bind(0_i64)
                .bind(since_ms_i64)
                .fetch_all(pool)
                .await?
                .iter()
                .map(request_row_from_pg)
                .collect::<Result<Vec<_>, _>>()?,
        };
        requests.retain(|request| request.state != "started");
        requests.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.ledger_id.cmp(&left.ledger_id))
        });
        Ok(requests.iter().map(operational_log_row).collect())
    }

    pub(crate) async fn operational_logs(
        &self,
        query: &OperationalLogQuery,
    ) -> Result<Option<OperationalLogPage>, AppError> {
        let LedgerBackend::Postgres(pool) = self.backend.as_ref() else {
            return Ok(None);
        };

        let mut summary_query = QueryBuilder::<Postgres>::new(
            "SELECT
                count(*)::bigint AS total_requests,
                count(*) FILTER (WHERE r.state = 'completed')::bigint AS success_requests,
                count(*) FILTER (WHERE r.tool_use_requested)::bigint AS tool_use_requests,
                count(*) FILTER (
                    WHERE r.tool_use_requested AND r.state = 'completed'
                )::bigint AS tool_use_success_requests,
                COALESCE(sum(r.input_tokens), 0)::bigint AS total_input_tokens,
                COALESCE(sum(r.output_tokens), 0)::bigint AS total_output_tokens,
                COALESCE(sum(r.cache_write_tokens), 0)::bigint AS total_cache_write_tokens,
                COALESCE(sum(r.cache_read_tokens), 0)::bigint AS total_cache_read_tokens,
                COALESCE(sum(r.cost_amount_microunits), 0)::bigint AS total_cost_microunits,
                COALESCE(sum(r.actual_cost_microunits), 0)::bigint AS total_actual_cost_microunits,
                COALESCE(sum(r.billable_cost_microunits), 0)::bigint AS total_billable_cost_microunits,
                count(*) FILTER (
                    WHERE r.billable_cost_microunits IS NOT NULL
                )::bigint AS billable_requests,
                count(*) FILTER (
                    WHERE r.billable_cost_microunits IS NULL
                )::bigint AS estimate_only_requests,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY r.latency_ms)
                    FILTER (WHERE r.latency_ms IS NOT NULL) AS latency_p95_ms,
                count(r.latency_ms)::bigint AS latency_sample_count,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY r.first_byte_latency_ms)
                    FILTER (WHERE r.first_byte_latency_ms IS NOT NULL)
                    AS first_byte_latency_p95_ms,
                count(r.first_byte_latency_ms)::bigint AS first_byte_latency_sample_count,
                (EXTRACT(EPOCH FROM min(r.created_at)) * 1000)::bigint AS first_timestamp_ms,
                (EXTRACT(EPOCH FROM max(r.created_at)) * 1000)::bigint AS last_timestamp_ms
             FROM modelport_gateway_requests r",
        );
        push_operational_log_filters(&mut summary_query, query);
        let summary_row = summary_query.build().fetch_one(pool).await?;
        let total: i64 = summary_row.try_get("total_requests")?;
        let total_input_tokens: i64 = summary_row.try_get("total_input_tokens")?;
        let total_output_tokens: i64 = summary_row.try_get("total_output_tokens")?;
        let total_cache_write_tokens: i64 = summary_row.try_get("total_cache_write_tokens")?;
        let total_cache_read_tokens: i64 = summary_row.try_get("total_cache_read_tokens")?;
        let total_tokens = total_input_tokens
            .saturating_add(total_output_tokens)
            .saturating_add(total_cache_write_tokens)
            .saturating_add(total_cache_read_tokens);
        let first_timestamp: Option<i64> = summary_row.try_get("first_timestamp_ms")?;
        let last_timestamp: Option<i64> = summary_row.try_get("last_timestamp_ms")?;
        let minutes = match (first_timestamp, last_timestamp) {
            (Some(first), Some(last)) if last > first => {
                ((last - first) as f64 / 60_000.0).max(1.0)
            }
            _ => 1.0,
        };
        let summary = json!({
            "totalRequests": nonnegative_u64(total),
            "successRequests": nonnegative_u64(summary_row.try_get("success_requests")?),
            "toolUseRequests": nonnegative_u64(summary_row.try_get("tool_use_requests")?),
            "toolUseSuccessRequests": nonnegative_u64(
                summary_row.try_get("tool_use_success_requests")?
            ),
            "totalInputTokens": nonnegative_u64(total_input_tokens),
            "totalOutputTokens": nonnegative_u64(total_output_tokens),
            "totalCacheWriteTokens": nonnegative_u64(total_cache_write_tokens),
            "totalCacheReadTokens": nonnegative_u64(total_cache_read_tokens),
            "totalTokens": nonnegative_u64(total_tokens),
            "totalCostEstimate": microunits_usd(
                summary_row.try_get("total_cost_microunits")?
            ),
            "totalActualCost": microunits_usd(
                summary_row.try_get("total_actual_cost_microunits")?
            ),
            "totalBillableCost": microunits_usd(
                summary_row.try_get("total_billable_cost_microunits")?
            ),
            "billableRequests": nonnegative_u64(
                summary_row.try_get("billable_requests")?
            ),
            "estimateOnlyRequests": nonnegative_u64(
                summary_row.try_get("estimate_only_requests")?
            ),
            "latencyP95Ms": summary_row
                .try_get::<Option<i64>, _>("latency_p95_ms")?
                .map(nonnegative_u64)
                .unwrap_or(0),
            "latencySampleCount": nonnegative_u64(
                summary_row.try_get("latency_sample_count")?
            ),
            "firstByteLatencyP95Ms": summary_row
                .try_get::<Option<i64>, _>("first_byte_latency_p95_ms")?
                .map(nonnegative_u64)
                .unwrap_or(0),
            "firstByteLatencySampleCount": nonnegative_u64(
                summary_row.try_get("first_byte_latency_sample_count")?
            ),
            "rpm": total.max(0) as f64 / minutes,
            "tpm": total_tokens.max(0) as f64 / minutes,
        });

        let mut rows_query = QueryBuilder::<Postgres>::new(OPERATIONAL_LOG_SELECT_SQL);
        push_operational_log_filters(&mut rows_query, query);
        rows_query
            .push(" ORDER BY r.created_at DESC, r.ledger_id DESC LIMIT ")
            .push_bind(usize_to_i64(query.page_size))
            .push(" OFFSET ")
            .push_bind(usize_to_i64(
                query.page.saturating_sub(1).saturating_mul(query.page_size),
            ));
        let rows = rows_query.build().fetch_all(pool).await?;
        let logs = rows
            .iter()
            .map(request_row_from_pg)
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .map(operational_log_row)
            .collect();

        Ok(Some(OperationalLogPage {
            logs,
            total,
            summary,
        }))
    }

    pub(crate) async fn dashboard_snapshot(
        &self,
        start_ms: u64,
        end_ms: u64,
        bucket_ms: u64,
        today_start_ms: u64,
        api_keys: (u64, u64),
    ) -> Result<Option<DashboardLedgerSnapshot>, AppError> {
        let LedgerBackend::Postgres(pool) = self.backend.as_ref() else {
            return Ok(None);
        };
        let start_ms = i64::try_from(start_ms).unwrap_or(i64::MAX);
        let end_ms = i64::try_from(end_ms).unwrap_or(i64::MAX);
        let bucket_ms = i64::try_from(bucket_ms.max(1)).unwrap_or(i64::MAX);
        let today_start_ms = i64::try_from(today_start_ms).unwrap_or(i64::MAX);

        let provider_rows = sqlx::query(
            "SELECT
                COALESCE(provider_id, 'unrouted') AS provider_id,
                count(*)::bigint AS requests,
                count(*) FILTER (WHERE state = 'completed')::bigint AS successes,
                COALESCE(sum(latency_ms), 0)::bigint AS duration_ms,
                COALESCE(sum(input_tokens), 0)::bigint AS input_tokens,
                COALESCE(sum(output_tokens), 0)::bigint AS output_tokens,
                COALESCE(sum(cache_write_tokens), 0)::bigint AS cache_write_tokens,
                COALESCE(sum(cache_read_tokens), 0)::bigint AS cache_read_tokens,
                COALESCE(sum(cost_amount_microunits), 0)::bigint AS cost_microunits
             FROM modelport_gateway_requests
             WHERE state <> 'started'
               AND traffic_class = 'business'
               AND created_at >= to_timestamp($1::double precision / 1000.0)
             GROUP BY COALESCE(provider_id, 'unrouted')",
        )
        .bind(today_start_ms)
        .fetch_all(pool)
        .await?;
        let mut usage_summary = UsageSummary {
            api_keys_total: api_keys.0,
            api_keys_active: api_keys.1,
            ..UsageSummary::default()
        };
        let mut provider_usage = BTreeMap::new();
        let mut total_duration_ms = 0u64;
        for row in provider_rows {
            let requests = nonnegative_u64(row.try_get("requests")?);
            let successes = nonnegative_u64(row.try_get("successes")?);
            let duration_ms = nonnegative_u64(row.try_get("duration_ms")?);
            let input_tokens = nonnegative_u64(row.try_get("input_tokens")?);
            let output_tokens = nonnegative_u64(row.try_get("output_tokens")?);
            let cache_write_tokens = nonnegative_u64(row.try_get("cache_write_tokens")?);
            let cache_read_tokens = nonnegative_u64(row.try_get("cache_read_tokens")?);
            let cost_microunits: i64 = row.try_get("cost_microunits")?;
            usage_summary.total_requests = usage_summary.total_requests.saturating_add(requests);
            usage_summary.total_successes = usage_summary.total_successes.saturating_add(successes);
            usage_summary.total_input_tokens = usage_summary
                .total_input_tokens
                .saturating_add(input_tokens);
            usage_summary.total_output_tokens = usage_summary
                .total_output_tokens
                .saturating_add(output_tokens);
            usage_summary.total_cache_write_tokens = usage_summary
                .total_cache_write_tokens
                .saturating_add(cache_write_tokens);
            usage_summary.total_cache_read_tokens = usage_summary
                .total_cache_read_tokens
                .saturating_add(cache_read_tokens);
            usage_summary.total_cost_estimate += microunits_usd(cost_microunits);
            total_duration_ms = total_duration_ms.saturating_add(duration_ms);
            provider_usage.insert(
                row.try_get("provider_id")?,
                ProviderUsageStats {
                    requests_total: requests,
                    successes_total: successes,
                    duration_ms_total: duration_ms,
                    input_tokens_total: input_tokens,
                    output_tokens_total: output_tokens,
                    cache_write_tokens_total: cache_write_tokens,
                    cache_read_tokens_total: cache_read_tokens,
                    cost_estimate_usd_total: microunits_usd(cost_microunits),
                },
            );
        }
        usage_summary.average_latency_ms = total_duration_ms
            .checked_div(usage_summary.total_requests)
            .unwrap_or(0);

        let bucket_count =
            usize::try_from((end_ms.saturating_sub(start_ms) / bucket_ms).saturating_add(1))
                .unwrap_or(1)
                .max(1);
        let mut requests = vec![0u64; bucket_count];
        let mut errors = vec![0u64; bucket_count];
        let mut input_tokens = vec![0u64; bucket_count];
        let mut output_tokens = vec![0u64; bucket_count];
        let mut cache_write_tokens = vec![0u64; bucket_count];
        let mut cache_read_tokens = vec![0u64; bucket_count];
        let bucket_rows = sqlx::query(
            "SELECT
                floor(
                    ((EXTRACT(EPOCH FROM created_at) * 1000) - $1::double precision)
                    / $3::double precision
                )::bigint AS bucket_index,
                count(*)::bigint AS requests,
                count(*) FILTER (WHERE state <> 'completed')::bigint AS errors,
                COALESCE(sum(input_tokens), 0)::bigint AS input_tokens,
                COALESCE(sum(output_tokens), 0)::bigint AS output_tokens,
                COALESCE(sum(cache_write_tokens), 0)::bigint AS cache_write_tokens,
                COALESCE(sum(cache_read_tokens), 0)::bigint AS cache_read_tokens,
                COALESCE(sum(cost_amount_microunits), 0)::bigint AS cost_microunits
             FROM modelport_gateway_requests
             WHERE state <> 'started'
               AND traffic_class = 'business'
               AND created_at >= to_timestamp($1::double precision / 1000.0)
               AND created_at <= to_timestamp($2::double precision / 1000.0)
             GROUP BY bucket_index
             ORDER BY bucket_index",
        )
        .bind(start_ms)
        .bind(end_ms)
        .bind(bucket_ms)
        .fetch_all(pool)
        .await?;
        let mut matched_requests = 0u64;
        let mut success_requests = 0u64;
        let mut total_input_tokens = 0u64;
        let mut total_output_tokens = 0u64;
        let mut total_cache_write_tokens = 0u64;
        let mut total_cache_read_tokens = 0u64;
        let mut total_cost_microunits = 0i64;
        for row in bucket_rows {
            let index = usize::try_from(row.try_get::<i64, _>("bucket_index")?)
                .unwrap_or(bucket_count.saturating_sub(1))
                .min(bucket_count.saturating_sub(1));
            let row_requests = nonnegative_u64(row.try_get("requests")?);
            let row_errors = nonnegative_u64(row.try_get("errors")?);
            let row_input_tokens = nonnegative_u64(row.try_get("input_tokens")?);
            let row_output_tokens = nonnegative_u64(row.try_get("output_tokens")?);
            let row_cache_write_tokens = nonnegative_u64(row.try_get("cache_write_tokens")?);
            let row_cache_read_tokens = nonnegative_u64(row.try_get("cache_read_tokens")?);
            let row_cost_microunits: i64 = row.try_get("cost_microunits")?;
            requests[index] = row_requests;
            errors[index] = row_errors;
            input_tokens[index] = row_input_tokens;
            output_tokens[index] = row_output_tokens;
            cache_write_tokens[index] = row_cache_write_tokens;
            cache_read_tokens[index] = row_cache_read_tokens;
            matched_requests = matched_requests.saturating_add(row_requests);
            success_requests =
                success_requests.saturating_add(row_requests.saturating_sub(row_errors));
            total_input_tokens = total_input_tokens.saturating_add(row_input_tokens);
            total_output_tokens = total_output_tokens.saturating_add(row_output_tokens);
            total_cache_write_tokens =
                total_cache_write_tokens.saturating_add(row_cache_write_tokens);
            total_cache_read_tokens = total_cache_read_tokens.saturating_add(row_cache_read_tokens);
            total_cost_microunits = total_cost_microunits.saturating_add(row_cost_microunits);
        }

        let model_rows = sqlx::query(
            "SELECT
                COALESCE(resolved_model, requested_model, 'unknown') AS model,
                COALESCE(provider_id, 'unknown') AS provider,
                count(*)::bigint AS requests,
                COALESCE(sum(
                    input_tokens + output_tokens + cache_write_tokens + cache_read_tokens
                ), 0)::bigint AS tokens,
                COALESCE(sum(cost_amount_microunits), 0)::bigint AS cost_microunits
             FROM modelport_gateway_requests
             WHERE state <> 'started'
               AND traffic_class = 'business'
               AND created_at >= to_timestamp($1::double precision / 1000.0)
               AND created_at <= to_timestamp($2::double precision / 1000.0)
             GROUP BY
                COALESCE(resolved_model, requested_model, 'unknown'),
                COALESCE(provider_id, 'unknown')
             ORDER BY tokens DESC, requests DESC, model ASC
             LIMIT 200",
        )
        .bind(start_ms)
        .bind(end_ms)
        .fetch_all(pool)
        .await?;
        let model_usage = model_rows
            .iter()
            .map(|row| {
                Ok(json!({
                    "model": row.try_get::<String, _>("model")?,
                    "provider": row.try_get::<String, _>("provider")?,
                    "requests": nonnegative_u64(row.try_get("requests")?),
                    "tokens": nonnegative_u64(row.try_get("tokens")?),
                    "cost": microunits_usd(row.try_get("cost_microunits")?),
                }))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        let request_time_series = dashboard_value_series(&requests, start_ms, bucket_ms);
        let error_time_series = dashboard_value_series(&errors, start_ms, bucket_ms);
        let token_time_series = (0..bucket_count)
            .map(|index| {
                let billed_input = input_tokens[index]
                    .saturating_add(cache_write_tokens[index])
                    .saturating_add(cache_read_tokens[index]);
                json!({
                    "timestamp": dashboard_bucket_timestamp(start_ms, bucket_ms, index),
                    "inputTokens": input_tokens[index],
                    "outputTokens": output_tokens[index],
                    "cacheWriteTokens": cache_write_tokens[index],
                    "cacheReadTokens": cache_read_tokens[index],
                    "cacheHitRate": if billed_input == 0 {
                        0.0
                    } else {
                        cache_read_tokens[index] as f64 / billed_input as f64 * 100.0
                    },
                })
            })
            .collect();
        let total_tokens = total_input_tokens
            .saturating_add(total_output_tokens)
            .saturating_add(total_cache_write_tokens)
            .saturating_add(total_cache_read_tokens);
        let minutes = (end_ms.saturating_sub(start_ms) as f64 / 60_000.0).max(1.0);

        Ok(Some(DashboardLedgerSnapshot {
            usage_summary,
            provider_usage,
            matched_requests,
            request_time_series,
            error_time_series,
            token_time_series,
            model_usage,
            summary: json!({
                "totalRequests": matched_requests,
                "successRequests": success_requests,
                "totalInputTokens": total_input_tokens,
                "totalOutputTokens": total_output_tokens,
                "totalCacheWriteTokens": total_cache_write_tokens,
                "totalCacheReadTokens": total_cache_read_tokens,
                "totalTokens": total_tokens,
                "totalCostEstimate": microunits_usd(total_cost_microunits),
                "rpm": matched_requests as f64 / minutes,
                "tpm": total_tokens as f64 / minutes,
            }),
        }))
    }

    pub(crate) async fn latency_stats_since(
        &self,
        since_ms: u64,
    ) -> Result<Option<Value>, AppError> {
        let LedgerBackend::Postgres(pool) = self.backend.as_ref() else {
            return Ok(None);
        };
        let since_ms = i64::try_from(since_ms).unwrap_or(i64::MAX);
        let overall = sqlx::query(
            "SELECT
                percentile_disc(0.50) WITHIN GROUP (ORDER BY latency_ms) AS p50,
                percentile_disc(0.90) WITHIN GROUP (ORDER BY latency_ms) AS p90,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY latency_ms) AS p95,
                percentile_disc(0.99) WITHIN GROUP (ORDER BY latency_ms) AS p99,
                floor(COALESCE(avg(latency_ms), 0))::bigint AS avg,
                COALESCE(max(latency_ms), 0)::bigint AS max,
                count(*)::bigint AS count
             FROM modelport_gateway_requests
             WHERE state <> 'started'
               AND created_at >= to_timestamp($1::double precision / 1000.0)",
        )
        .bind(since_ms)
        .fetch_one(pool)
        .await?;
        let by_model_rows = sqlx::query(
            "SELECT
                COALESCE(resolved_model, requested_model, 'unknown') AS name,
                percentile_disc(0.50) WITHIN GROUP (ORDER BY latency_ms) AS p50,
                percentile_disc(0.90) WITHIN GROUP (ORDER BY latency_ms) AS p90,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY latency_ms) AS p95,
                percentile_disc(0.99) WITHIN GROUP (ORDER BY latency_ms) AS p99,
                floor(COALESCE(avg(latency_ms), 0))::bigint AS avg,
                COALESCE(max(latency_ms), 0)::bigint AS max,
                count(*)::bigint AS count
             FROM modelport_gateway_requests
             WHERE state <> 'started'
               AND created_at >= to_timestamp($1::double precision / 1000.0)
             GROUP BY COALESCE(resolved_model, requested_model, 'unknown')
             ORDER BY count DESC
             LIMIT 200",
        )
        .bind(since_ms)
        .fetch_all(pool)
        .await?;
        let by_provider_rows = sqlx::query(
            "SELECT
                COALESCE(provider_id, 'unrouted') AS name,
                percentile_disc(0.50) WITHIN GROUP (ORDER BY latency_ms) AS p50,
                percentile_disc(0.90) WITHIN GROUP (ORDER BY latency_ms) AS p90,
                percentile_disc(0.95) WITHIN GROUP (ORDER BY latency_ms) AS p95,
                percentile_disc(0.99) WITHIN GROUP (ORDER BY latency_ms) AS p99,
                floor(COALESCE(avg(latency_ms), 0))::bigint AS avg,
                COALESCE(max(latency_ms), 0)::bigint AS max,
                count(*)::bigint AS count
             FROM modelport_gateway_requests
             WHERE state <> 'started'
               AND created_at >= to_timestamp($1::double precision / 1000.0)
             GROUP BY COALESCE(provider_id, 'unrouted')
             ORDER BY count DESC
             LIMIT 200",
        )
        .bind(since_ms)
        .fetch_all(pool)
        .await?;
        let grouped = |rows: Vec<PgRow>| -> Result<Value, sqlx::Error> {
            let mut values = serde_json::Map::new();
            for row in rows {
                values.insert(row.try_get("name")?, latency_stats_from_pg(&row)?);
            }
            Ok(Value::Object(values))
        };

        Ok(Some(json!({
            "p50": optional_nonnegative_u64(&overall, "p50")?,
            "p90": optional_nonnegative_u64(&overall, "p90")?,
            "p95": optional_nonnegative_u64(&overall, "p95")?,
            "p99": optional_nonnegative_u64(&overall, "p99")?,
            "avg": nonnegative_u64(overall.try_get("avg")?),
            "max": nonnegative_u64(overall.try_get("max")?),
            "byModel": grouped(by_model_rows)?,
            "byProvider": grouped(by_provider_rows)?,
            "sampleCount": nonnegative_u64(overall.try_get("count")?),
            "percentilesEstimated": false,
        })))
    }

    pub(crate) async fn usage_row(&self, ledger_id: &str) -> Result<Option<Value>, AppError> {
        let request = match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                ledger.requests.get(ledger_id).map(|request| {
                    memory_request_row(
                        ledger_id,
                        request,
                        usize_to_i64(
                            ledger
                                .attempts
                                .values()
                                .filter(|attempt| attempt.request_ledger_id == ledger_id)
                                .count(),
                        ),
                    )
                })
            }
            LedgerBackend::Postgres(pool) => sqlx::query(REQUEST_DETAIL_SQL)
                .bind(ledger_id)
                .fetch_optional(pool)
                .await?
                .as_ref()
                .map(request_row_from_pg)
                .transpose()?,
        };
        Ok(request
            .filter(|request| request.state != "started")
            .as_ref()
            .map(operational_log_row))
    }

    pub(crate) async fn management_usage(&self) -> Result<ManagementUsageStats, AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let now = u64::try_from(now_millis()).unwrap_or(u64::MAX);
                let day_start = current_period("daily", now).0;
                let month_start = current_period("monthly", now).0;
                let rolling_day_start = now.saturating_sub(24 * 60 * 60 * 1_000);
                let mut stats = ManagementUsageStats::default();
                for request in ledger
                    .requests
                    .values()
                    .filter(|request| request.record.terminal)
                {
                    let created_at = u64::try_from(request.record.created_at_ms).unwrap_or(0);
                    if created_at >= rolling_day_start {
                        let requests = stats
                            .users_24h
                            .entry(request.principal_id.clone())
                            .or_default();
                        *requests = requests.saturating_add(1);
                    }
                    if let Some(api_key_id) = request.api_key_id.as_deref()
                        && created_at >= day_start
                    {
                        let row = stats.api_keys.entry(api_key_id.to_owned()).or_default();
                        row.requests_today = row.requests_today.saturating_add(1);
                        row.tokens_today = row
                            .tokens_today
                            .saturating_add(request_total_tokens(&request.record));
                    }
                    if let Some(team_id) = request.team_id.as_deref()
                        && created_at >= month_start
                    {
                        let row = stats.teams.entry(team_id.to_owned()).or_default();
                        let cost = request
                            .record
                            .billable_cost_microunits
                            .map_or(0.0, microunits_usd);
                        row.monthly_spend_usd += cost;
                        if created_at >= day_start {
                            row.requests_today = row.requests_today.saturating_add(1);
                            row.daily_spend_usd += cost;
                        }
                    }
                }
                Ok(stats)
            }
            LedgerBackend::Postgres(pool) => {
                let api_key_rows = sqlx::query(
                    "SELECT
                        api_key_id,
                        count(*)::bigint AS requests_today,
                        COALESCE(sum(
                            input_tokens + output_tokens
                            + cache_write_tokens + cache_read_tokens
                        ), 0)::bigint AS tokens_today
                     FROM modelport_gateway_requests
                     WHERE state <> 'started'
                       AND api_key_id IS NOT NULL
                       AND created_at >= (
                           date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
                       )
                     GROUP BY api_key_id",
                )
                .fetch_all(pool)
                .await?;
                let team_rows = sqlx::query(
                    "SELECT
                        team_id,
                        count(*) FILTER (
                            WHERE created_at >= (
                                date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
                            )
                        )::bigint AS requests_today,
                        COALESCE(sum(billable_cost_microunits) FILTER (
                            WHERE chargeable
                              AND created_at >= (
                                  date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
                              )
                        ), 0)::bigint AS daily_spend_microunits,
                        COALESCE(sum(billable_cost_microunits) FILTER (
                            WHERE chargeable
                        ), 0)::bigint AS monthly_spend_microunits
                     FROM modelport_gateway_requests
                     WHERE state <> 'started'
                       AND team_id IS NOT NULL
                       AND created_at >= (
                           date_trunc('month', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
                       )
                     GROUP BY team_id",
                )
                .fetch_all(pool)
                .await?;
                let user_rows = sqlx::query(
                    "SELECT principal_id, count(*)::bigint AS requests_24h
                     FROM modelport_gateway_requests
                     WHERE state <> 'started'
                       AND created_at >= now() - interval '24 hours'
                     GROUP BY principal_id",
                )
                .fetch_all(pool)
                .await?;
                let mut stats = ManagementUsageStats::default();
                for row in api_key_rows {
                    stats.api_keys.insert(
                        row.try_get("api_key_id")?,
                        ApiKeyUsageStats {
                            requests_today: nonnegative_u64(row.try_get("requests_today")?),
                            tokens_today: nonnegative_u64(row.try_get("tokens_today")?),
                        },
                    );
                }
                for row in team_rows {
                    stats.teams.insert(
                        row.try_get("team_id")?,
                        TeamUsageStats {
                            requests_today: nonnegative_u64(row.try_get("requests_today")?),
                            daily_spend_usd: microunits_usd(row.try_get("daily_spend_microunits")?),
                            monthly_spend_usd: microunits_usd(
                                row.try_get("monthly_spend_microunits")?,
                            ),
                        },
                    );
                }
                for row in user_rows {
                    stats.users_24h.insert(
                        row.try_get("principal_id")?,
                        nonnegative_u64(row.try_get("requests_24h")?),
                    );
                }
                Ok(stats)
            }
        }
    }

    pub(crate) async fn check_usage_policy(
        &self,
        policy: &UsagePolicySnapshot,
        estimate: UsageEstimate,
        pricing_verified: bool,
    ) -> Result<(), AppError> {
        if policy.user_id.is_empty() {
            return Ok(());
        }
        validate_usage_reservation_pricing(policy, pricing_verified)?;
        let spend = self.usage_spend_totals(policy).await?;
        let limits = &policy.api_key_policy;
        enforce_spend_limit(
            "total spend",
            limits.spend_limit_usd,
            spend.api_key_all_time,
            estimate.cost_estimate,
        )?;
        if limits.rate_limited {
            for (label, limit, used) in [
                (
                    "5 hour spend",
                    limits.five_hour_limit_usd,
                    spend.api_key_five_hours,
                ),
                ("daily spend", limits.daily_limit_usd, spend.api_key_day),
                ("7 day spend", limits.weekly_limit_usd, spend.api_key_week),
                (
                    "monthly spend",
                    limits.monthly_limit_usd,
                    spend.api_key_month,
                ),
            ] {
                enforce_spend_limit(label, limit, used, estimate.cost_estimate)?;
            }
        }
        enforce_spend_limit(
            "team daily spend",
            limits.team_daily_limit_usd,
            spend.team_day,
            estimate.cost_estimate,
        )?;
        enforce_spend_limit(
            "team monthly spend",
            limits.team_monthly_limit_usd,
            spend.team_month,
            estimate.cost_estimate,
        )?;

        for quota in &policy.quotas {
            let used = self.quota_value(quota).await?;
            let increment = quota_increment(&quota.quota_type, estimate);
            if increment > 0.0 && used + increment > quota.limit {
                return Err(AppError::QuotaExceeded(format!(
                    "{} quota exceeded for user {}",
                    quota.quota_type, policy.username
                )));
            }
        }
        Ok(())
    }

    pub(crate) async fn quota_usage_values(
        &self,
        quotas: &[UsageQuotaLimit],
    ) -> Result<HashMap<String, f64>, AppError> {
        let mut values = HashMap::with_capacity(quotas.len());
        for quota in quotas {
            values.insert(quota.id.clone(), self.quota_value(quota).await?);
        }
        Ok(values)
    }

    async fn usage_spend_totals(
        &self,
        policy: &UsagePolicySnapshot,
    ) -> Result<UsageSpendTotals, AppError> {
        let quota_subject_id = policy
            .quota_subject_id
            .as_deref()
            .or(policy.api_key_id.as_deref());
        let mut quota_subject_aliases = policy.quota_subject_aliases.clone();
        if let Some(quota_subject_id) = quota_subject_id {
            quota_subject_aliases.push(quota_subject_id.to_owned());
        }
        if let Some(api_key_id) = policy.api_key_id.as_deref() {
            quota_subject_aliases.push(api_key_id.to_owned());
        }
        quota_subject_aliases.sort();
        quota_subject_aliases.dedup();
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let now = now_millis();
                let mut totals = UsageSpendTotals::default();
                for request in ledger
                    .requests
                    .values()
                    .filter(|request| request.record.terminal && request.record.chargeable)
                {
                    let cost = microunits_usd(request.record.billable_cost_microunits.unwrap_or(0));
                    let age = now.saturating_sub(request.record.created_at_ms);
                    let request_quota_subject_id = request
                        .quota_subject_id
                        .as_deref()
                        .or(request.api_key_id.as_deref());
                    if request_quota_subject_id.is_some_and(|subject| {
                        quota_subject_aliases.iter().any(|item| item == subject)
                    }) {
                        totals.api_key_all_time += cost;
                        if age <= 5 * 60 * 60 * 1_000 {
                            totals.api_key_five_hours += cost;
                        }
                        if age <= 24 * 60 * 60 * 1_000 {
                            totals.api_key_day += cost;
                        }
                        if age <= 7 * 24 * 60 * 60 * 1_000 {
                            totals.api_key_week += cost;
                        }
                        if age <= 30 * 24 * 60 * 60 * 1_000 {
                            totals.api_key_month += cost;
                        }
                    }
                    if request.team_id == policy.team_id && policy.team_id.is_some() {
                        if age <= 24 * 60 * 60 * 1_000 {
                            totals.team_day += cost;
                        }
                        if age <= 30 * 24 * 60 * 60 * 1_000 {
                            totals.team_month += cost;
                        }
                    }
                }
                Ok(totals)
            }
            LedgerBackend::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE quota_subject_id = ANY($1::text[])), 0)::bigint AS api_key_all_time,
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE quota_subject_id = ANY($1::text[])
                                AND created_at >= now() - interval '5 hours'), 0)::bigint
                            AS api_key_five_hours,
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE quota_subject_id = ANY($1::text[])
                                AND created_at >= now() - interval '1 day'), 0)::bigint
                            AS api_key_day,
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE quota_subject_id = ANY($1::text[])
                                AND created_at >= now() - interval '7 days'), 0)::bigint
                            AS api_key_week,
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE quota_subject_id = ANY($1::text[])
                                AND created_at >= now() - interval '30 days'), 0)::bigint
                            AS api_key_month,
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE team_id = $2
                                AND created_at >= now() - interval '1 day'), 0)::bigint
                            AS team_day,
                        COALESCE(sum(billable_cost_microunits)
                            FILTER (WHERE team_id = $2
                                AND created_at >= now() - interval '30 days'), 0)::bigint
                            AS team_month
                     FROM modelport_gateway_requests
                     WHERE state <> 'started'
                       AND chargeable
                       AND ((cardinality($1::text[]) > 0
                                AND quota_subject_id = ANY($1::text[]))
                         OR ($2::text IS NOT NULL AND team_id = $2))",
                )
                .bind(&quota_subject_aliases)
                .bind(policy.team_id.as_deref())
                .fetch_one(pool)
                .await?;
                Ok(UsageSpendTotals {
                    api_key_all_time: microunits_usd(row.try_get("api_key_all_time")?),
                    api_key_five_hours: microunits_usd(row.try_get("api_key_five_hours")?),
                    api_key_day: microunits_usd(row.try_get("api_key_day")?),
                    api_key_week: microunits_usd(row.try_get("api_key_week")?),
                    api_key_month: microunits_usd(row.try_get("api_key_month")?),
                    team_day: microunits_usd(row.try_get("team_day")?),
                    team_month: microunits_usd(row.try_get("team_month")?),
                })
            }
        }
    }

    async fn quota_value(&self, quota: &UsageQuotaLimit) -> Result<f64, AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let mut requests = 0u64;
                let mut tokens = 0u64;
                let mut cost_microunits = 0i64;
                for request in ledger.requests.values().filter(|request| {
                    request.record.terminal
                        && request.record.chargeable
                        && request.principal_id == quota.user_id
                        && request.record.created_at_ms
                            >= i64::try_from(quota.period_start_ms).unwrap_or(i64::MAX)
                }) {
                    requests = requests.saturating_add(1);
                    tokens = tokens
                        .saturating_add(nonnegative_u64(request.record.input_tokens))
                        .saturating_add(nonnegative_u64(request.record.output_tokens))
                        .saturating_add(nonnegative_u64(request.record.cache_write_tokens))
                        .saturating_add(nonnegative_u64(request.record.cache_read_tokens));
                    cost_microunits = cost_microunits.saturating_add(
                        request.record.billable_cost_microunits.unwrap_or(0).max(0),
                    );
                }
                Ok(quota_value_from_totals(
                    &quota.quota_type,
                    requests,
                    tokens,
                    cost_microunits,
                ))
            }
            LedgerBackend::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT
                        count(*)::bigint AS requests,
                        COALESCE(sum(
                            input_tokens + output_tokens
                            + cache_write_tokens + cache_read_tokens
                        ), 0)::bigint AS tokens,
                        COALESCE(sum(billable_cost_microunits), 0)::bigint
                            AS cost_microunits
                     FROM modelport_gateway_requests
                     WHERE principal_id = $1
                       AND state <> 'started'
                       AND chargeable
                       AND created_at >= to_timestamp($2::double precision / 1000.0)",
                )
                .bind(&quota.user_id)
                .bind(i64::try_from(quota.period_start_ms).unwrap_or(i64::MAX))
                .fetch_one(pool)
                .await?;
                Ok(quota_value_from_totals(
                    &quota.quota_type,
                    nonnegative_u64(row.try_get("requests")?),
                    nonnegative_u64(row.try_get("tokens")?),
                    row.try_get("cost_microunits")?,
                ))
            }
        }
    }

    pub(crate) async fn ops_runtime_snapshot(
        &self,
        window_seconds: u64,
    ) -> Result<(OpsRequestWindow, OpsLedgerHealth, Option<u64>), AppError> {
        let window_seconds = window_seconds.clamp(60, 3_600);
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let now = u64::try_from(now_millis()).unwrap_or_default();
                let cutoff = now.saturating_sub(window_seconds.saturating_mul(1_000));
                let mut requests = OpsRequestWindow {
                    window_seconds,
                    ..OpsRequestWindow::default()
                };
                let mut latency_total = 0_u64;
                for request in ledger.requests.values().filter(|request| {
                    u64::try_from(request.record.created_at_ms).unwrap_or(0) >= cutoff
                        && request.traffic_class == "business"
                }) {
                    requests.total_requests = requests.total_requests.saturating_add(1);
                    if request
                        .record
                        .status_code
                        .is_some_and(|status| (200..300).contains(&status))
                    {
                        requests.successful_requests =
                            requests.successful_requests.saturating_add(1);
                    }
                    if request
                        .record
                        .status_code
                        .is_some_and(|status| status >= 500)
                    {
                        requests.server_errors = requests.server_errors.saturating_add(1);
                    }
                    if request.record.status_code == Some(429) {
                        requests.rate_limited = requests.rate_limited.saturating_add(1);
                    }
                    if request.record.tool_outcome == "protocol_error" {
                        requests.protocol_failures = requests.protocol_failures.saturating_add(1);
                    }
                    if request
                        .record
                        .terminal_reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("stream"))
                    {
                        requests.stream_failures = requests.stream_failures.saturating_add(1);
                    }
                    latency_total = latency_total
                        .saturating_add(u64::try_from(request.record.latency_ms).unwrap_or(0));
                }
                requests.average_latency_ms = latency_total
                    .checked_div(requests.total_requests)
                    .unwrap_or_default();
                let mut oldest_open_reservation_age_ms = 0_u64;
                let mut open_usage_reservations = 0_u64;
                for reservation in ledger
                    .usage_reservations
                    .values()
                    .filter(|reservation| reservation.state == "reserved")
                {
                    open_usage_reservations = open_usage_reservations.saturating_add(1);
                    oldest_open_reservation_age_ms = oldest_open_reservation_age_ms.max(
                        now.saturating_sub(u64::try_from(reservation.created_at_ms).unwrap_or(now)),
                    );
                }
                let unreconciled_requests = ledger
                    .requests
                    .values()
                    .filter(|request| {
                        request.record.billing_mode.as_deref() == Some("unreconciled")
                            && u64::try_from(request.record.updated_at_ms).unwrap_or_default()
                                >= now.saturating_sub(24 * 60 * 60 * 1_000)
                    })
                    .count();
                Ok((
                    requests,
                    OpsLedgerHealth {
                        unreconciled_requests: usize_to_u64(unreconciled_requests),
                        open_usage_reservations,
                        oldest_open_reservation_age_ms,
                        budget_accounts_at_or_above_80_percent: 0,
                        budget_accounts_exhausted: 0,
                    },
                    None,
                ))
            }
            LedgerBackend::Postgres(pool) => {
                let request_row = sqlx::query(
                    r#"
                    SELECT
                        count(*)::bigint AS total_requests,
                        count(*) FILTER (
                            WHERE status_code >= 200 AND status_code < 300
                        )::bigint AS successful_requests,
                        count(*) FILTER (WHERE status_code >= 500)::bigint AS server_errors,
                        count(*) FILTER (WHERE status_code = 429)::bigint AS rate_limited,
                        count(*) FILTER (
                            WHERE tool_outcome = 'protocol_error'
                               OR terminal_reason LIKE '%protocol%'
                        )::bigint AS protocol_failures,
                        count(*) FILTER (
                            WHERE terminal_reason LIKE '%stream%'
                               OR terminal_reason = 'downstream_cancelled'
                        )::bigint AS stream_failures,
                        COALESCE(avg(latency_ms), 0)::bigint AS average_latency_ms
                    FROM modelport_gateway_requests
                    WHERE traffic_class = 'business'
                      AND created_at >= now() - ($1::bigint * interval '1 second')
                    "#,
                )
                .bind(i64::try_from(window_seconds).unwrap_or(3_600))
                .fetch_one(pool)
                .await?;
                let ledger_row = sqlx::query(
                    r#"
                    SELECT
                        (SELECT count(*)::bigint
                         FROM modelport_gateway_requests
                         WHERE billing_mode = 'unreconciled'
                           AND updated_at >= now() - interval '24 hours')
                            AS unreconciled_requests,
                        (SELECT count(*)::bigint
                         FROM modelport_usage_reservations
                         WHERE state = 'reserved') AS open_usage_reservations,
                        COALESCE((
                            SELECT (EXTRACT(EPOCH FROM (now() - min(created_at))) * 1000)::bigint
                            FROM modelport_usage_reservations
                            WHERE state = 'reserved'
                        ), 0)::bigint AS oldest_open_reservation_age_ms,
                        (SELECT count(*)::bigint
                         FROM modelport_budget_accounts
                         WHERE limit_microunits IS NOT NULL
                           AND limit_microunits > 0
                           AND reserved_microunits + settled_microunits
                               >= limit_microunits * 0.8) AS budget_warning,
                        (SELECT count(*)::bigint
                         FROM modelport_budget_accounts
                         WHERE limit_microunits IS NOT NULL
                           AND reserved_microunits + settled_microunits
                               >= limit_microunits) AS budget_exhausted,
                        (SELECT max((EXTRACT(EPOCH FROM occurred_at) * 1000)::bigint)
                         FROM modelport_audit_events
                         WHERE activity_type IN ('config_change', 'high_risk_change_applied'))
                            AS recent_change_at_ms
                    "#,
                )
                .fetch_one(pool)
                .await?;
                Ok((
                    OpsRequestWindow {
                        window_seconds,
                        total_requests: nonnegative_u64(request_row.try_get("total_requests")?),
                        successful_requests: nonnegative_u64(
                            request_row.try_get("successful_requests")?,
                        ),
                        server_errors: nonnegative_u64(request_row.try_get("server_errors")?),
                        rate_limited: nonnegative_u64(request_row.try_get("rate_limited")?),
                        protocol_failures: nonnegative_u64(
                            request_row.try_get("protocol_failures")?,
                        ),
                        stream_failures: nonnegative_u64(request_row.try_get("stream_failures")?),
                        average_latency_ms: nonnegative_u64(
                            request_row.try_get("average_latency_ms")?,
                        ),
                    },
                    OpsLedgerHealth {
                        unreconciled_requests: nonnegative_u64(
                            ledger_row.try_get("unreconciled_requests")?,
                        ),
                        open_usage_reservations: nonnegative_u64(
                            ledger_row.try_get("open_usage_reservations")?,
                        ),
                        oldest_open_reservation_age_ms: nonnegative_u64(
                            ledger_row.try_get("oldest_open_reservation_age_ms")?,
                        ),
                        budget_accounts_at_or_above_80_percent: nonnegative_u64(
                            ledger_row.try_get("budget_warning")?,
                        ),
                        budget_accounts_exhausted: nonnegative_u64(
                            ledger_row.try_get("budget_exhausted")?,
                        ),
                    },
                    ledger_row
                        .try_get::<Option<i64>, _>("recent_change_at_ms")?
                        .and_then(|value| u64::try_from(value).ok()),
                ))
            }
        }
    }

    pub(crate) async fn upsert_ops_observation(
        &self,
        observation: &OpsObservation,
        actor_id: &str,
        actor_name: &str,
    ) -> Result<Option<OpsIncidentSummary>, AppError> {
        validate_ops_observation(observation)?;
        validate_ops_actor(actor_id, actor_name)?;
        let evidence_hash = ops_evidence_hash(&observation.evidence)?;
        let observed_at_ms = to_i64(observation.observed_at_ms);

        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let incident_id = ledger.ops_event_index.get(&observation.event_key).cloned();
                let Some(incident_id) = incident_id else {
                    if !observation.active {
                        return Ok(None);
                    }
                    let incident_id = format!("opi_{}", Uuid::new_v4().simple());
                    let evidence_id = format!("ope_{}", Uuid::new_v4().simple());
                    let timeline_id = format!("opt_{}", Uuid::new_v4().simple());
                    let summary = OpsIncidentSummary {
                        id: incident_id.clone(),
                        event_key: observation.event_key.clone(),
                        detector_type: observation.detector_type.clone(),
                        severity: observation.severity,
                        status: OpsIncidentStatus::Open,
                        title: observation.title.clone(),
                        summary: observation.summary.clone(),
                        affected_scope: observation.affected_scope.clone(),
                        recovery_criteria: observation.recovery_criteria.clone(),
                        first_seen_at_ms: observation.observed_at_ms,
                        last_seen_at_ms: observation.observed_at_ms,
                        resolved_at_ms: None,
                        occurrence_count: 1,
                    };
                    ledger
                        .ops_event_index
                        .insert(observation.event_key.clone(), incident_id.clone());
                    ledger.ops_incidents.insert(
                        incident_id.clone(),
                        OpsIncidentDetail {
                            incident: summary.clone(),
                            evidence: vec![OpsIncidentEvidence {
                                id: evidence_id,
                                incident_id: incident_id.clone(),
                                observed_at_ms: observation.observed_at_ms,
                                evidence: observation.evidence.clone(),
                            }],
                            timeline: vec![OpsIncidentTimelineEntry {
                                id: timeline_id,
                                incident_id,
                                event_type: "detected".to_owned(),
                                actor_id: actor_id.to_owned(),
                                actor_name: actor_name.to_owned(),
                                message: "deterministic detector opened the incident".to_owned(),
                                occurred_at_ms: observation.observed_at_ms,
                            }],
                        },
                    );
                    return Ok(Some(summary));
                };

                let detail = ledger
                    .ops_incidents
                    .get_mut(&incident_id)
                    .expect("ops event index must reference an incident");
                let previous_status = detail.incident.status;
                if !observation.active && previous_status == OpsIncidentStatus::Resolved {
                    return Ok(Some(detail.incident.clone()));
                }
                detail.incident.detector_type = observation.detector_type.clone();
                detail.incident.severity = observation.severity;
                detail.incident.title = observation.title.clone();
                detail.incident.summary = observation.summary.clone();
                detail.incident.affected_scope = observation.affected_scope.clone();
                detail.incident.recovery_criteria = observation.recovery_criteria.clone();
                detail.incident.last_seen_at_ms = observation.observed_at_ms;
                let event_type = if observation.active {
                    if previous_status == OpsIncidentStatus::Resolved {
                        detail.incident.status = OpsIncidentStatus::Open;
                        detail.incident.resolved_at_ms = None;
                        detail.incident.occurrence_count =
                            detail.incident.occurrence_count.saturating_add(1);
                        Some(("reopened", "recovery criteria no longer hold"))
                    } else {
                        None
                    }
                } else if previous_status != OpsIncidentStatus::Resolved {
                    detail.incident.status = OpsIncidentStatus::Resolved;
                    detail.incident.resolved_at_ms = Some(observation.observed_at_ms);
                    Some(("resolved", "deterministic recovery criteria were satisfied"))
                } else {
                    None
                };
                let already_recorded = detail.evidence.iter().any(|existing| {
                    ops_evidence_hash(&existing.evidence).ok().as_deref()
                        == Some(evidence_hash.as_str())
                });
                if !already_recorded {
                    detail.evidence.push(OpsIncidentEvidence {
                        id: format!("ope_{}", Uuid::new_v4().simple()),
                        incident_id: incident_id.clone(),
                        observed_at_ms: observation.observed_at_ms,
                        evidence: observation.evidence.clone(),
                    });
                }
                if let Some((event_type, message)) = event_type {
                    detail.timeline.push(OpsIncidentTimelineEntry {
                        id: format!("opt_{}", Uuid::new_v4().simple()),
                        incident_id,
                        event_type: event_type.to_owned(),
                        actor_id: actor_id.to_owned(),
                        actor_name: actor_name.to_owned(),
                        message: message.to_owned(),
                        occurred_at_ms: observation.observed_at_ms,
                    });
                }
                Ok(Some(detail.incident.clone()))
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let existing = sqlx::query(
                    "SELECT incident_id, status
                     FROM modelport_ops_incidents
                     WHERE event_key = $1
                     FOR UPDATE",
                )
                .bind(&observation.event_key)
                .fetch_optional(&mut *transaction)
                .await?;
                let (incident_id, timeline_event, timeline_message) = if let Some(row) = existing {
                    let incident_id: String = row.try_get("incident_id")?;
                    let previous_status: String = row.try_get("status")?;
                    if !observation.active && previous_status == "resolved" {
                        let row = fetch_ops_incident_row(&mut transaction, &incident_id).await?;
                        transaction.commit().await?;
                        return Ok(Some(ops_incident_summary_from_row(&row)?));
                    }
                    let (next_status, resolved_at, occurrence_increment, timeline) = if observation
                        .active
                        && previous_status == "resolved"
                    {
                        (
                            "open",
                            None,
                            1_i64,
                            Some(("reopened", "recovery criteria no longer hold")),
                        )
                    } else if !observation.active && previous_status != "resolved" {
                        (
                            "resolved",
                            Some(observed_at_ms),
                            0_i64,
                            Some(("resolved", "deterministic recovery criteria were satisfied")),
                        )
                    } else {
                        (previous_status.as_str(), None, 0_i64, None)
                    };
                    sqlx::query(
                        "UPDATE modelport_ops_incidents
                         SET detector_type = $2, severity = $3, status = $4,
                             title = $5, summary = $6, affected_scope = $7,
                             recovery_criteria = $8,
                             last_seen_at = to_timestamp($9::double precision / 1000.0),
                             resolved_at = CASE
                                 WHEN $4 = 'resolved' THEN COALESCE(
                                     resolved_at,
                                     to_timestamp($10::double precision / 1000.0)
                                 )
                                 ELSE NULL
                             END,
                             occurrence_count = occurrence_count + $11,
                             updated_at = now()
                         WHERE incident_id = $1",
                    )
                    .bind(&incident_id)
                    .bind(&observation.detector_type)
                    .bind(observation.severity.as_str())
                    .bind(next_status)
                    .bind(&observation.title)
                    .bind(&observation.summary)
                    .bind(&observation.affected_scope)
                    .bind(&observation.recovery_criteria)
                    .bind(observed_at_ms)
                    .bind(resolved_at)
                    .bind(occurrence_increment)
                    .execute(&mut *transaction)
                    .await?;
                    (
                        incident_id,
                        timeline.map(|value| value.0),
                        timeline.map(|value| value.1),
                    )
                } else {
                    if !observation.active {
                        transaction.commit().await?;
                        return Ok(None);
                    }
                    let incident_id = format!("opi_{}", Uuid::new_v4().simple());
                    sqlx::query(
                        "INSERT INTO modelport_ops_incidents (
                            incident_id, event_key, detector_type, severity, status,
                            title, summary, affected_scope, recovery_criteria,
                            first_seen_at, last_seen_at
                         ) VALUES (
                            $1, $2, $3, $4, 'open', $5, $6, $7, $8,
                            to_timestamp($9::double precision / 1000.0),
                            to_timestamp($9::double precision / 1000.0)
                         )",
                    )
                    .bind(&incident_id)
                    .bind(&observation.event_key)
                    .bind(&observation.detector_type)
                    .bind(observation.severity.as_str())
                    .bind(&observation.title)
                    .bind(&observation.summary)
                    .bind(&observation.affected_scope)
                    .bind(&observation.recovery_criteria)
                    .bind(observed_at_ms)
                    .execute(&mut *transaction)
                    .await?;
                    (
                        incident_id,
                        Some("detected"),
                        Some("deterministic detector opened the incident"),
                    )
                };

                sqlx::query(
                    "INSERT INTO modelport_ops_incident_evidence (
                        evidence_id, incident_id, evidence_hash, observed_at, evidence
                     ) VALUES (
                        $1, $2, $3, to_timestamp($4::double precision / 1000.0), $5
                     ) ON CONFLICT (incident_id, evidence_hash) DO NOTHING",
                )
                .bind(format!("ope_{}", Uuid::new_v4().simple()))
                .bind(&incident_id)
                .bind(&evidence_hash)
                .bind(observed_at_ms)
                .bind(&observation.evidence)
                .execute(&mut *transaction)
                .await?;
                if let (Some(event_type), Some(message)) = (timeline_event, timeline_message) {
                    sqlx::query(
                        "INSERT INTO modelport_ops_incident_timeline (
                            timeline_id, incident_id, event_type, actor_id,
                            actor_name, message, occurred_at
                         ) VALUES (
                            $1, $2, $3, $4, $5, $6,
                            to_timestamp($7::double precision / 1000.0)
                         )",
                    )
                    .bind(format!("opt_{}", Uuid::new_v4().simple()))
                    .bind(&incident_id)
                    .bind(event_type)
                    .bind(actor_id)
                    .bind(actor_name)
                    .bind(message)
                    .bind(observed_at_ms)
                    .execute(&mut *transaction)
                    .await?;
                }
                let row = fetch_ops_incident_row(&mut transaction, &incident_id).await?;
                transaction.commit().await?;
                Ok(Some(ops_incident_summary_from_row(&row)?))
            }
        }
    }

    pub(crate) async fn record_ops_heartbeat(
        &self,
        heartbeat: &OpsHeartbeat,
    ) -> Result<(), AppError> {
        validate_ops_heartbeat(heartbeat)?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                ledger
                    .lock()
                    .expect("enterprise ledger lock poisoned")
                    .ops_heartbeats
                    .insert(heartbeat.instance_id.clone(), heartbeat.clone());
            }
            LedgerBackend::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO modelport_ops_agent_heartbeats (
                        instance_id, agent_version, mode, rule_set_version,
                        queue_depth, interval_seconds, analysis_enabled,
                        selected_model, model_status, model_last_success_at, observed_at
                     ) VALUES (
                        $1, $2, $3, $4, $5, $6, $7, $8, $9,
                        CASE WHEN $10::bigint IS NULL THEN NULL
                             ELSE to_timestamp($10::double precision / 1000.0) END,
                        to_timestamp($11::double precision / 1000.0)
                     )
                     ON CONFLICT (instance_id) DO UPDATE SET
                        agent_version = EXCLUDED.agent_version,
                        mode = EXCLUDED.mode,
                        rule_set_version = EXCLUDED.rule_set_version,
                        queue_depth = EXCLUDED.queue_depth,
                        interval_seconds = EXCLUDED.interval_seconds,
                        analysis_enabled = EXCLUDED.analysis_enabled,
                        selected_model = EXCLUDED.selected_model,
                        model_status = EXCLUDED.model_status,
                        model_last_success_at = EXCLUDED.model_last_success_at,
                        observed_at = EXCLUDED.observed_at,
                        updated_at = now()",
                )
                .bind(&heartbeat.instance_id)
                .bind(&heartbeat.agent_version)
                .bind(&heartbeat.mode)
                .bind(&heartbeat.rule_set_version)
                .bind(to_i64(heartbeat.queue_depth))
                .bind(to_i64(heartbeat.interval_seconds))
                .bind(heartbeat.analysis_enabled)
                .bind(&heartbeat.selected_model)
                .bind(&heartbeat.model_status)
                .bind(heartbeat.model_last_success_at_ms.map(to_i64))
                .bind(to_i64(heartbeat.observed_at_ms))
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn list_ops_incidents(
        &self,
        status: Option<OpsIncidentStatus>,
        limit: usize,
    ) -> Result<OpsIncidentList, AppError> {
        let limit = limit.clamp(1, 500);
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let mut items = ledger
                    .ops_incidents
                    .values()
                    .map(|detail| detail.incident.clone())
                    .filter(|incident| status.is_none_or(|status| incident.status == status))
                    .collect::<Vec<_>>();
                items.sort_by(|left, right| {
                    right
                        .last_seen_at_ms
                        .cmp(&left.last_seen_at_ms)
                        .then_with(|| right.id.cmp(&left.id))
                });
                items.truncate(limit);
                let open_items = ledger
                    .ops_incidents
                    .values()
                    .filter(|detail| detail.incident.status != OpsIncidentStatus::Resolved)
                    .map(|detail| detail.incident.severity)
                    .collect::<Vec<_>>();
                Ok(OpsIncidentList {
                    total: usize_to_u64(ledger.ops_incidents.len()),
                    open: usize_to_u64(open_items.len()),
                    highest_open_severity: highest_ops_severity(open_items),
                    items,
                    agents: ledger
                        .ops_heartbeats
                        .values()
                        .map(ops_agent_summary)
                        .collect(),
                })
            }
            LedgerBackend::Postgres(pool) => {
                let rows = if let Some(status) = status {
                    sqlx::query(
                        "SELECT *,
                            (EXTRACT(EPOCH FROM first_seen_at) * 1000)::bigint AS first_seen_at_ms,
                            (EXTRACT(EPOCH FROM last_seen_at) * 1000)::bigint AS last_seen_at_ms,
                            (EXTRACT(EPOCH FROM resolved_at) * 1000)::bigint AS resolved_at_ms
                         FROM modelport_ops_incidents
                         WHERE status = $1
                         ORDER BY last_seen_at DESC, incident_id DESC
                         LIMIT $2",
                    )
                    .bind(status.as_str())
                    .bind(usize_to_i64(limit))
                    .fetch_all(pool)
                    .await?
                } else {
                    sqlx::query(
                        "SELECT *,
                            (EXTRACT(EPOCH FROM first_seen_at) * 1000)::bigint AS first_seen_at_ms,
                            (EXTRACT(EPOCH FROM last_seen_at) * 1000)::bigint AS last_seen_at_ms,
                            (EXTRACT(EPOCH FROM resolved_at) * 1000)::bigint AS resolved_at_ms
                         FROM modelport_ops_incidents
                         ORDER BY last_seen_at DESC, incident_id DESC
                         LIMIT $1",
                    )
                    .bind(usize_to_i64(limit))
                    .fetch_all(pool)
                    .await?
                };
                let items = rows
                    .iter()
                    .map(ops_incident_summary_from_row)
                    .collect::<Result<Vec<_>, _>>()?;
                let aggregate = sqlx::query(
                    "SELECT count(*)::bigint AS total,
                            count(*) FILTER (WHERE status <> 'resolved')::bigint AS open,
                            min(CASE severity
                                WHEN 'SEV-1' THEN 1 WHEN 'SEV-2' THEN 2
                                WHEN 'SEV-3' THEN 3 WHEN 'SEV-4' THEN 4
                                ELSE 5 END
                            ) FILTER (WHERE status <> 'resolved') AS highest
                     FROM modelport_ops_incidents",
                )
                .fetch_one(pool)
                .await?;
                let agent_rows = sqlx::query(
                    "SELECT instance_id, agent_version, mode, rule_set_version, queue_depth,
                            interval_seconds, analysis_enabled, selected_model, model_status,
                            (EXTRACT(EPOCH FROM model_last_success_at) * 1000)::bigint
                                AS model_last_success_at_ms,
                            (EXTRACT(EPOCH FROM observed_at) * 1000)::bigint AS observed_at_ms
                     FROM modelport_ops_agent_heartbeats
                     ORDER BY observed_at DESC, instance_id",
                )
                .fetch_all(pool)
                .await?;
                Ok(OpsIncidentList {
                    items,
                    total: nonnegative_u64(aggregate.try_get("total")?),
                    open: nonnegative_u64(aggregate.try_get("open")?),
                    highest_open_severity: aggregate
                        .try_get::<Option<i32>, _>("highest")?
                        .and_then(ops_severity_from_rank),
                    agents: agent_rows
                        .iter()
                        .map(ops_agent_summary_from_row)
                        .collect::<Result<Vec<_>, _>>()?,
                })
            }
        }
    }

    pub(crate) async fn ops_incident_detail(
        &self,
        incident_id: &str,
    ) -> Result<OpsIncidentDetail, AppError> {
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => ledger
                .lock()
                .expect("enterprise ledger lock poisoned")
                .ops_incidents
                .get(incident_id)
                .cloned()
                .ok_or_else(|| AppError::NotFound("operations incident not found".to_owned())),
            LedgerBackend::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT *,
                        (EXTRACT(EPOCH FROM first_seen_at) * 1000)::bigint AS first_seen_at_ms,
                        (EXTRACT(EPOCH FROM last_seen_at) * 1000)::bigint AS last_seen_at_ms,
                        (EXTRACT(EPOCH FROM resolved_at) * 1000)::bigint AS resolved_at_ms
                     FROM modelport_ops_incidents WHERE incident_id = $1",
                )
                .bind(incident_id)
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| AppError::NotFound("operations incident not found".to_owned()))?;
                let evidence = sqlx::query(
                    "SELECT evidence_id, evidence,
                            (EXTRACT(EPOCH FROM observed_at) * 1000)::bigint AS observed_at_ms
                     FROM modelport_ops_incident_evidence
                     WHERE incident_id = $1
                     ORDER BY observed_at DESC, evidence_id DESC LIMIT 100",
                )
                .bind(incident_id)
                .fetch_all(pool)
                .await?
                .iter()
                .map(|row| {
                    Ok(OpsIncidentEvidence {
                        id: row.try_get("evidence_id")?,
                        incident_id: incident_id.to_owned(),
                        observed_at_ms: nonnegative_u64(row.try_get("observed_at_ms")?),
                        evidence: row.try_get("evidence")?,
                    })
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()?;
                let timeline = sqlx::query(
                    "SELECT timeline_id, event_type, actor_id, actor_name, message,
                            (EXTRACT(EPOCH FROM occurred_at) * 1000)::bigint AS occurred_at_ms
                     FROM modelport_ops_incident_timeline
                     WHERE incident_id = $1
                     ORDER BY occurred_at, timeline_id LIMIT 500",
                )
                .bind(incident_id)
                .fetch_all(pool)
                .await?
                .iter()
                .map(|row| {
                    Ok(OpsIncidentTimelineEntry {
                        id: row.try_get("timeline_id")?,
                        incident_id: incident_id.to_owned(),
                        event_type: row.try_get("event_type")?,
                        actor_id: row.try_get("actor_id")?,
                        actor_name: row.try_get("actor_name")?,
                        message: row.try_get("message")?,
                        occurred_at_ms: nonnegative_u64(row.try_get("occurred_at_ms")?),
                    })
                })
                .collect::<Result<Vec<_>, sqlx::Error>>()?;
                Ok(OpsIncidentDetail {
                    incident: ops_incident_summary_from_row(&row)?,
                    evidence,
                    timeline,
                })
            }
        }
    }

    pub(crate) async fn update_ops_incident_status(
        &self,
        incident_id: &str,
        update: &OpsIncidentStatusUpdate,
        actor_id: &str,
        actor_name: &str,
    ) -> Result<OpsIncidentSummary, AppError> {
        validate_ops_actor(actor_id, actor_name)?;
        let reason = update.reason.trim();
        if reason.is_empty() || reason.len() > 1_000 {
            return Err(AppError::InvalidRequest(
                "incident status reason must contain 1 to 1000 characters".to_owned(),
            ));
        }
        if matches!(
            update.status,
            OpsIncidentStatus::Resolved | OpsIncidentStatus::Open
        ) {
            return Err(AppError::InvalidRequest(
                "open/resolved status is controlled by deterministic detector evidence".to_owned(),
            ));
        }
        let now = u64::try_from(now_millis()).unwrap_or_default();
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let detail = ledger.ops_incidents.get_mut(incident_id).ok_or_else(|| {
                    AppError::NotFound("operations incident not found".to_owned())
                })?;
                if detail.incident.status == OpsIncidentStatus::Resolved {
                    return Err(AppError::StateConflict(
                        "resolved incidents cannot be manually reopened".to_owned(),
                    ));
                }
                detail.incident.status = update.status;
                detail.timeline.push(OpsIncidentTimelineEntry {
                    id: format!("opt_{}", Uuid::new_v4().simple()),
                    incident_id: incident_id.to_owned(),
                    event_type: "status_changed".to_owned(),
                    actor_id: actor_id.to_owned(),
                    actor_name: actor_name.to_owned(),
                    message: reason.to_owned(),
                    occurred_at_ms: now,
                });
                Ok(detail.incident.clone())
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let result = sqlx::query(
                    "UPDATE modelport_ops_incidents
                     SET status = $2, updated_at = now()
                     WHERE incident_id = $1 AND status <> 'resolved'",
                )
                .bind(incident_id)
                .bind(update.status.as_str())
                .execute(&mut *transaction)
                .await?;
                if result.rows_affected() == 0 {
                    let exists = sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS(
                            SELECT 1 FROM modelport_ops_incidents WHERE incident_id = $1
                         )",
                    )
                    .bind(incident_id)
                    .fetch_one(&mut *transaction)
                    .await?;
                    return Err(if exists {
                        AppError::StateConflict(
                            "resolved incidents cannot be manually reopened".to_owned(),
                        )
                    } else {
                        AppError::NotFound("operations incident not found".to_owned())
                    });
                }
                sqlx::query(
                    "INSERT INTO modelport_ops_incident_timeline (
                        timeline_id, incident_id, event_type, actor_id, actor_name, message
                     ) VALUES ($1, $2, 'status_changed', $3, $4, $5)",
                )
                .bind(format!("opt_{}", Uuid::new_v4().simple()))
                .bind(incident_id)
                .bind(actor_id)
                .bind(actor_name)
                .bind(reason)
                .execute(&mut *transaction)
                .await?;
                let row = fetch_ops_incident_row(&mut transaction, incident_id).await?;
                transaction.commit().await?;
                Ok(ops_incident_summary_from_row(&row)?)
            }
        }
    }

    pub(crate) async fn record_ops_incident_feedback(
        &self,
        incident_id: &str,
        feedback: &OpsIncidentFeedbackInput,
        actor_id: &str,
        actor_name: &str,
    ) -> Result<(), AppError> {
        validate_ops_actor(actor_id, actor_name)?;
        if !matches!(
            feedback.outcome.as_str(),
            "true_positive" | "false_positive" | "needs_review"
        ) {
            return Err(AppError::InvalidRequest(
                "feedback outcome must be true_positive, false_positive, or needs_review"
                    .to_owned(),
            ));
        }
        if feedback
            .note
            .as_deref()
            .is_some_and(|note| note.len() > 1_000)
        {
            return Err(AppError::InvalidRequest(
                "feedback note must not exceed 1000 characters".to_owned(),
            ));
        }
        let message = format!("incident feedback recorded: {}", feedback.outcome);
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let detail = ledger.ops_incidents.get_mut(incident_id).ok_or_else(|| {
                    AppError::NotFound("operations incident not found".to_owned())
                })?;
                detail.timeline.push(OpsIncidentTimelineEntry {
                    id: format!("opt_{}", Uuid::new_v4().simple()),
                    incident_id: incident_id.to_owned(),
                    event_type: "feedback".to_owned(),
                    actor_id: actor_id.to_owned(),
                    actor_name: actor_name.to_owned(),
                    message,
                    occurred_at_ms: u64::try_from(now_millis()).unwrap_or_default(),
                });
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let exists = sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(
                        SELECT 1 FROM modelport_ops_incidents WHERE incident_id = $1
                     )",
                )
                .bind(incident_id)
                .fetch_one(&mut *transaction)
                .await?;
                if !exists {
                    return Err(AppError::NotFound(
                        "operations incident not found".to_owned(),
                    ));
                }
                sqlx::query(
                    "INSERT INTO modelport_ops_incident_feedback (
                        feedback_id, incident_id, actor_id, actor_name, outcome,
                        root_cause_correct, recommendation_adopted, note
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                )
                .bind(format!("opf_{}", Uuid::new_v4().simple()))
                .bind(incident_id)
                .bind(actor_id)
                .bind(actor_name)
                .bind(&feedback.outcome)
                .bind(feedback.root_cause_correct)
                .bind(feedback.recommendation_adopted)
                .bind(feedback.note.as_deref())
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT INTO modelport_ops_incident_timeline (
                        timeline_id, incident_id, event_type, actor_id, actor_name, message
                     ) VALUES ($1, $2, 'feedback', $3, $4, $5)",
                )
                .bind(format!("opt_{}", Uuid::new_v4().simple()))
                .bind(incident_id)
                .bind(actor_id)
                .bind(actor_name)
                .bind(message)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn record_audit_event(&self, input: &AuditEventInput) -> Result<(), AppError> {
        validate_audit_event(input)?;
        let event = EnterpriseAuditEvent {
            id: format!("aev_{}", Uuid::new_v4().simple()),
            timestamp: now_millis().to_string(),
            activity_type: input.activity_type.clone(),
            actor_id: input.actor_id.clone(),
            actor: input.actor_name.clone(),
            target: input.target.clone(),
            message: input.message.clone(),
            severity: input.severity.clone(),
        };
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                ledger
                    .lock()
                    .expect("enterprise ledger lock poisoned")
                    .audit_events
                    .push(event);
            }
            LedgerBackend::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO modelport_audit_events (
                        event_id, activity_type, actor_id, actor_name,
                        target, message, severity
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(&event.id)
                .bind(&event.activity_type)
                .bind(&event.actor_id)
                .bind(&event.actor)
                .bind(&event.target)
                .bind(&event.message)
                .bind(&event.severity)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn audit_events(&self, limit: usize) -> Result<(Vec<Value>, i64), AppError> {
        let limit = limit.clamp(1, 1_000);
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let total = usize_to_i64(ledger.audit_events.len());
                let rows = ledger
                    .audit_events
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|event| json!(event))
                    .collect();
                Ok((rows, total))
            }
            LedgerBackend::Postgres(pool) => {
                let total =
                    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM modelport_audit_events")
                        .fetch_one(pool)
                        .await?;
                let rows = sqlx::query(
                    "SELECT event_id, activity_type, actor_id, actor_name,
                            target, message, severity,
                            (EXTRACT(EPOCH FROM occurred_at) * 1000)::bigint AS occurred_at_ms
                     FROM modelport_audit_events
                     ORDER BY occurred_at DESC, event_id DESC
                     LIMIT $1",
                )
                .bind(usize_to_i64(limit))
                .fetch_all(pool)
                .await?
                .iter()
                .map(|row| {
                    Ok(json!({
                        "id": row.try_get::<String, _>("event_id")?,
                        "timestamp": row.try_get::<i64, _>("occurred_at_ms")?.to_string(),
                        "type": row.try_get::<String, _>("activity_type")?,
                        "actorId": row.try_get::<String, _>("actor_id")?,
                        "actor": row.try_get::<String, _>("actor_name")?,
                        "target": row.try_get::<String, _>("target")?,
                        "message": row.try_get::<String, _>("message")?,
                        "severity": row.try_get::<String, _>("severity")?,
                    }))
                })
                .collect::<Result<Vec<Value>, sqlx::Error>>()?;
                Ok((rows, total))
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

    pub(crate) async fn run_retention(
        &self,
        policy: RetentionPolicy,
        dry_run: bool,
    ) -> Result<RetentionResult, AppError> {
        let evaluated_at_ms = nonnegative_u64(now_millis());
        self.run_retention_at(policy, dry_run, evaluated_at_ms)
            .await
    }

    pub(crate) async fn run_retention_at(
        &self,
        policy: RetentionPolicy,
        dry_run: bool,
        evaluated_at_ms: u64,
    ) -> Result<RetentionResult, AppError> {
        let detail_cutoff_ms = evaluated_at_ms
            .saturating_sub(policy.request_detail_days.saturating_mul(MILLIS_PER_DAY));
        let usage_cutoff_ms =
            evaluated_at_ms.saturating_sub(policy.user_usage_days.saturating_mul(MILLIS_PER_DAY));
        let audit_cutoff_ms =
            evaluated_at_ms.saturating_sub(policy.audit_days.saturating_mul(MILLIS_PER_DAY));
        let mutation_allowed = !dry_run && !policy.legal_hold;

        let counts = match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let detail_request_ids = ledger
                    .requests
                    .iter()
                    .filter(|(_, request)| {
                        request.record.terminal
                            && request.record.created_at_ms >= 0
                            && (request.record.created_at_ms as u64) < detail_cutoff_ms
                            && !request.request_id.starts_with(RETAINED_REQUEST_ID_PREFIX)
                    })
                    .map(|(ledger_id, _)| ledger_id.clone())
                    .collect::<HashSet<_>>();
                let usage_request_ids = ledger
                    .requests
                    .iter()
                    .filter(|(_, request)| {
                        request.record.terminal
                            && request.record.created_at_ms >= 0
                            && (request.record.created_at_ms as u64) < usage_cutoff_ms
                            && request.principal_id != RETAINED_PRINCIPAL_ID
                    })
                    .map(|(ledger_id, _)| ledger_id.clone())
                    .collect::<HashSet<_>>();
                let usage_reservation_ids = ledger
                    .usage_reservations
                    .iter()
                    .filter(|(_, reservation)| {
                        reservation.state != "reserved"
                            && reservation.created_at_ms >= 0
                            && (reservation.created_at_ms as u64) < usage_cutoff_ms
                            && reservation.user_id != RETAINED_PRINCIPAL_ID
                    })
                    .map(|(ledger_id, _)| ledger_id.clone())
                    .collect::<HashSet<_>>();
                let provider_attempt_ids = ledger
                    .attempts
                    .iter()
                    .filter(|(_, attempt)| {
                        attempt.terminal && detail_request_ids.contains(&attempt.request_ledger_id)
                    })
                    .map(|(attempt_id, _)| attempt_id.clone())
                    .collect::<HashSet<_>>();
                let routing_decisions_deleted = detail_request_ids
                    .iter()
                    .filter(|ledger_id| {
                        ledger
                            .requests
                            .get(*ledger_id)
                            .is_some_and(|request| request.routing_decision.is_some())
                    })
                    .count() as u64;
                let expired_audit_ids = ledger
                    .audit_events
                    .iter()
                    .filter(|event| {
                        event
                            .timestamp
                            .parse::<u64>()
                            .is_ok_and(|timestamp| timestamp < audit_cutoff_ms)
                    })
                    .map(|event| event.id.clone())
                    .collect::<HashSet<_>>();
                let counts = RetentionCounts {
                    request_details_redacted: detail_request_ids.len() as u64,
                    provider_attempts_redacted: provider_attempt_ids.len() as u64,
                    routing_decisions_deleted,
                    user_usage_rows_deidentified: usize_to_u64(
                        usage_request_ids
                            .len()
                            .saturating_add(usage_reservation_ids.len()),
                    ),
                    audit_events_deleted: expired_audit_ids.len() as u64,
                };

                if mutation_allowed {
                    for attempt_id in provider_attempt_ids {
                        if let Some(attempt) = ledger.attempts.get_mut(&attempt_id) {
                            attempt.error_message = None;
                            attempt.terminal_reason =
                                Some("retained_financial_evidence".to_owned());
                            attempt.lease_owner = "retained".to_owned();
                        }
                    }
                    for ledger_id in detail_request_ids {
                        if let Some(request) = ledger.requests.get_mut(&ledger_id) {
                            request.request_id = format!("{RETAINED_REQUEST_ID_PREFIX}{ledger_id}");
                            request.client_ip = None;
                            request.idempotency_key_hash = None;
                            request.request_fingerprint = retained_request_fingerprint(&ledger_id);
                            request.routing_decision = None;
                            request.record.error_message = None;
                        }
                    }
                    for ledger_id in usage_request_ids {
                        if let Some(request) = ledger.requests.get_mut(&ledger_id) {
                            request.principal_id = RETAINED_PRINCIPAL_ID.to_owned();
                            request.username = RETAINED_PRINCIPAL_ID.to_owned();
                            request.api_key_id = None;
                            if let Some(subject) = request.quota_subject_id.as_mut()
                                && !subject.starts_with("qsub_")
                            {
                                *subject = quota_subject_for_seed(subject);
                            }
                            request.api_key_name = None;
                            request.api_key_group = None;
                            request.team_id = None;
                            request.team_name = None;
                        }
                    }
                    for ledger_id in usage_reservation_ids {
                        if let Some(reservation) = ledger.usage_reservations.get_mut(&ledger_id) {
                            reservation.user_id = RETAINED_PRINCIPAL_ID.to_owned();
                            reservation.team_id = None;
                            if let Some(subject) = reservation.quota_subject_id.as_mut()
                                && !subject.starts_with("qsub_")
                            {
                                *subject = quota_subject_for_seed(subject);
                            }
                        }
                    }
                    ledger
                        .audit_events
                        .retain(|event| !expired_audit_ids.contains(&event.id));
                }
                counts
            }
            LedgerBackend::Postgres(pool) => {
                let mut transaction = pool.begin().await?;
                let detail_cutoff = i64::try_from(detail_cutoff_ms).unwrap_or(i64::MAX);
                let usage_cutoff = i64::try_from(usage_cutoff_ms).unwrap_or(i64::MAX);
                let audit_cutoff = i64::try_from(audit_cutoff_ms).unwrap_or(i64::MAX);
                let request_details_redacted = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint FROM modelport_gateway_requests
                     WHERE state <> 'started'
                       AND created_at < to_timestamp($1::double precision / 1000.0)
                       AND request_id NOT LIKE 'retained:%'",
                )
                .bind(detail_cutoff)
                .fetch_one(&mut *transaction)
                .await?;
                let provider_attempts_redacted = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint
                     FROM modelport_provider_attempts a
                     JOIN modelport_gateway_requests r
                       ON r.ledger_id = a.request_ledger_id
                      AND r.organization_id = a.organization_id
                      AND r.project_id = a.project_id
                      AND r.environment_id = a.environment_id
                     WHERE a.state <> 'started'
                       AND r.state <> 'started'
                       AND r.created_at < to_timestamp($1::double precision / 1000.0)
                       AND r.request_id NOT LIKE 'retained:%'",
                )
                .bind(detail_cutoff)
                .fetch_one(&mut *transaction)
                .await?;
                let routing_decisions_deleted = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint
                     FROM modelport_routing_decisions d
                     JOIN modelport_gateway_requests r
                       ON r.ledger_id = d.request_ledger_id
                      AND r.organization_id = d.organization_id
                      AND r.project_id = d.project_id
                      AND r.environment_id = d.environment_id
                     WHERE r.state <> 'started'
                       AND r.created_at < to_timestamp($1::double precision / 1000.0)
                       AND r.request_id NOT LIKE 'retained:%'",
                )
                .bind(detail_cutoff)
                .fetch_one(&mut *transaction)
                .await?;
                let user_usage_rows_deidentified = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint FROM modelport_gateway_requests
                     WHERE state <> 'started'
                       AND created_at < to_timestamp($1::double precision / 1000.0)
                       AND principal_id <> $2",
                )
                .bind(usage_cutoff)
                .bind(RETAINED_PRINCIPAL_ID)
                .fetch_one(&mut *transaction)
                .await?;
                let usage_reservations_deidentified = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint FROM modelport_usage_reservations
                     WHERE state <> 'reserved'
                       AND created_at < to_timestamp($1::double precision / 1000.0)
                       AND user_id <> $2",
                )
                .bind(usage_cutoff)
                .bind(RETAINED_PRINCIPAL_ID)
                .fetch_one(&mut *transaction)
                .await?;
                let audit_events_deleted = sqlx::query_scalar::<_, i64>(
                    "SELECT count(*)::bigint FROM modelport_audit_events
                     WHERE occurred_at < to_timestamp($1::double precision / 1000.0)",
                )
                .bind(audit_cutoff)
                .fetch_one(&mut *transaction)
                .await?;
                let preview = RetentionCounts {
                    request_details_redacted: nonnegative_u64(request_details_redacted),
                    provider_attempts_redacted: nonnegative_u64(provider_attempts_redacted),
                    routing_decisions_deleted: nonnegative_u64(routing_decisions_deleted),
                    user_usage_rows_deidentified: nonnegative_u64(user_usage_rows_deidentified)
                        .saturating_add(nonnegative_u64(usage_reservations_deidentified)),
                    audit_events_deleted: nonnegative_u64(audit_events_deleted),
                };

                if !mutation_allowed {
                    transaction.rollback().await?;
                    preview
                } else {
                    let routing_decisions_deleted = sqlx::query(
                        "DELETE FROM modelport_routing_decisions d
                         USING modelport_gateway_requests r
                         WHERE r.ledger_id = d.request_ledger_id
                           AND r.organization_id = d.organization_id
                           AND r.project_id = d.project_id
                           AND r.environment_id = d.environment_id
                           AND r.state <> 'started'
                           AND r.created_at < to_timestamp($1::double precision / 1000.0)
                           AND r.request_id NOT LIKE 'retained:%'",
                    )
                    .bind(detail_cutoff)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected();
                    let provider_attempts_redacted = sqlx::query(
                        "UPDATE modelport_provider_attempts a
                         SET error_message = NULL,
                             terminal_reason = 'retained_financial_evidence',
                             lease_owner = 'retained'
                         FROM modelport_gateway_requests r
                         WHERE r.ledger_id = a.request_ledger_id
                           AND r.organization_id = a.organization_id
                           AND r.project_id = a.project_id
                           AND r.environment_id = a.environment_id
                           AND a.state <> 'started'
                           AND r.state <> 'started'
                           AND r.created_at < to_timestamp($1::double precision / 1000.0)
                           AND r.request_id NOT LIKE 'retained:%'",
                    )
                    .bind(detail_cutoff)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected();
                    let request_details_redacted = sqlx::query(
                        "UPDATE modelport_gateway_requests
                         SET request_id = 'retained:' || ledger_id,
                             client_ip = NULL,
                             idempotency_key_hash = NULL,
                             request_fingerprint = encode(
                                 sha256(convert_to($2 || ledger_id, 'UTF8')),
                                 'hex'
                             ),
                             error_message = NULL
                         WHERE state <> 'started'
                           AND created_at < to_timestamp($1::double precision / 1000.0)
                           AND request_id NOT LIKE 'retained:%'",
                    )
                    .bind(detail_cutoff)
                    .bind(RETAINED_REQUEST_FINGERPRINT_PREFIX)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected();
                    let legacy_quota_subjects = sqlx::query_scalar::<_, String>(
                        "SELECT DISTINCT quota_subject_id
                         FROM (
                            SELECT quota_subject_id
                            FROM modelport_gateway_requests
                            WHERE state <> 'started'
                              AND created_at < to_timestamp($1::double precision / 1000.0)
                              AND principal_id <> $2
                            UNION ALL
                            SELECT quota_subject_id
                            FROM modelport_usage_reservations
                            WHERE state <> 'reserved'
                              AND created_at < to_timestamp($1::double precision / 1000.0)
                              AND user_id <> $2
                         ) retained_subjects
                         WHERE quota_subject_id IS NOT NULL
                           AND left(quota_subject_id, 5) <> 'qsub_'",
                    )
                    .bind(usage_cutoff)
                    .bind(RETAINED_PRINCIPAL_ID)
                    .fetch_all(&mut *transaction)
                    .await?;
                    for subject in legacy_quota_subjects {
                        sqlx::query(
                            "UPDATE modelport_gateway_requests
                             SET quota_subject_id = $3
                             WHERE state <> 'started'
                               AND created_at < to_timestamp($1::double precision / 1000.0)
                               AND quota_subject_id = $2",
                        )
                        .bind(usage_cutoff)
                        .bind(&subject)
                        .bind(quota_subject_for_seed(&subject))
                        .execute(&mut *transaction)
                        .await?;
                        sqlx::query(
                            "UPDATE modelport_usage_reservations
                             SET quota_subject_id = $3
                             WHERE state <> 'reserved'
                               AND created_at < to_timestamp($1::double precision / 1000.0)
                               AND quota_subject_id = $2",
                        )
                        .bind(usage_cutoff)
                        .bind(&subject)
                        .bind(quota_subject_for_seed(&subject))
                        .execute(&mut *transaction)
                        .await?;
                    }
                    let user_usage_rows_deidentified = sqlx::query(
                        "UPDATE modelport_gateway_requests
                         SET principal_id = $2,
                             username = $2,
                             api_key_id = NULL,
                             api_key_name = NULL,
                             api_key_group = NULL,
                             team_id = NULL,
                             team_name = NULL
                         WHERE state <> 'started'
                           AND created_at < to_timestamp($1::double precision / 1000.0)
                           AND principal_id <> $2",
                    )
                    .bind(usage_cutoff)
                    .bind(RETAINED_PRINCIPAL_ID)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected();
                    let usage_reservations_deidentified = sqlx::query(
                        "UPDATE modelport_usage_reservations
                         SET user_id = $2,
                             team_id = NULL,
                             updated_at = now()
                         WHERE state <> 'reserved'
                           AND created_at < to_timestamp($1::double precision / 1000.0)
                           AND user_id <> $2",
                    )
                    .bind(usage_cutoff)
                    .bind(RETAINED_PRINCIPAL_ID)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected();
                    let audit_events_deleted = sqlx::query(
                        "DELETE FROM modelport_audit_events
                         WHERE occurred_at < to_timestamp($1::double precision / 1000.0)",
                    )
                    .bind(audit_cutoff)
                    .execute(&mut *transaction)
                    .await?
                    .rows_affected();
                    transaction.commit().await?;
                    RetentionCounts {
                        request_details_redacted,
                        provider_attempts_redacted,
                        routing_decisions_deleted,
                        user_usage_rows_deidentified: user_usage_rows_deidentified
                            .saturating_add(usage_reservations_deidentified),
                        audit_events_deleted,
                    }
                }
            }
        };

        Ok(RetentionResult {
            dry_run,
            applied: mutation_allowed,
            skipped_reason: policy.legal_hold.then_some("legal_hold"),
            evaluated_at_ms,
            request_detail_cutoff_ms: detail_cutoff_ms,
            user_usage_cutoff_ms: usage_cutoff_ms,
            audit_cutoff_ms,
            policy,
            counts,
            immutable_budget_events_retained: true,
        })
    }

    fn backend_name(&self) -> &'static str {
        match self.backend.as_ref() {
            LedgerBackend::Memory(_) => "memory",
            LedgerBackend::Postgres(_) => "postgres",
        }
    }

    pub(crate) fn spawn_reconciler(&self, metrics: Arc<Metrics>) {
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
                    Ok(result) => {
                        metrics.record_ledger_operation("lease_reconciliation", true);
                        metrics.record_reconciliation(result.requests, result.attempts);
                        if result.requests > 0 || result.attempts > 0 {
                            info!(
                                requests = result.requests,
                                attempts = result.attempts,
                                "reconciled expired inference ledger leases"
                            );
                        }
                    }
                    Err(err) => {
                        metrics.record_ledger_operation("lease_reconciliation", false);
                        error!(error = %err, "failed to reconcile expired ledger leases");
                    }
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

async fn ensure_tenant_catalog(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &TenantKey,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO modelport_organizations (organization_id, display_name)
         VALUES ($1, $1)
         ON CONFLICT (organization_id) DO NOTHING",
    )
    .bind(&tenant.organization_id)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO modelport_projects (organization_id, project_id, display_name)
         VALUES ($1, $2, $2)
         ON CONFLICT (organization_id, project_id) DO NOTHING",
    )
    .bind(&tenant.organization_id)
    .bind(&tenant.project_id)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO modelport_environments (
             organization_id, project_id, environment_id, display_name
         ) VALUES ($1, $2, $3, $3)
         ON CONFLICT (organization_id, project_id, environment_id) DO NOTHING",
    )
    .bind(&tenant.organization_id)
    .bind(&tenant.project_id)
    .bind(&tenant.environment_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
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
             actual_cost_microunits = $10,
             billable_cost_microunits = $11,
             pricing_evidence = $12,
             billing_mode = $13,
             chargeable = $14,
             latency_ms = $15,
             first_byte_latency_ms = $16,
             tool_outcome = $17,
             tool_repair_attempted = $18,
             tool_repair_recovered = $19,
             retry_count = $20,
             fallback_from_provider = $21,
             updated_at = now(),
             completed_at = now()
         WHERE {id_column} = $22
           AND organization_id = $23
           AND project_id = $24
           AND environment_id = $25
           AND lease_owner = $26
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
        .bind(outcome.estimate.actual_cost.map(cost_microunits))
        .bind(outcome.estimate.billable_cost.map(cost_microunits))
        .bind(&outcome.pricing_evidence)
        .bind(&outcome.billing_mode)
        .bind(outcome.chargeable)
        .bind(outcome.latency_ms)
        .bind(outcome.first_byte_latency_ms)
        .bind(&outcome.tool_outcome)
        .bind(outcome.tool_repair_attempted)
        .bind(outcome.tool_repair_recovered)
        .bind(outcome.retry_count)
        .bind(&outcome.fallback_from_provider)
        .bind(id)
        .bind(&tenant.organization_id)
        .bind(&tenant.project_id)
        .bind(&tenant.environment_id)
        .bind(lease_owner)
        .execute(&mut **transaction)
        .await?;
    Ok(result.rows_affected() == 1)
}

fn validate_usage_reservation_pricing(
    policy: &UsagePolicySnapshot,
    pricing_verified: bool,
) -> Result<(), AppError> {
    if pricing_verified || policy.user_id.is_empty() || !usage_policy_has_amount_limit(policy) {
        return Ok(());
    }
    Err(AppError::PricingUnverified(
        "API key, team, or user amount limit requires configured model pricing".to_owned(),
    ))
}

fn usage_policy_has_amount_limit(policy: &UsagePolicySnapshot) -> bool {
    let limits = &policy.api_key_policy;
    limits.spend_limit_usd > 0.0
        || limits.five_hour_limit_usd > 0.0
        || limits.daily_limit_usd > 0.0
        || limits.weekly_limit_usd > 0.0
        || limits.monthly_limit_usd > 0.0
        || limits.team_daily_limit_usd > 0.0
        || limits.team_monthly_limit_usd > 0.0
        || policy.quotas.iter().any(|quota| quota.quota_type == "cost")
}

fn usage_policy_has_hard_limit(policy: &UsagePolicySnapshot) -> bool {
    usage_policy_has_amount_limit(policy) || !policy.quotas.is_empty()
}

fn effective_quota_subject(policy: &UsagePolicySnapshot) -> Option<&str> {
    policy
        .quota_subject_id
        .as_deref()
        .or(policy.api_key_id.as_deref())
}

fn effective_quota_subject_aliases(policy: &UsagePolicySnapshot) -> Vec<String> {
    let mut aliases = policy.quota_subject_aliases.clone();
    if let Some(subject) = effective_quota_subject(policy) {
        aliases.push(subject.to_owned());
    }
    if let Some(api_key_id) = policy.api_key_id.as_deref() {
        aliases.push(api_key_id.to_owned());
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn enforce_usage_spend_limits(
    policy: &UsagePolicySnapshot,
    spend: UsageSpendTotals,
    incoming_cost_microunits: i64,
) -> Result<(), AppError> {
    let incoming = microunits_usd(incoming_cost_microunits);
    let limits = &policy.api_key_policy;
    enforce_spend_limit(
        "total spend",
        limits.spend_limit_usd,
        spend.api_key_all_time,
        incoming,
    )?;
    if limits.rate_limited {
        for (label, limit, used) in [
            (
                "5 hour spend",
                limits.five_hour_limit_usd,
                spend.api_key_five_hours,
            ),
            ("daily spend", limits.daily_limit_usd, spend.api_key_day),
            ("7 day spend", limits.weekly_limit_usd, spend.api_key_week),
            (
                "monthly spend",
                limits.monthly_limit_usd,
                spend.api_key_month,
            ),
        ] {
            enforce_spend_limit(label, limit, used, incoming)?;
        }
    }
    enforce_spend_limit(
        "team daily spend",
        limits.team_daily_limit_usd,
        spend.team_day,
        incoming,
    )?;
    enforce_spend_limit(
        "team monthly spend",
        limits.team_monthly_limit_usd,
        spend.team_month,
        incoming,
    )
}

fn check_memory_usage_admission(
    ledger: &MemoryLedger,
    request_ledger_id: &str,
    policy: &UsagePolicySnapshot,
    estimate: UsageEstimate,
) -> Result<UsageReservationIncrement, AppError> {
    if policy.user_id.is_empty() || !usage_policy_has_hard_limit(policy) {
        return Ok(UsageReservationIncrement::for_attempt(estimate, false));
    }
    let existing = ledger.usage_reservations.get(request_ledger_id);
    if let Some(existing) = existing
        && (existing.state != "reserved"
            || existing.quota_subject_id.as_deref() != effective_quota_subject(policy)
            || existing.team_id != policy.team_id
            || existing.user_id != policy.user_id)
    {
        return Err(AppError::Database(
            "usage reservation scope changed during a logical request".to_owned(),
        ));
    }
    let increment = UsageReservationIncrement::for_attempt(estimate, existing.is_none());
    enforce_usage_spend_limits(
        policy,
        memory_usage_spend_totals(ledger, policy, true),
        increment.cost_microunits,
    )?;
    for quota in &policy.quotas {
        let (requests, tokens, cost_microunits) = memory_quota_totals(ledger, quota, true);
        let used = quota_value_from_totals(&quota.quota_type, requests, tokens, cost_microunits);
        let incoming = increment.quota_value(&quota.quota_type);
        if incoming > 0.0 && used + incoming > quota.limit {
            return Err(AppError::QuotaExceeded(format!(
                "{} quota exceeded for user {}",
                quota.quota_type, policy.username
            )));
        }
    }
    Ok(increment)
}

fn reserve_memory_usage(
    ledger: &mut MemoryLedger,
    request_ledger_id: &str,
    policy: &UsagePolicySnapshot,
    increment: UsageReservationIncrement,
    now: i64,
) {
    if policy.user_id.is_empty() || !usage_policy_has_hard_limit(policy) {
        return;
    }
    if let Some(reservation) = ledger.usage_reservations.get_mut(request_ledger_id) {
        reservation.reserved_tokens = reservation.reserved_tokens.saturating_add(increment.tokens);
        reservation.reserved_cost_microunits = reservation
            .reserved_cost_microunits
            .saturating_add(increment.cost_microunits);
        reservation.updated_at_ms = now;
        return;
    }
    ledger.usage_reservations.insert(
        request_ledger_id.to_owned(),
        MemoryUsageReservation {
            reservation_id: format!("urs_{}", Uuid::new_v4().simple()),
            quota_subject_id: effective_quota_subject(policy).map(str::to_owned),
            team_id: policy.team_id.clone(),
            user_id: policy.user_id.clone(),
            reserved_requests: increment.requests,
            reserved_tokens: increment.tokens,
            reserved_cost_microunits: increment.cost_microunits,
            actual_requests: 0,
            actual_tokens: 0,
            actual_cost_microunits: 0,
            state: "reserved".to_owned(),
            evidence_source: None,
            billing_mode: None,
            created_at_ms: now,
            updated_at_ms: now,
            terminal_at_ms: None,
        },
    );
}

fn memory_usage_spend_totals(
    ledger: &MemoryLedger,
    policy: &UsagePolicySnapshot,
    include_reservations: bool,
) -> UsageSpendTotals {
    let aliases = effective_quota_subject_aliases(policy);
    let now = now_millis();
    let mut totals = UsageSpendTotals::default();
    for request in ledger
        .requests
        .values()
        .filter(|request| request.record.terminal && request.record.chargeable)
    {
        add_scoped_spend(
            &mut totals,
            request
                .quota_subject_id
                .as_deref()
                .or(request.api_key_id.as_deref()),
            request.team_id.as_deref(),
            request.record.created_at_ms,
            request.record.cost_amount_microunits,
            policy,
            &aliases,
            now,
        );
    }
    if include_reservations {
        for reservation in ledger
            .usage_reservations
            .values()
            .filter(|reservation| reservation.state == "reserved")
        {
            add_scoped_spend(
                &mut totals,
                reservation.quota_subject_id.as_deref(),
                reservation.team_id.as_deref(),
                reservation.created_at_ms,
                reservation.reserved_cost_microunits,
                policy,
                &aliases,
                now,
            );
        }
    }
    totals
}

#[allow(clippy::too_many_arguments)]
fn add_scoped_spend(
    totals: &mut UsageSpendTotals,
    quota_subject_id: Option<&str>,
    team_id: Option<&str>,
    created_at_ms: i64,
    cost_microunits: i64,
    policy: &UsagePolicySnapshot,
    aliases: &[String],
    now: i64,
) {
    let cost = microunits_usd(cost_microunits);
    let age = now.saturating_sub(created_at_ms);
    if quota_subject_id.is_some_and(|subject| aliases.iter().any(|alias| alias == subject)) {
        totals.api_key_all_time += cost;
        if age <= 5 * 60 * 60 * 1_000 {
            totals.api_key_five_hours += cost;
        }
        if age <= 24 * 60 * 60 * 1_000 {
            totals.api_key_day += cost;
        }
        if age <= 7 * 24 * 60 * 60 * 1_000 {
            totals.api_key_week += cost;
        }
        if age <= 30 * 24 * 60 * 60 * 1_000 {
            totals.api_key_month += cost;
        }
    }
    if team_id == policy.team_id.as_deref() && team_id.is_some() {
        if age <= 24 * 60 * 60 * 1_000 {
            totals.team_day += cost;
        }
        if age <= 30 * 24 * 60 * 60 * 1_000 {
            totals.team_month += cost;
        }
    }
}

fn memory_quota_totals(
    ledger: &MemoryLedger,
    quota: &UsageQuotaLimit,
    include_reservations: bool,
) -> (u64, u64, i64) {
    let period_start = i64::try_from(quota.period_start_ms).unwrap_or(i64::MAX);
    let mut requests = 0u64;
    let mut tokens = 0u64;
    let mut cost_microunits = 0i64;
    for request in ledger.requests.values().filter(|request| {
        request.record.terminal
            && request.record.chargeable
            && request.principal_id == quota.user_id
            && request.record.created_at_ms >= period_start
    }) {
        requests = requests.saturating_add(1);
        tokens = tokens.saturating_add(request_total_tokens(&request.record));
        cost_microunits =
            cost_microunits.saturating_add(request.record.cost_amount_microunits.max(0));
    }
    if include_reservations {
        for reservation in ledger.usage_reservations.values().filter(|reservation| {
            reservation.state == "reserved"
                && reservation.user_id == quota.user_id
                && reservation.created_at_ms >= period_start
        }) {
            requests = requests.saturating_add(reservation.reserved_requests);
            tokens = tokens.saturating_add(reservation.reserved_tokens);
            cost_microunits =
                cost_microunits.saturating_add(reservation.reserved_cost_microunits.max(0));
        }
    }
    (requests, tokens, cost_microunits)
}

async fn reserve_usage_capacity_pg(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    request: &LedgerRequest,
    policy: &UsagePolicySnapshot,
    estimate: UsageEstimate,
) -> Result<(), AppError> {
    if policy.user_id.is_empty() || !usage_policy_has_hard_limit(policy) {
        return Ok(());
    }
    lock_usage_scopes_pg(transaction, policy).await?;
    let existing = sqlx::query_as::<_, (String, Option<String>, Option<String>, String)>(
        "SELECT state, quota_subject_id, team_id, user_id
         FROM modelport_usage_reservations
         WHERE organization_id = $1
           AND project_id = $2
           AND environment_id = $3
           AND request_ledger_id = $4
         FOR UPDATE",
    )
    .bind(&request.tenant.organization_id)
    .bind(&request.tenant.project_id)
    .bind(&request.tenant.environment_id)
    .bind(&request.ledger_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some((state, subject, team_id, user_id)) = &existing
        && (state != "reserved"
            || subject.as_deref() != effective_quota_subject(policy)
            || team_id != &policy.team_id
            || user_id != &policy.user_id)
    {
        return Err(AppError::Database(
            "usage reservation scope changed during a logical request".to_owned(),
        ));
    }
    let increment = UsageReservationIncrement::for_attempt(estimate, existing.is_none());
    let spend = usage_spend_totals_pg_tx(transaction, policy).await?;
    enforce_usage_spend_limits(policy, spend, increment.cost_microunits)?;
    for quota in &policy.quotas {
        let (requests, tokens, cost_microunits) = quota_totals_pg_tx(transaction, quota).await?;
        let used = quota_value_from_totals(&quota.quota_type, requests, tokens, cost_microunits);
        let incoming = increment.quota_value(&quota.quota_type);
        if incoming > 0.0 && used + incoming > quota.limit {
            return Err(AppError::QuotaExceeded(format!(
                "{} quota exceeded for user {}",
                quota.quota_type, policy.username
            )));
        }
    }
    if existing.is_some() {
        let updated = sqlx::query(
            "UPDATE modelport_usage_reservations
             SET reserved_tokens = reserved_tokens + $1,
                 reserved_cost_microunits = reserved_cost_microunits + $2,
                 updated_at = now()
             WHERE organization_id = $3
               AND project_id = $4
               AND environment_id = $5
               AND request_ledger_id = $6
               AND state = 'reserved'",
        )
        .bind(to_i64(increment.tokens))
        .bind(increment.cost_microunits)
        .bind(&request.tenant.organization_id)
        .bind(&request.tenant.project_id)
        .bind(&request.tenant.environment_id)
        .bind(&request.ledger_id)
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(AppError::Database(
                "open usage reservation disappeared during admission".to_owned(),
            ));
        }
    } else {
        sqlx::query(
            "INSERT INTO modelport_usage_reservations (
                reservation_id,
                organization_id, project_id, environment_id,
                request_ledger_id, quota_subject_id, team_id, user_id,
                reserved_requests, reserved_tokens, reserved_cost_microunits
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(format!("urs_{}", Uuid::new_v4().simple()))
        .bind(&request.tenant.organization_id)
        .bind(&request.tenant.project_id)
        .bind(&request.tenant.environment_id)
        .bind(&request.ledger_id)
        .bind(effective_quota_subject(policy))
        .bind(policy.team_id.as_deref())
        .bind(&policy.user_id)
        .bind(to_i64(increment.requests))
        .bind(to_i64(increment.tokens))
        .bind(increment.cost_microunits)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn lock_usage_scopes_pg(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    policy: &UsagePolicySnapshot,
) -> Result<(), AppError> {
    let mut scopes = vec![format!("usage:user:{}", policy.user_id)];
    if let Some(subject) = effective_quota_subject(policy) {
        scopes.push(format!("usage:subject:{subject}"));
    }
    if let Some(team_id) = policy.team_id.as_deref() {
        scopes.push(format!("usage:team:{team_id}"));
    }
    scopes.sort();
    scopes.dedup();
    for scope in scopes {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(scope)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn usage_spend_totals_pg_tx(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    policy: &UsagePolicySnapshot,
) -> Result<UsageSpendTotals, AppError> {
    let aliases = effective_quota_subject_aliases(policy);
    let row = sqlx::query(
        "SELECT
            COALESCE(sum(cost_microunits)
                FILTER (WHERE quota_subject_id = ANY($1::text[])), 0)::bigint
                AS api_key_all_time,
            COALESCE(sum(cost_microunits)
                FILTER (WHERE quota_subject_id = ANY($1::text[])
                    AND created_at >= now() - interval '5 hours'), 0)::bigint
                AS api_key_five_hours,
            COALESCE(sum(cost_microunits)
                FILTER (WHERE quota_subject_id = ANY($1::text[])
                    AND created_at >= now() - interval '1 day'), 0)::bigint
                AS api_key_day,
            COALESCE(sum(cost_microunits)
                FILTER (WHERE quota_subject_id = ANY($1::text[])
                    AND created_at >= now() - interval '7 days'), 0)::bigint
                AS api_key_week,
            COALESCE(sum(cost_microunits)
                FILTER (WHERE quota_subject_id = ANY($1::text[])
                    AND created_at >= now() - interval '30 days'), 0)::bigint
                AS api_key_month,
            COALESCE(sum(cost_microunits)
                FILTER (WHERE team_id = $2
                    AND created_at >= now() - interval '1 day'), 0)::bigint
                AS team_day,
            COALESCE(sum(cost_microunits)
                FILTER (WHERE team_id = $2
                    AND created_at >= now() - interval '30 days'), 0)::bigint
                AS team_month
         FROM (
            SELECT quota_subject_id, team_id, created_at,
                   COALESCE(billable_cost_microunits, 0) AS cost_microunits
            FROM modelport_gateway_requests
            WHERE state <> 'started' AND chargeable
            UNION ALL
            SELECT quota_subject_id, team_id, created_at,
                   reserved_cost_microunits AS cost_microunits
            FROM modelport_usage_reservations
            WHERE state = 'reserved'
         ) usage
         WHERE ((cardinality($1::text[]) > 0
                    AND quota_subject_id = ANY($1::text[]))
             OR ($2::text IS NOT NULL AND team_id = $2))",
    )
    .bind(&aliases)
    .bind(policy.team_id.as_deref())
    .fetch_one(&mut **transaction)
    .await?;
    Ok(UsageSpendTotals {
        api_key_all_time: microunits_usd(row.try_get("api_key_all_time")?),
        api_key_five_hours: microunits_usd(row.try_get("api_key_five_hours")?),
        api_key_day: microunits_usd(row.try_get("api_key_day")?),
        api_key_week: microunits_usd(row.try_get("api_key_week")?),
        api_key_month: microunits_usd(row.try_get("api_key_month")?),
        team_day: microunits_usd(row.try_get("team_day")?),
        team_month: microunits_usd(row.try_get("team_month")?),
    })
}

async fn quota_totals_pg_tx(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    quota: &UsageQuotaLimit,
) -> Result<(u64, u64, i64), AppError> {
    let period_start = i64::try_from(quota.period_start_ms).unwrap_or(i64::MAX);
    let row = sqlx::query(
        "SELECT
            (SELECT count(*)::bigint
             FROM modelport_gateway_requests
             WHERE principal_id = $1
               AND state <> 'started'
               AND chargeable
               AND created_at >= to_timestamp($2::double precision / 1000.0))
                + COALESCE((SELECT sum(reserved_requests)::bigint
                    FROM modelport_usage_reservations
                    WHERE user_id = $1
                      AND state = 'reserved'
                      AND created_at >= to_timestamp($2::double precision / 1000.0)), 0)
                AS requests,
            COALESCE((SELECT sum(
                    input_tokens + output_tokens
                    + cache_write_tokens + cache_read_tokens
                )::bigint
                FROM modelport_gateway_requests
                WHERE principal_id = $1
                  AND state <> 'started'
                  AND chargeable
                  AND created_at >= to_timestamp($2::double precision / 1000.0)), 0)
                + COALESCE((SELECT sum(reserved_tokens)::bigint
                    FROM modelport_usage_reservations
                    WHERE user_id = $1
                      AND state = 'reserved'
                      AND created_at >= to_timestamp($2::double precision / 1000.0)), 0)
                AS tokens,
            COALESCE((SELECT sum(billable_cost_microunits)::bigint
                FROM modelport_gateway_requests
                WHERE principal_id = $1
                  AND state <> 'started'
                  AND chargeable
                  AND created_at >= to_timestamp($2::double precision / 1000.0)), 0)
                + COALESCE((SELECT sum(reserved_cost_microunits)::bigint
                    FROM modelport_usage_reservations
                    WHERE user_id = $1
                      AND state = 'reserved'
                      AND created_at >= to_timestamp($2::double precision / 1000.0)), 0)
                AS cost_microunits",
    )
    .bind(&quota.user_id)
    .bind(period_start)
    .fetch_one(&mut **transaction)
    .await?;
    Ok((
        nonnegative_u64(row.try_get("requests")?),
        nonnegative_u64(row.try_get("tokens")?),
        row.try_get("cost_microunits")?,
    ))
}

fn settle_memory_usage(
    ledger: &mut MemoryLedger,
    request_ledger_id: &str,
    outcome: &LedgerOutcome,
) {
    let Some(reservation) = ledger.usage_reservations.get_mut(request_ledger_id) else {
        return;
    };
    if reservation.state != "reserved" {
        return;
    }
    let now = now_millis();
    reservation.updated_at_ms = now;
    reservation.terminal_at_ms = Some(now);
    reservation.billing_mode = Some(outcome.billing_mode.clone());
    reservation.evidence_source = Some(outcome_evidence_source(outcome).to_owned());
    if outcome.chargeable {
        reservation.state = "settled".to_owned();
        reservation.actual_requests = 1;
        reservation.actual_tokens = estimate_total_tokens(outcome.estimate);
        reservation.actual_cost_microunits =
            outcome.estimate.billable_cost.map_or(0, cost_microunits);
    } else {
        reservation.state = "released".to_owned();
    }
}

fn release_memory_usage(ledger: &mut MemoryLedger, request_ledger_id: &str) {
    let Some(reservation) = ledger.usage_reservations.get_mut(request_ledger_id) else {
        return;
    };
    if reservation.state != "reserved" {
        return;
    }
    let now = now_millis();
    reservation.state = "released".to_owned();
    reservation.evidence_source = Some("lease-expired".to_owned());
    reservation.billing_mode = Some("unreconciled".to_owned());
    reservation.updated_at_ms = now;
    reservation.terminal_at_ms = Some(now);
}

async fn settle_usage_reservation_pg(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    request: &LedgerRequest,
    outcome: &LedgerOutcome,
) -> Result<(), AppError> {
    settle_usage_reservation_by_id_pg(transaction, &request.ledger_id, &request.tenant, outcome)
        .await
}

async fn settle_usage_reservation_by_id_pg(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    request_ledger_id: &str,
    tenant: &TenantKey,
    outcome: &LedgerOutcome,
) -> Result<(), AppError> {
    let (state, actual_requests, actual_tokens, actual_cost_microunits) = if outcome.chargeable {
        (
            "settled",
            1i64,
            to_i64(estimate_total_tokens(outcome.estimate)),
            outcome.estimate.billable_cost.map_or(0, cost_microunits),
        )
    } else {
        ("released", 0, 0, 0)
    };
    sqlx::query(
        "UPDATE modelport_usage_reservations
         SET state = $1,
             actual_requests = $2,
             actual_tokens = $3,
             actual_cost_microunits = $4,
             evidence_source = $5,
             billing_mode = $6,
             updated_at = now(),
             terminal_at = now()
         WHERE organization_id = $7
           AND project_id = $8
           AND environment_id = $9
           AND request_ledger_id = $10
           AND state = 'reserved'",
    )
    .bind(state)
    .bind(actual_requests)
    .bind(actual_tokens)
    .bind(actual_cost_microunits)
    .bind(outcome_evidence_source(outcome))
    .bind(&outcome.billing_mode)
    .bind(&tenant.organization_id)
    .bind(&tenant.project_id)
    .bind(&tenant.environment_id)
    .bind(request_ledger_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn release_usage_reservation_pg(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    request_ledger_id: &str,
    tenant: &TenantKey,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE modelport_usage_reservations
         SET state = 'released',
             evidence_source = 'lease-expired',
             billing_mode = 'unreconciled',
             updated_at = now(),
             terminal_at = now()
         WHERE organization_id = $1
           AND project_id = $2
           AND environment_id = $3
           AND request_ledger_id = $4
           AND state = 'reserved'",
    )
    .bind(&tenant.organization_id)
    .bind(&tenant.project_id)
    .bind(&tenant.environment_id)
    .bind(request_ledger_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn settle_memory_budget(
    ledger: &mut MemoryLedger,
    attempt: &LedgerAttempt,
    outcome: &LedgerOutcome,
) -> Result<(), AppError> {
    let settled_microunits = cost_microunits(
        outcome
            .estimate
            .billable_cost
            .expect("settlement requires billable cost"),
    );
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

fn release_memory_budget(
    ledger: &mut MemoryLedger,
    attempt_id: &str,
    evidence_source: &str,
    billing_mode: &str,
    reason: &str,
) -> Result<(), AppError> {
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
        evidence_source,
        Some(billing_mode),
        Some(reason),
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
    let settled_microunits = cost_microunits(
        outcome
            .estimate
            .billable_cost
            .expect("settlement requires billable cost"),
    );
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
    evidence_source: &str,
    billing_mode: &str,
    reason: &str,
) -> Result<(), AppError> {
    let reservation = sqlx::query_as::<_, (String, String, i64)>(
        "UPDATE modelport_budget_reservations
         SET state = 'released',
             evidence_source = $5,
             billing_mode = $6,
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
    .bind(evidence_source)
    .bind(billing_mode)
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
        evidence_source,
        Some(billing_mode),
        Some(reason),
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
    match outcome
        .pricing_evidence
        .as_ref()
        .and_then(|value| value.get("method"))
        .and_then(Value::as_str)
    {
        Some("provider_reported") => "provider-reported-cost",
        Some("exact_rate_card") => "verified-rate-card",
        _ if outcome.billing_mode == "upstream-returned" => "provider-usage-unpriced",
        _ => "local-estimate",
    }
}

fn validate_billing_outcome(outcome: &LedgerOutcome) -> Result<(), AppError> {
    for value in [outcome.estimate.actual_cost, outcome.estimate.billable_cost]
        .into_iter()
        .flatten()
    {
        if !value.is_finite() || value < 0.0 {
            return Err(AppError::Database(
                "billing outcome contains an invalid reconciled amount".to_owned(),
            ));
        }
    }
    if outcome.estimate.billable_cost.is_some() && outcome.pricing_evidence.is_none() {
        return Err(AppError::Database(
            "billable outcome requires pricing evidence".to_owned(),
        ));
    }
    Ok(())
}

fn push_operational_log_filters<'args>(
    query_builder: &mut QueryBuilder<'args, Postgres>,
    query: &'args OperationalLogQuery,
) {
    query_builder.push(" WHERE r.state <> 'started'");
    if let Some(status) = query.status.as_deref() {
        match status {
            "success" => {
                query_builder.push(" AND r.state = 'completed'");
            }
            "timeout" => {
                query_builder.push(
                    " AND r.state <> 'completed'
                      AND COALESCE(r.terminal_reason, '') ILIKE '%timeout%'",
                );
            }
            "error" => {
                query_builder.push(
                    " AND r.state <> 'completed'
                      AND COALESCE(r.terminal_reason, '') NOT ILIKE '%timeout%'",
                );
            }
            _ => {}
        }
    }
    if let Some(provider) = query.provider.as_deref() {
        query_builder
            .push(" AND COALESCE(r.provider_id, 'unrouted') = ")
            .push_bind(provider);
    }
    if let Some(model) = query.model.as_deref() {
        let pattern = format!("%{}%", model.trim());
        query_builder
            .push(" AND (r.requested_model ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.resolved_model, '') ILIKE ")
            .push_bind(pattern)
            .push(")");
    }
    if let Some(user_id) = query.user_id.as_deref() {
        query_builder
            .push(" AND r.principal_id = ")
            .push_bind(user_id);
    }
    if let Some(api_key_id) = query.api_key_id.as_deref() {
        query_builder
            .push(" AND r.api_key_id = ")
            .push_bind(api_key_id);
    }
    if let Some(date_from) = query.date_from {
        query_builder
            .push(" AND r.created_at >= to_timestamp(")
            .push_bind(i64::try_from(date_from).unwrap_or(i64::MAX))
            .push("::double precision / 1000.0)");
    }
    if let Some(date_to) = query.date_to {
        query_builder
            .push(" AND r.created_at <= to_timestamp(")
            .push_bind(i64::try_from(date_to).unwrap_or(i64::MAX))
            .push("::double precision / 1000.0)");
    }
    if let Some(username) = query.username.as_deref() {
        query_builder
            .push(" AND r.username ILIKE ")
            .push_bind(format!("%{}%", username.trim()));
    }
    if let Some(group) = query.group.as_deref() {
        query_builder
            .push(" AND COALESCE(r.api_key_group, '') ILIKE ")
            .push_bind(format!("%{}%", group.trim()));
    }
    if let Some(stream) = query.stream {
        query_builder.push(" AND r.stream = ").push_bind(stream);
    }
    if let Some(tool_use_requested) = query.tool_use_requested {
        query_builder
            .push(" AND r.tool_use_requested = ")
            .push_bind(tool_use_requested);
    }
    if let Some(traffic_class) = query.traffic_class.as_deref() {
        query_builder
            .push(" AND r.traffic_class = ")
            .push_bind(traffic_class);
    }
    if let Some(search) = query.search.as_deref() {
        let pattern = format!("%{}%", search.trim());
        query_builder
            .push(" AND (r.ledger_id ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.request_id ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.last_attempt_id, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.provider_id, 'unrouted') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.requested_model ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.resolved_model, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.principal_id ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.username ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.api_key_id, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.api_key_name, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.api_key_group, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.team_id, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.team_name, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.error_message, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.terminal_reason, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.request_path ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.client_protocol ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR COALESCE(r.provider_protocol, '') ILIKE ")
            .push_bind(pattern.clone())
            .push(" OR r.traffic_class ILIKE ")
            .push_bind(pattern)
            .push(")");
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
        AND ($7::text IS NULL OR r.traffic_class = $7)
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
        r.principal_id, r.username,
        r.api_key_id, r.api_key_name, r.api_key_group,
        r.team_id, r.team_name, host(r.client_ip) AS client_ip,
        r.client_protocol, r.requested_model, r.stream,
        r.request_path, r.traffic_class, r.tool_use_requested,
        r.provider_id, r.resolved_model, r.provider_protocol,
        r.last_attempt_id, r.model_pricing,
        r.state, r.status_code, r.terminal_reason, r.error_message,
        r.input_tokens, r.output_tokens, r.cache_write_tokens, r.cache_read_tokens,
        r.cost_amount_microunits, r.actual_cost_microunits,
        r.billable_cost_microunits, r.pricing_evidence,
        r.currency, r.billing_mode, r.chargeable,
        r.latency_ms, r.first_byte_latency_ms,
        r.tool_outcome, r.tool_repair_attempted, r.tool_repair_recovered,
        r.retry_count, r.fallback_from_provider,
        d.decision_id AS routing_decision_id,
        d.route_group_id AS routing_group_id,
        d.routing_profile,
        d.routing_mode,
        d.policy_version AS routing_policy_version,
        d.selected_provider_id AS routing_selected_provider,
        d.selected_model AS routing_selected_model,
        d.recommended_provider_id AS routing_recommended_provider,
        d.recommended_model AS routing_recommended_model,
        d.candidate_count AS routing_candidate_count,
        d.selected_score AS routing_selected_score,
        d.recommended_score AS routing_recommended_score,
        d.reason_codes AS routing_reason_codes,
        d.session_affinity AS routing_session_affinity,
        d.shadow_disagreement AS routing_shadow_disagreement,
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
    LEFT JOIN modelport_routing_decisions d
      ON d.request_ledger_id = r.ledger_id
     AND d.organization_id = r.organization_id
     AND d.project_id = r.project_id
     AND d.environment_id = r.environment_id
    WHERE
        ($1::text IS NULL OR r.state = $1)
        AND ($2::text IS NULL OR r.client_protocol = $2)
        AND ($3::text IS NULL OR r.organization_id = $3)
        AND ($4::text IS NULL OR r.project_id = $4)
        AND ($5::text IS NULL OR r.environment_id = $5)
        AND ($7::text IS NULL OR r.traffic_class = $7)
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
        AND (
            $10::bigint IS NULL
            OR r.created_at >= to_timestamp($10::double precision / 1000.0)
        )
    ORDER BY r.created_at DESC, r.ledger_id DESC
    LIMIT $8 OFFSET $9";

const OPERATIONAL_LOG_SELECT_SQL: &str = "SELECT
        r.ledger_id, r.request_id,
        r.organization_id, r.project_id, r.environment_id,
        r.principal_id, r.username,
        r.api_key_id, r.api_key_name, r.api_key_group,
        r.team_id, r.team_name, host(r.client_ip) AS client_ip,
        r.client_protocol, r.requested_model, r.stream,
        r.request_path, r.traffic_class, r.tool_use_requested,
        r.provider_id, r.resolved_model, r.provider_protocol,
        r.last_attempt_id, r.model_pricing,
        r.state, r.status_code, r.terminal_reason, r.error_message,
        r.input_tokens, r.output_tokens, r.cache_write_tokens, r.cache_read_tokens,
        r.cost_amount_microunits, r.actual_cost_microunits,
        r.billable_cost_microunits, r.pricing_evidence,
        r.currency, r.billing_mode, r.chargeable,
        r.latency_ms, r.first_byte_latency_ms,
        r.tool_outcome, r.tool_repair_attempted, r.tool_repair_recovered,
        r.retry_count, r.fallback_from_provider,
        d.decision_id AS routing_decision_id,
        d.route_group_id AS routing_group_id,
        d.routing_profile,
        d.routing_mode,
        d.policy_version AS routing_policy_version,
        d.selected_provider_id AS routing_selected_provider,
        d.selected_model AS routing_selected_model,
        d.recommended_provider_id AS routing_recommended_provider,
        d.recommended_model AS routing_recommended_model,
        d.candidate_count AS routing_candidate_count,
        d.selected_score AS routing_selected_score,
        d.recommended_score AS routing_recommended_score,
        d.reason_codes AS routing_reason_codes,
        d.session_affinity AS routing_session_affinity,
        d.shadow_disagreement AS routing_shadow_disagreement,
        (r.idempotency_key_hash IS NOT NULL) AS has_idempotency_key,
        r.lease_owner,
        (EXTRACT(EPOCH FROM r.lease_expires_at) * 1000)::bigint AS lease_expires_at_ms,
        (EXTRACT(EPOCH FROM r.created_at) * 1000)::bigint AS created_at_ms,
        (EXTRACT(EPOCH FROM r.updated_at) * 1000)::bigint AS updated_at_ms,
        (EXTRACT(EPOCH FROM r.completed_at) * 1000)::bigint AS completed_at_ms,
        0::bigint AS attempt_count
    FROM modelport_gateway_requests r
    LEFT JOIN modelport_routing_decisions d
      ON d.request_ledger_id = r.ledger_id
     AND d.organization_id = r.organization_id
     AND d.project_id = r.project_id
     AND d.environment_id = r.environment_id";

const REQUEST_DETAIL_SQL: &str = "SELECT
        r.ledger_id, r.request_id,
        r.organization_id, r.project_id, r.environment_id,
        r.principal_id, r.username,
        r.api_key_id, r.api_key_name, r.api_key_group,
        r.team_id, r.team_name, host(r.client_ip) AS client_ip,
        r.client_protocol, r.requested_model, r.stream,
        r.request_path, r.traffic_class, r.tool_use_requested,
        r.provider_id, r.resolved_model, r.provider_protocol,
        r.last_attempt_id, r.model_pricing,
        r.state, r.status_code, r.terminal_reason, r.error_message,
        r.input_tokens, r.output_tokens, r.cache_write_tokens, r.cache_read_tokens,
        r.cost_amount_microunits, r.actual_cost_microunits,
        r.billable_cost_microunits, r.pricing_evidence,
        r.currency, r.billing_mode, r.chargeable,
        r.latency_ms, r.first_byte_latency_ms,
        r.tool_outcome, r.tool_repair_attempted, r.tool_repair_recovered,
        r.retry_count, r.fallback_from_provider,
        d.decision_id AS routing_decision_id,
        d.route_group_id AS routing_group_id,
        d.routing_profile,
        d.routing_mode,
        d.policy_version AS routing_policy_version,
        d.selected_provider_id AS routing_selected_provider,
        d.selected_model AS routing_selected_model,
        d.recommended_provider_id AS routing_recommended_provider,
        d.recommended_model AS routing_recommended_model,
        d.candidate_count AS routing_candidate_count,
        d.selected_score AS routing_selected_score,
        d.recommended_score AS routing_recommended_score,
        d.reason_codes AS routing_reason_codes,
        d.session_affinity AS routing_session_affinity,
        d.shadow_disagreement AS routing_shadow_disagreement,
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
    LEFT JOIN modelport_routing_decisions d
      ON d.request_ledger_id = r.ledger_id
     AND d.organization_id = r.organization_id
     AND d.project_id = r.project_id
     AND d.environment_id = r.environment_id
    WHERE r.ledger_id = $1";

const ATTEMPT_LIST_SQL: &str = "SELECT
        attempt_id, request_ledger_id,
        organization_id, project_id, environment_id,
        provider_id, resolved_model, provider_protocol,
        state, status_code, terminal_reason, error_message,
        input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
        cost_amount_microunits, actual_cost_microunits,
        billable_cost_microunits, pricing_evidence,
        currency, billing_mode, chargeable,
        latency_ms, first_byte_latency_ms,
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
    let utilization_basis_points = limit_microunits.map(|limit| utilization_bps(consumed, limit));
    EnterpriseBudgetAccount {
        organization_id: tenant.organization_id.clone(),
        project_id: tenant.project_id.clone(),
        environment_id: tenant.environment_id.clone(),
        currency: "USD".to_owned(),
        limit_microunits,
        reserved_microunits,
        settled_microunits,
        available_microunits: limit_microunits.map(|limit| limit.saturating_sub(consumed)),
        utilization_basis_points,
        warning_threshold_reached: utilization_basis_points.is_some_and(|value| value >= 8_000),
        hard_limit_reached: utilization_basis_points.is_some_and(|value| value >= 10_000),
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
    traffic_class: Option<String>,
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
        let traffic_class = normalized_filter(self.traffic_class.as_deref(), "trafficClass", 32)?;
        if traffic_class
            .as_deref()
            .is_some_and(|value| !matches!(value, "business" | "synthetic" | "diagnostic"))
        {
            return Err(AppError::InvalidRequest(
                "enterprise ledger trafficClass must be business, synthetic, or diagnostic"
                    .to_owned(),
            ));
        }
        Ok(NormalizedLedgerQuery {
            page,
            page_size,
            state,
            protocol,
            traffic_class,
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
                .traffic_class
                .as_deref()
                .is_some_and(|value| request.traffic_class != value)
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

fn validate_request_metadata(metadata: &LedgerRequestMetadata) -> Result<(), AppError> {
    validate_metadata_text("username", Some(&metadata.username), true)?;
    for (field, value) in [
        ("apiKeyId", metadata.api_key_id.as_deref()),
        ("apiKeyName", metadata.api_key_name.as_deref()),
        ("apiKeyGroup", metadata.api_key_group.as_deref()),
        ("teamId", metadata.team_id.as_deref()),
        ("teamName", metadata.team_name.as_deref()),
    ] {
        validate_metadata_text(field, value, false)?;
    }
    if let Some(client_ip) = metadata.client_ip.as_deref()
        && client_ip.parse::<IpAddr>().is_err()
    {
        return Err(AppError::InvalidRequest(
            "client IP metadata must be an IPv4 or IPv6 address".to_owned(),
        ));
    }
    if let Some(decision) = &metadata.routing_decision {
        for (field, value, max_len) in [
            ("decisionId", decision.decision_id.as_str(), 80),
            ("routingProfile", decision.profile.as_str(), 32),
            ("routingMode", decision.mode.as_str(), 32),
            ("policyVersion", decision.policy_version.as_str(), 64),
            ("selectedProvider", decision.selected_provider.as_str(), 80),
            ("selectedModel", decision.selected_model.as_str(), 240),
            (
                "recommendedProvider",
                decision.recommended_provider.as_str(),
                80,
            ),
            ("recommendedModel", decision.recommended_model.as_str(), 240),
        ] {
            if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
                return Err(AppError::InvalidRequest(format!(
                    "routing {field} must contain 1-{max_len} non-control bytes"
                )));
            }
        }
        if decision.group_id.as_deref().is_some_and(|value| {
            value.is_empty() || value.len() > 80 || value.chars().any(char::is_control)
        }) || !(1..=256).contains(&decision.candidate_count)
            || !decision.selected_score.is_finite()
            || !(0.0..=2.0).contains(&decision.selected_score)
            || !decision.recommended_score.is_finite()
            || !(0.0..=2.0).contains(&decision.recommended_score)
            || decision.reason_codes.len() > 16
            || decision.reason_codes.iter().any(|reason| {
                reason.is_empty() || reason.len() > 80 || reason.chars().any(char::is_control)
            })
        {
            return Err(AppError::InvalidRequest(
                "routing decision evidence is outside supported bounds".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_metadata_text(
    field: &str,
    value: Option<&str>,
    required: bool,
) -> Result<(), AppError> {
    if required && value.is_none_or(str::is_empty) {
        return Err(AppError::InvalidRequest(format!(
            "{field} metadata is required"
        )));
    }
    if value.is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control)) {
        return Err(AppError::InvalidRequest(format!(
            "{field} metadata must not exceed 256 non-control bytes"
        )));
    }
    Ok(())
}

fn validate_audit_event(input: &AuditEventInput) -> Result<(), AppError> {
    if !matches!(input.severity.as_str(), "info" | "warning" | "error") {
        return Err(AppError::InvalidRequest(
            "audit severity must be info, warning, or error".to_owned(),
        ));
    }
    for (field, value, max_len) in [
        ("type", input.activity_type.as_str(), 80),
        ("actorId", input.actor_id.as_str(), 160),
        ("actorName", input.actor_name.as_str(), 160),
        ("target", input.target.as_str(), 500),
        ("message", input.message.as_str(), 1_000),
    ] {
        if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
            return Err(AppError::InvalidRequest(format!(
                "audit {field} must contain 1-{max_len} non-control bytes"
            )));
        }
    }
    Ok(())
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
        username: request.username.clone(),
        api_key_id: request.api_key_id.clone(),
        api_key_name: request.api_key_name.clone(),
        api_key_group: request.api_key_group.clone(),
        team_id: request.team_id.clone(),
        team_name: request.team_name.clone(),
        client_ip: request.client_ip.clone(),
        client_protocol: request.client_protocol.clone(),
        requested_model: request.requested_model.clone(),
        request_path: request.request_path.clone(),
        traffic_class: request.traffic_class.clone(),
        tool_use_requested: request.tool_use_requested,
        provider_id: request.provider_id.clone(),
        resolved_model: request.resolved_model.clone(),
        provider_protocol: request.provider_protocol.clone(),
        last_attempt_id: request.last_attempt_id.clone(),
        model_pricing: request.model_pricing.clone(),
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
        actual_cost_microunits: record.actual_cost_microunits,
        billable_cost_microunits: record.billable_cost_microunits,
        pricing_evidence: record.pricing_evidence.clone(),
        currency: "USD".to_owned(),
        billing_mode: record.billing_mode.clone(),
        chargeable: record.chargeable,
        latency_ms: record.latency_ms,
        first_byte_latency_ms: record.first_byte_latency_ms,
        tool_outcome: record.tool_outcome.clone(),
        tool_repair_attempted: record.tool_repair_attempted,
        tool_repair_recovered: record.tool_repair_recovered,
        retry_count: record.retry_count,
        fallback_from_provider: record.fallback_from_provider.clone(),
        routing_decision: request.routing_decision.clone(),
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
        actual_cost_microunits: record.actual_cost_microunits,
        billable_cost_microunits: record.billable_cost_microunits,
        pricing_evidence: record.pricing_evidence.clone(),
        currency: "USD".to_owned(),
        billing_mode: record.billing_mode.clone(),
        chargeable: record.chargeable,
        latency_ms: record.latency_ms,
        first_byte_latency_ms: record.first_byte_latency_ms,
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
        username: row.try_get("username")?,
        api_key_id: row.try_get("api_key_id")?,
        api_key_name: row.try_get("api_key_name")?,
        api_key_group: row.try_get("api_key_group")?,
        team_id: row.try_get("team_id")?,
        team_name: row.try_get("team_name")?,
        client_ip: row.try_get("client_ip")?,
        client_protocol: row.try_get("client_protocol")?,
        requested_model: row.try_get("requested_model")?,
        request_path: row.try_get("request_path")?,
        traffic_class: row.try_get("traffic_class")?,
        tool_use_requested: row.try_get("tool_use_requested")?,
        provider_id: row.try_get("provider_id")?,
        resolved_model: row.try_get("resolved_model")?,
        provider_protocol: row.try_get("provider_protocol")?,
        last_attempt_id: row.try_get("last_attempt_id")?,
        model_pricing: row.try_get("model_pricing")?,
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
        actual_cost_microunits: row.try_get("actual_cost_microunits")?,
        billable_cost_microunits: row.try_get("billable_cost_microunits")?,
        pricing_evidence: row.try_get("pricing_evidence")?,
        currency: row.try_get("currency")?,
        billing_mode: row.try_get("billing_mode")?,
        chargeable: row.try_get("chargeable")?,
        latency_ms: row.try_get("latency_ms")?,
        first_byte_latency_ms: row.try_get("first_byte_latency_ms")?,
        tool_outcome: row.try_get("tool_outcome")?,
        tool_repair_attempted: row.try_get("tool_repair_attempted")?,
        tool_repair_recovered: row.try_get("tool_repair_recovered")?,
        retry_count: row.try_get("retry_count")?,
        fallback_from_provider: row.try_get("fallback_from_provider")?,
        routing_decision: routing_decision_from_pg(row)?,
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
        actual_cost_microunits: row.try_get("actual_cost_microunits")?,
        billable_cost_microunits: row.try_get("billable_cost_microunits")?,
        pricing_evidence: row.try_get("pricing_evidence")?,
        currency: row.try_get("currency")?,
        billing_mode: row.try_get("billing_mode")?,
        chargeable: row.try_get("chargeable")?,
        latency_ms: row.try_get("latency_ms")?,
        first_byte_latency_ms: row.try_get("first_byte_latency_ms")?,
        lease_owner: row.try_get("lease_owner")?,
        lease_expires_at_ms: row.try_get("lease_expires_at_ms")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
        completed_at_ms: row.try_get("completed_at_ms")?,
    })
}

fn routing_decision_from_pg(row: &PgRow) -> Result<Option<RoutingDecisionEvidence>, sqlx::Error> {
    let Some(decision_id) = row.try_get::<Option<String>, _>("routing_decision_id")? else {
        return Ok(None);
    };
    Ok(Some(RoutingDecisionEvidence {
        decision_id,
        group_id: row.try_get("routing_group_id")?,
        profile: row.try_get("routing_profile")?,
        policy_version: row.try_get("routing_policy_version")?,
        mode: row.try_get("routing_mode")?,
        candidate_count: usize::try_from(row.try_get::<i32, _>("routing_candidate_count")?)
            .unwrap_or_default(),
        selected_provider: row.try_get("routing_selected_provider")?,
        selected_model: row.try_get("routing_selected_model")?,
        recommended_provider: row.try_get("routing_recommended_provider")?,
        recommended_model: row.try_get("routing_recommended_model")?,
        selected_score: row.try_get("routing_selected_score")?,
        recommended_score: row.try_get("routing_recommended_score")?,
        reason_codes: row.try_get("routing_reason_codes")?,
        session_affinity: row.try_get("routing_session_affinity")?,
        shadow_disagreement: row.try_get("routing_shadow_disagreement")?,
    }))
}

fn operational_log_row(request: &EnterpriseRequestRow) -> Value {
    let input_tokens = nonnegative_u64(request.input_tokens);
    let output_tokens = nonnegative_u64(request.output_tokens);
    let cache_write_tokens = nonnegative_u64(request.cache_write_tokens);
    let cache_read_tokens = nonnegative_u64(request.cache_read_tokens);
    let billed_input_tokens = input_tokens
        .saturating_add(cache_write_tokens)
        .saturating_add(cache_read_tokens);
    let total_tokens = billed_input_tokens.saturating_add(output_tokens);
    let cache_tokens = cache_write_tokens.saturating_add(cache_read_tokens);
    let cache_hit_rate = if billed_input_tokens == 0 {
        0.0
    } else {
        cache_tokens as f64 / billed_input_tokens as f64 * 100.0
    };
    let resolved_model = request
        .resolved_model
        .as_deref()
        .unwrap_or(&request.requested_model);
    let provider = request.provider_id.as_deref().unwrap_or("unrouted");
    let pricing = request
        .model_pricing
        .clone()
        .and_then(|value| serde_json::from_value::<ModelPricing>(value).ok())
        .unwrap_or_else(|| pricing::pricing_for_model(resolved_model));
    let cost_estimate = request.cost_amount_microunits.max(0) as f64 / 1_000_000.0;
    let actual_cost = request.actual_cost_microunits.map(microunits_usd);
    let billable_cost = request.billable_cost_microunits.map(microunits_usd);
    let reconciliation_status = if billable_cost.is_some() && actual_cost.is_none() {
        "partially_billable"
    } else if billable_cost.is_some() {
        "billable"
    } else if actual_cost.is_some() {
        "actual_unbillable"
    } else {
        "estimate_only"
    };
    let status = if request.state == "completed" {
        "success"
    } else if request
        .terminal_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("timeout"))
    {
        "timeout"
    } else {
        "error"
    };

    json!({
        "id": request.ledger_id,
        "requestId": request.request_id,
        "attemptId": request.last_attempt_id,
        "timestamp": request.created_at_ms.to_string(),
        "userId": request.principal_id,
        "username": request.username,
        "apiKeyId": request.api_key_id,
        "apiKeyName": request.api_key_name,
        "apiKeyGroup": request.api_key_group,
        "teamId": request.team_id,
        "teamName": request.team_name,
        "model": request.requested_model,
        "resolvedModel": resolved_model,
        "provider": provider,
        "protocol": request.provider_protocol,
        "clientProtocol": request.client_protocol,
        "toolUseRequested": request.tool_use_requested,
        "toolOutcome": request.tool_outcome,
        "trafficClass": request.traffic_class,
        "routingDecision": request.routing_decision,
        "toolRepairAttempted": request.tool_repair_attempted,
        "toolRepairRecovered": request.tool_repair_recovered,
        "stream": if request.stream { "stream" } else { "non-stream" },
        "status": status,
        "statusCode": request.status_code,
        "terminalReason": request.terminal_reason,
        "inputTokens": input_tokens,
        "outputTokens": output_tokens,
        "cacheWriteTokens": cache_write_tokens,
        "cacheReadTokens": cache_read_tokens,
        "billedInputTokens": billed_input_tokens,
        "totalTokens": total_tokens,
        "cacheHitRate": cache_hit_rate,
        "costEstimate": cost_estimate,
        "actualCost": actual_cost,
        "billableCost": billable_cost,
        "reconciliationStatus": reconciliation_status,
        "pricingEvidence": request.pricing_evidence,
        "modelPricing": pricing,
        "costBreakdown": {
            "inputCost": pricing::cost_component(input_tokens, pricing.input_per_million),
            "outputCost": pricing::cost_component(output_tokens, pricing.output_per_million),
            "cacheWriteCost": pricing::cost_component(cache_write_tokens, pricing.cache_write_per_million),
            "cacheReadCost": pricing::cost_component(cache_read_tokens, pricing.cache_read_per_million),
            "totalCost": cost_estimate,
        },
        "latencyMs": nonnegative_u64(request.latency_ms),
        "firstByteLatencyMs": request.first_byte_latency_ms.map(nonnegative_u64),
        "retryCount": request.retry_count.max(0),
        "fallbackFromProvider": request.fallback_from_provider,
        "clientIp": request.client_ip,
        "requestPath": request.request_path,
        "billingMode": request.billing_mode,
        "chargeable": request.chargeable,
        "errorMessage": request.error_message,
    })
}

fn nonnegative_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

fn optional_nonnegative_u64(row: &PgRow, name: &str) -> Result<u64, sqlx::Error> {
    Ok(row
        .try_get::<Option<i64>, _>(name)?
        .map(nonnegative_u64)
        .unwrap_or(0))
}

fn latency_stats_from_pg(row: &PgRow) -> Result<Value, sqlx::Error> {
    Ok(json!({
        "p50": optional_nonnegative_u64(row, "p50")?,
        "p90": optional_nonnegative_u64(row, "p90")?,
        "p95": optional_nonnegative_u64(row, "p95")?,
        "p99": optional_nonnegative_u64(row, "p99")?,
        "avg": nonnegative_u64(row.try_get("avg")?),
        "max": nonnegative_u64(row.try_get("max")?),
        "count": nonnegative_u64(row.try_get("count")?),
    }))
}

fn dashboard_bucket_timestamp(start_ms: i64, bucket_ms: i64, index: usize) -> String {
    start_ms
        .saturating_add(
            i64::try_from(index)
                .unwrap_or(i64::MAX)
                .saturating_mul(bucket_ms),
        )
        .max(0)
        .to_string()
}

fn dashboard_value_series(values: &[u64], start_ms: i64, bucket_ms: i64) -> Vec<Value> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            json!({
                "timestamp": dashboard_bucket_timestamp(start_ms, bucket_ms, index),
                "value": value,
            })
        })
        .collect()
}

fn request_total_tokens(request: &MemoryRecord) -> u64 {
    nonnegative_u64(request.input_tokens)
        .saturating_add(nonnegative_u64(request.output_tokens))
        .saturating_add(nonnegative_u64(request.cache_write_tokens))
        .saturating_add(nonnegative_u64(request.cache_read_tokens))
}

fn estimate_total_tokens(estimate: UsageEstimate) -> u64 {
    estimate
        .input_tokens
        .saturating_add(estimate.output_tokens)
        .saturating_add(estimate.cache_write_tokens)
        .saturating_add(estimate.cache_read_tokens)
}

fn microunits_usd(value: i64) -> f64 {
    value.max(0) as f64 / 1_000_000.0
}

fn quota_value_from_totals(
    quota_type: &str,
    requests: u64,
    tokens: u64,
    cost_microunits: i64,
) -> f64 {
    match quota_type {
        "requests" => requests as f64,
        "tokens" => tokens as f64,
        "cost" => microunits_usd(cost_microunits),
        _ => 0.0,
    }
}

impl Drop for LedgerLease {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl LedgerOutcome {
    #[cfg(test)]
    pub(crate) fn provider_attempt(
        success: bool,
        status_code: u16,
        error_message: Option<String>,
        estimate: UsageEstimate,
        billing_mode: &'static str,
        latency: Duration,
    ) -> Self {
        let pricing_evidence = estimate.billable_cost.map(|_| pricing::PricingEvidence {
            provider: "test-provider".to_owned(),
            model: "test-model".to_owned(),
            method: pricing::PricingMethod::ExactRateCard,
            currency: "USD".to_owned(),
            version: "test-card-v1".to_owned(),
            effective_at: "2026-01-01T00:00:00Z".to_owned(),
            source: pricing::PricingSource::InternalChargeback,
            service_tier: pricing::PricingServiceTier::Standard,
            region: None,
            evidence: "test://rate-card/v1".to_owned(),
            rates: None,
        });
        Self::provider_attempt_with_evidence(
            success,
            status_code,
            error_message,
            estimate,
            pricing_evidence,
            billing_mode,
            latency,
        )
    }

    pub(crate) fn provider_attempt_with_evidence(
        success: bool,
        status_code: u16,
        error_message: Option<String>,
        estimate: UsageEstimate,
        pricing_evidence: Option<pricing::PricingEvidence>,
        billing_mode: &'static str,
        latency: Duration,
    ) -> Self {
        debug_assert!(matches!(
            billing_mode,
            "local-estimate" | "upstream-returned"
        ));
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
            pricing_evidence: pricing_evidence
                .and_then(|evidence| serde_json::to_value(evidence).ok()),
            billing_mode: billing_mode.to_owned(),
            chargeable: true,
            latency_ms: duration_millis_i64(latency),
            first_byte_latency_ms: None,
            tool_outcome: "not_requested".to_owned(),
            tool_repair_attempted: false,
            tool_repair_recovered: false,
            retry_count: 0,
            fallback_from_provider: None,
        }
    }

    pub(crate) fn from_usage(usage: &UsageEventInput) -> Self {
        Self::from_usage_with_latency(usage, usage.latency)
    }

    pub(crate) fn from_usage_with_latency(usage: &UsageEventInput, latency: Duration) -> Self {
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
            pricing_evidence: usage
                .pricing_evidence
                .as_ref()
                .and_then(|evidence| serde_json::to_value(evidence).ok()),
            billing_mode: usage.billing_mode.clone(),
            chargeable: usage.chargeable,
            latency_ms: duration_millis_i64(latency),
            first_byte_latency_ms: usage.first_byte_latency.map(duration_millis_i64),
            tool_outcome: usage.tool_outcome.clone(),
            tool_repair_attempted: usage.tool_repair_attempted,
            tool_repair_recovered: usage.tool_repair_recovered,
            retry_count: i32::try_from(usage.retry_count).unwrap_or(i32::MAX),
            fallback_from_provider: usage.fallback_from_provider.clone(),
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

fn retained_request_fingerprint(ledger_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(RETAINED_REQUEST_FINGERPRINT_PREFIX.as_bytes());
    hasher.update(ledger_id.as_bytes());
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

fn retention_days_from_env(name: &str, default: u64) -> Result<u64, AppError> {
    let days = match env::var(name) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|_| AppError::Config(format!("{name} must be an integer number of days")))?,
        Err(_) => default,
    };
    if !(1..=3_650).contains(&days) {
        return Err(AppError::Config(format!(
            "{name} must be between 1 and 3650 days"
        )));
    }
    Ok(days)
}

fn retention_flag_from_env(name: &str) -> Result<bool, AppError> {
    match env::var(name) {
        Err(_) => Ok(false),
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" | "" => Ok(false),
            _ => Err(AppError::Config(format!(
                "{name} must be a boolean (1/0, true/false, yes/no, or on/off)"
            ))),
        },
    }
}

fn duration_secs_i32(duration: Duration) -> i32 {
    i32::try_from(duration.as_secs()).unwrap_or(i32::MAX)
}

fn duration_millis_i64(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn validate_ops_observation(observation: &OpsObservation) -> Result<(), AppError> {
    let bounded = [
        ("eventKey", observation.event_key.as_str(), 240_usize),
        ("detectorType", observation.detector_type.as_str(), 80),
        ("title", observation.title.as_str(), 240),
        ("summary", observation.summary.as_str(), 2_000),
        (
            "recoveryCriteria",
            observation.recovery_criteria.as_str(),
            1_000,
        ),
    ];
    for (name, value, maximum) in bounded {
        let length = value.chars().count();
        if length == 0 || length > maximum {
            return Err(AppError::InvalidRequest(format!(
                "{name} must contain 1 to {maximum} characters"
            )));
        }
    }
    let allowed = match observation.event_key.as_str() {
        "readiness:gateway" => {
            observation.detector_type == "readiness_storage"
                && matches!(observation.severity, OpsSeverity::Sev1 | OpsSeverity::Sev2)
        }
        "provider:availability" => {
            observation.detector_type == "provider_health"
                && matches!(observation.severity, OpsSeverity::Sev2 | OpsSeverity::Sev3)
        }
        "requests:failure-ratio" => {
            observation.detector_type == "request_anomaly"
                && matches!(observation.severity, OpsSeverity::Sev2 | OpsSeverity::Sev3)
        }
        "budget:capacity" => {
            observation.detector_type == "budget_quota"
                && matches!(observation.severity, OpsSeverity::Sev2 | OpsSeverity::Sev3)
        }
        "ledger:finalization-backlog" => {
            observation.detector_type == "ledger_backlog"
                && matches!(observation.severity, OpsSeverity::Sev2 | OpsSeverity::Sev3)
        }
        "change:verification" => {
            observation.detector_type == "post_change_verification"
                && observation.severity == OpsSeverity::Sev2
        }
        _ => false,
    };
    if !allowed {
        return Err(AppError::Forbidden(
            "observation is outside the versioned operations rule allowlist".to_owned(),
        ));
    }
    if !observation.affected_scope.is_object() || !observation.evidence.is_object() {
        return Err(AppError::InvalidRequest(
            "affectedScope and evidence must be JSON objects".to_owned(),
        ));
    }
    if serde_json::to_vec(&observation.evidence)?.len() > 32 * 1_024 {
        return Err(AppError::InvalidRequest(
            "incident evidence must not exceed 32 KiB".to_owned(),
        ));
    }
    if serde_json::to_vec(&observation.affected_scope)?.len() > 8 * 1_024 {
        return Err(AppError::InvalidRequest(
            "incident affectedScope must not exceed 8 KiB".to_owned(),
        ));
    }
    let now = u64::try_from(now_millis()).unwrap_or_default();
    if observation.observed_at_ms == 0
        || observation.observed_at_ms > now.saturating_add(5 * 60 * 1_000)
    {
        return Err(AppError::InvalidRequest(
            "observedAtMs must be a current, non-zero timestamp".to_owned(),
        ));
    }
    Ok(())
}

fn validate_ops_heartbeat(heartbeat: &OpsHeartbeat) -> Result<(), AppError> {
    for (name, value, maximum) in [
        ("instanceId", heartbeat.instance_id.as_str(), 160_usize),
        ("agentVersion", heartbeat.agent_version.as_str(), 80),
        ("ruleSetVersion", heartbeat.rule_set_version.as_str(), 80),
    ] {
        let length = value.chars().count();
        if length == 0 || length > maximum {
            return Err(AppError::InvalidRequest(format!(
                "{name} must contain 1 to {maximum} characters"
            )));
        }
    }
    if heartbeat.selected_model.as_deref().is_some_and(|value| {
        value.is_empty() || value.chars().count() > 320 || value.chars().any(char::is_control)
    }) {
        return Err(AppError::InvalidRequest(
            "heartbeat selectedModel must contain 1 to 320 non-control characters".to_owned(),
        ));
    }
    if !matches!(
        heartbeat.model_status.as_str(),
        "disabled" | "configured" | "missing_credential" | "error"
    ) {
        return Err(AppError::InvalidRequest(
            "heartbeat modelStatus is unsupported".to_owned(),
        ));
    }
    if !matches!(
        heartbeat.mode.as_str(),
        "disabled" | "replay" | "shadow" | "read_only"
    ) {
        return Err(AppError::InvalidRequest(
            "agent mode must be disabled, replay, shadow, or read_only".to_owned(),
        ));
    }
    if !(10..=3_600).contains(&heartbeat.interval_seconds) {
        return Err(AppError::InvalidRequest(
            "agent intervalSeconds must be between 10 and 3600".to_owned(),
        ));
    }
    let now = u64::try_from(now_millis()).unwrap_or_default();
    if heartbeat.observed_at_ms == 0
        || heartbeat.observed_at_ms > now.saturating_add(5 * 60 * 1_000)
    {
        return Err(AppError::InvalidRequest(
            "heartbeat observedAtMs must be a current, non-zero timestamp".to_owned(),
        ));
    }
    Ok(())
}

fn validate_ops_actor(actor_id: &str, actor_name: &str) -> Result<(), AppError> {
    if actor_id.is_empty()
        || actor_name.is_empty()
        || actor_id.chars().count() > 160
        || actor_name.chars().count() > 160
    {
        return Err(AppError::InvalidRequest(
            "operations actor identity is missing or too long".to_owned(),
        ));
    }
    Ok(())
}

fn ops_evidence_hash(value: &Value) -> Result<String, AppError> {
    let encoded = serde_json::to_vec(value)?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

async fn fetch_ops_incident_row(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    incident_id: &str,
) -> Result<PgRow, AppError> {
    Ok(sqlx::query(
        "SELECT *,
            (EXTRACT(EPOCH FROM first_seen_at) * 1000)::bigint AS first_seen_at_ms,
            (EXTRACT(EPOCH FROM last_seen_at) * 1000)::bigint AS last_seen_at_ms,
            (EXTRACT(EPOCH FROM resolved_at) * 1000)::bigint AS resolved_at_ms
         FROM modelport_ops_incidents WHERE incident_id = $1",
    )
    .bind(incident_id)
    .fetch_one(&mut **transaction)
    .await?)
}

fn ops_incident_summary_from_row(row: &PgRow) -> Result<OpsIncidentSummary, AppError> {
    let severity: String = row.try_get("severity")?;
    let status: String = row.try_get("status")?;
    Ok(OpsIncidentSummary {
        id: row.try_get("incident_id")?,
        event_key: row.try_get("event_key")?,
        detector_type: row.try_get("detector_type")?,
        severity: parse_ops_severity(&severity)?,
        status: parse_ops_status(&status)?,
        title: row.try_get("title")?,
        summary: row.try_get("summary")?,
        affected_scope: row.try_get("affected_scope")?,
        recovery_criteria: row.try_get("recovery_criteria")?,
        first_seen_at_ms: nonnegative_u64(row.try_get("first_seen_at_ms")?),
        last_seen_at_ms: nonnegative_u64(row.try_get("last_seen_at_ms")?),
        resolved_at_ms: row
            .try_get::<Option<i64>, _>("resolved_at_ms")?
            .map(nonnegative_u64),
        occurrence_count: nonnegative_u64(row.try_get("occurrence_count")?),
    })
}

fn parse_ops_severity(value: &str) -> Result<OpsSeverity, AppError> {
    match value {
        "SEV-1" => Ok(OpsSeverity::Sev1),
        "SEV-2" => Ok(OpsSeverity::Sev2),
        "SEV-3" => Ok(OpsSeverity::Sev3),
        "SEV-4" => Ok(OpsSeverity::Sev4),
        _ => Err(AppError::Database(
            "operations incident contains an invalid severity".to_owned(),
        )),
    }
}

fn parse_ops_status(value: &str) -> Result<OpsIncidentStatus, AppError> {
    match value {
        "open" => Ok(OpsIncidentStatus::Open),
        "acknowledged" => Ok(OpsIncidentStatus::Acknowledged),
        "mitigating" => Ok(OpsIncidentStatus::Mitigating),
        "monitoring" => Ok(OpsIncidentStatus::Monitoring),
        "resolved" => Ok(OpsIncidentStatus::Resolved),
        "suppressed" => Ok(OpsIncidentStatus::Suppressed),
        _ => Err(AppError::Database(
            "operations incident contains an invalid status".to_owned(),
        )),
    }
}

fn highest_ops_severity(values: Vec<OpsSeverity>) -> Option<OpsSeverity> {
    values.into_iter().min_by_key(|severity| match severity {
        OpsSeverity::Sev1 => 1,
        OpsSeverity::Sev2 => 2,
        OpsSeverity::Sev3 => 3,
        OpsSeverity::Sev4 => 4,
    })
}

fn ops_severity_from_rank(value: i32) -> Option<OpsSeverity> {
    match value {
        1 => Some(OpsSeverity::Sev1),
        2 => Some(OpsSeverity::Sev2),
        3 => Some(OpsSeverity::Sev3),
        4 => Some(OpsSeverity::Sev4),
        _ => None,
    }
}

fn ops_agent_summary(heartbeat: &OpsHeartbeat) -> OpsAgentSummary {
    OpsAgentSummary {
        instance_id: heartbeat.instance_id.clone(),
        agent_version: heartbeat.agent_version.clone(),
        mode: heartbeat.mode.clone(),
        rule_set_version: heartbeat.rule_set_version.clone(),
        observed_at_ms: heartbeat.observed_at_ms,
        queue_depth: heartbeat.queue_depth,
        interval_seconds: heartbeat.interval_seconds,
        online: u64::try_from(now_millis())
            .unwrap_or_default()
            .saturating_sub(heartbeat.observed_at_ms)
            <= heartbeat.interval_seconds.saturating_mul(3_000),
        analysis_enabled: heartbeat.analysis_enabled,
        selected_model: heartbeat.selected_model.clone(),
        model_status: heartbeat.model_status.clone(),
        model_last_success_at_ms: heartbeat.model_last_success_at_ms,
    }
}

fn ops_agent_summary_from_row(row: &PgRow) -> Result<OpsAgentSummary, AppError> {
    let observed_at_ms = nonnegative_u64(row.try_get("observed_at_ms")?);
    Ok(OpsAgentSummary {
        instance_id: row.try_get("instance_id")?,
        agent_version: row.try_get("agent_version")?,
        mode: row.try_get("mode")?,
        rule_set_version: row.try_get("rule_set_version")?,
        observed_at_ms,
        queue_depth: nonnegative_u64(row.try_get("queue_depth")?),
        interval_seconds: nonnegative_u64(row.try_get("interval_seconds")?),
        online: u64::try_from(now_millis())
            .unwrap_or_default()
            .saturating_sub(observed_at_ms)
            <= nonnegative_u64(row.try_get("interval_seconds")?).saturating_mul(3_000),
        analysis_enabled: row.try_get("analysis_enabled")?,
        selected_model: row.try_get("selected_model")?,
        model_status: row.try_get("model_status")?,
        model_last_success_at_ms: row
            .try_get::<Option<i64>, _>("model_last_success_at_ms")?
            .map(nonnegative_u64),
    })
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
    use crate::{
        control::ApiKeyPolicy,
        domain::{ClientProtocol, RequestId},
    };

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
            actual_cost: Some(cost_estimate),
            billable_cost: Some(cost_estimate),
        }
    }

    fn unreconciled_estimate(cost_estimate: f64) -> UsageEstimate {
        UsageEstimate {
            actual_cost: None,
            billable_cost: None,
            ..estimate(cost_estimate)
        }
    }

    fn test_pricing_evidence() -> pricing::PricingEvidence {
        pricing::PricingEvidence {
            provider: "test-provider".to_owned(),
            model: "gpt-test".to_owned(),
            method: pricing::PricingMethod::ExactRateCard,
            currency: "USD".to_owned(),
            version: "test-card-v1".to_owned(),
            effective_at: "2026-01-01T00:00:00Z".to_owned(),
            source: pricing::PricingSource::InternalChargeback,
            service_tier: pricing::PricingServiceTier::Standard,
            region: None,
            evidence: "test://rate-card/v1".to_owned(),
            rates: None,
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

    fn usage_policy(
        subject: &str,
        team_id: Option<&str>,
        api_key_policy: ApiKeyPolicy,
        quotas: Vec<UsageQuotaLimit>,
    ) -> UsagePolicySnapshot {
        UsagePolicySnapshot {
            user_id: "usr_atomic_usage".to_owned(),
            username: "atomic-usage".to_owned(),
            api_key_id: Some("key_atomic_usage".to_owned()),
            quota_subject_id: Some(subject.to_owned()),
            quota_subject_aliases: vec![subject.to_owned()],
            team_id: team_id.map(str::to_owned),
            api_key_policy,
            quotas,
        }
    }

    async fn begin_usage_request(
        ledger: &EnterpriseLedger,
        request_id: &str,
        subject: &str,
        team_id: Option<&str>,
    ) -> LedgerRequest {
        let context = RequestContext::legacy(
            RequestId::from_string(request_id),
            "usr_atomic_usage",
            ClientProtocol::OpenAiChatCompletions,
        );
        ledger
            .begin_request_with_metadata(
                &context,
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    username: "atomic-usage".to_owned(),
                    api_key_id: Some("key_atomic_usage".to_owned()),
                    quota_subject_id: Some(subject.to_owned()),
                    team_id: team_id.map(str::to_owned),
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap()
    }

    async fn begin_usage_attempt(
        ledger: &EnterpriseLedger,
        request: &LedgerRequest,
        attempt_id: &str,
        policy: &UsagePolicySnapshot,
        estimate: UsageEstimate,
    ) -> Result<LedgerAttempt, AppError> {
        ledger
            .begin_attempt_with_pricing(
                request,
                &AttemptId::from_string(attempt_id),
                "openai",
                "gpt-test",
                "openai-compatible",
                AttemptPricingEvidence {
                    estimate,
                    verified: true,
                    usage_policy: policy,
                },
            )
            .await
    }

    #[tokio::test]
    async fn postgres_critical_paths_use_database_transactions_and_aggregation() {
        let Ok(database_url) = std::env::var("MODELPORT_TEST_DATABASE_URL") else {
            return;
        };
        let ledger = EnterpriseLedger::postgres_for_tests(&database_url)
            .await
            .unwrap();
        let LedgerBackend::Postgres(pool) = ledger.backend.as_ref() else {
            unreachable!();
        };
        sqlx::query(
            "TRUNCATE TABLE
                modelport_usage_reservations,
                modelport_budget_events,
                modelport_budget_reservations,
                modelport_routing_feedback,
                modelport_routing_decisions,
                modelport_provider_attempts,
                modelport_gateway_requests",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE modelport_budget_accounts
             SET limit_microunits = NULL,
                 reserved_microunits = 0,
                 settled_microunits = 0,
                 version = 0,
                 updated_at = now()",
        )
        .execute(pool)
        .await
        .unwrap();

        let context = context();
        let request = ledger
            .begin_request_with_metadata(
                &context,
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    username: "database-test".to_owned(),
                    api_key_id: Some("key_postgres_a".to_owned()),
                    quota_subject_id: Some("qsub_postgres_rotation".to_owned()),
                    traffic_class: "business".to_owned(),
                    routing_decision: Some(RoutingDecisionEvidence {
                        decision_id: format!("rtd_{}", Uuid::new_v4().simple()),
                        group_id: Some("general".to_owned()),
                        profile: "balanced".to_owned(),
                        policy_version: "test-v1".to_owned(),
                        mode: "shadow".to_owned(),
                        candidate_count: 2,
                        selected_provider: "openai".to_owned(),
                        selected_model: "gpt-test".to_owned(),
                        recommended_provider: "openai".to_owned(),
                        recommended_model: "gpt-test".to_owned(),
                        selected_score: 0.75,
                        recommended_score: 0.80,
                        reason_codes: vec!["profile_balanced".to_owned()],
                        session_affinity: false,
                        shadow_disagreement: false,
                    }),
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap();
        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string(format!("att_{}", Uuid::new_v4().simple())),
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.25),
            )
            .await
            .unwrap();
        let usage = UsageEventInput {
            request_id: Some("req_postgres_operational_test".to_owned()),
            attempt_id: Some(attempt.attempt_id.clone()),
            resolved_model: "gpt-test".to_owned(),
            provider: "openai".to_owned(),
            protocol: "openai-compatible".to_owned(),
            tool_use_requested: false,
            tool_outcome: "not_requested".to_owned(),
            traffic_class: "business".to_owned(),
            tool_repair_attempted: false,
            tool_repair_recovered: false,
            success: true,
            timed_out: false,
            status_code: 200,
            terminal_reason: "completed".to_owned(),
            estimate: estimate(0.125),
            model_pricing: None,
            pricing_evidence: Some(test_pricing_evidence()),
            billing_mode: "upstream-returned".to_owned(),
            chargeable: true,
            latency: Duration::from_millis(120),
            first_byte_latency: Some(Duration::from_millis(40)),
            retry_count: 0,
            fallback_from_provider: None,
            error_message: None,
        };
        let outcome = LedgerOutcome::from_usage(&usage);
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger
            .finalize_request_usage(&request, &usage)
            .await
            .unwrap();

        let rotated_policy = UsagePolicySnapshot {
            user_id: "usr_test".to_owned(),
            username: "database-test".to_owned(),
            api_key_id: Some("key_postgres_b".to_owned()),
            quota_subject_id: Some("qsub_postgres_rotation".to_owned()),
            quota_subject_aliases: vec!["qsub_postgres_rotation".to_owned()],
            team_id: None,
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.125,
                ..ApiKeyPolicy::default()
            },
            quotas: Vec::new(),
        };
        assert!(matches!(
            ledger
                .check_usage_policy(&rotated_policy, estimate(0.01), true)
                .await,
            Err(AppError::QuotaExceeded(_))
        ));

        let now = u64::try_from(now_millis()).unwrap_or(u64::MAX);
        let logs = ledger
            .operational_logs(&OperationalLogQuery {
                page: 1,
                page_size: 20,
                provider: Some("openai".to_owned()),
                date_from: Some(now.saturating_sub(60_000)),
                date_to: Some(now.saturating_add(60_000)),
                ..OperationalLogQuery::default()
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(logs.total, 1);
        assert_eq!(logs.logs[0]["provider"], "openai");
        assert_eq!(logs.logs[0]["routingDecision"]["mode"], "shadow");
        assert_eq!(logs.logs[0]["routingDecision"]["policyVersion"], "test-v1");
        assert_eq!(logs.summary["totalRequests"], 1);
        assert_eq!(logs.summary["totalTokens"], 120);
        assert_eq!(logs.summary["latencyP95Ms"], 120);

        let today_start = (now / (24 * 60 * 60 * 1_000)) * (24 * 60 * 60 * 1_000);
        let dashboard = ledger
            .dashboard_snapshot(
                now.saturating_sub(60_000),
                now.saturating_add(60_000),
                60_000,
                today_start,
                (0, 0),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dashboard.matched_requests, 1);
        assert_eq!(dashboard.usage_summary.total_requests, 1);
        assert_eq!(dashboard.summary["totalTokens"], 120);
        assert_eq!(dashboard.provider_usage["openai"].requests_total, 1);
        assert_eq!(
            ledger.onboarding_milestones().await.unwrap(),
            (true, true),
            "onboarding evidence must use the current gateway ledger schema"
        );

        let latency = ledger
            .latency_stats_since(now.saturating_sub(60_000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latency["sampleCount"], 1);
        assert_eq!(latency["p95"], 120);

        sqlx::query(
            "UPDATE modelport_gateway_requests
             SET created_at = now() - interval '100 days'
             WHERE ledger_id = $1",
        )
        .bind(&request.ledger_id)
        .execute(pool)
        .await
        .unwrap();
        ledger
            .run_retention(
                RetentionPolicy {
                    request_detail_days: 1,
                    user_usage_days: 2,
                    audit_days: 3,
                    legal_hold: false,
                    content_persistence: false,
                },
                false,
            )
            .await
            .unwrap();
        let retained_fingerprint = sqlx::query_scalar::<_, String>(
            "SELECT request_fingerprint
             FROM modelport_gateway_requests
             WHERE ledger_id = $1",
        )
        .bind(&request.ledger_id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            retained_fingerprint,
            retained_request_fingerprint(&request.ledger_id)
        );
        assert_eq!(retained_fingerprint.len(), 64);
        assert_ne!(retained_fingerprint, TEST_FINGERPRINT);
        assert!(matches!(
            ledger
                .check_usage_policy(&rotated_policy, estimate(0.01), true)
                .await,
            Err(AppError::QuotaExceeded(_))
        ));

        sqlx::query(
            "TRUNCATE TABLE
                modelport_usage_reservations,
                modelport_budget_events,
                modelport_budget_reservations,
                modelport_routing_feedback,
                modelport_routing_decisions,
                modelport_provider_attempts,
                modelport_gateway_requests",
        )
        .execute(pool)
        .await
        .unwrap();
        set_local_budget(&ledger, 1_000_000).await;

        let budget_request_a = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        let budget_request_b = ledger
            .begin_request(&context, "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        let budget_attempt_a =
            AttemptId::from_string(format!("att_postgres_budget_a_{}", Uuid::new_v4().simple()));
        let budget_attempt_b =
            AttemptId::from_string(format!("att_postgres_budget_b_{}", Uuid::new_v4().simple()));
        let (budget_a, budget_b) = tokio::join!(
            ledger.begin_attempt(
                &budget_request_a,
                &budget_attempt_a,
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.75),
            ),
            ledger.begin_attempt(
                &budget_request_b,
                &budget_attempt_b,
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.75),
            )
        );
        assert_eq!(
            usize::from(budget_a.is_ok()) + usize::from(budget_b.is_ok()),
            1,
            "the PostgreSQL budget row lock must prevent concurrent overspend"
        );

        let idempotency_key = format!("postgres-idempotency-{}", Uuid::new_v4().simple());
        let (idempotent_a, idempotent_b) = tokio::join!(
            ledger.begin_request(
                &context,
                "gpt-test",
                false,
                Some(&idempotency_key),
                TEST_FINGERPRINT,
            ),
            ledger.begin_request(
                &context,
                "gpt-test",
                false,
                Some(&idempotency_key),
                TEST_FINGERPRINT,
            )
        );
        assert_eq!(
            usize::from(idempotent_a.is_ok()) + usize::from(idempotent_b.is_ok()),
            1,
            "the PostgreSQL idempotency constraint must admit exactly one request"
        );

        sqlx::query(
            "TRUNCATE TABLE
                modelport_usage_reservations,
                modelport_budget_events,
                modelport_budget_reservations,
                modelport_routing_feedback,
                modelport_routing_decisions,
                modelport_provider_attempts,
                modelport_gateway_requests",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE modelport_budget_accounts
             SET limit_microunits = NULL,
                 reserved_microunits = 0,
                 settled_microunits = 0,
                 version = 0,
                 updated_at = now()",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn postgres_usage_admission_is_atomic_across_concurrent_transactions() {
        let Ok(database_url) = std::env::var("MODELPORT_TEST_DATABASE_URL") else {
            return;
        };
        let ledger = EnterpriseLedger::postgres_for_tests(&database_url)
            .await
            .unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let tenant =
            TenantScope::from_strings(format!("org_atomic_{suffix}"), "prj_atomic", "env_atomic");
        let user_id = format!("usr_atomic_{suffix}");
        let subject = format!("qsub_atomic_{suffix}");
        let team_id = format!("team_atomic_{suffix}");
        let context_a = RequestContext::scoped(
            RequestId::from_string(format!("req_atomic_pg_{suffix}_a")),
            tenant.clone(),
            user_id.clone(),
            ClientProtocol::OpenAiChatCompletions,
        );
        let context_b = RequestContext::scoped(
            RequestId::from_string(format!("req_atomic_pg_{suffix}_b")),
            tenant.clone(),
            user_id.clone(),
            ClientProtocol::OpenAiChatCompletions,
        );
        let metadata = LedgerRequestMetadata {
            username: user_id.clone(),
            api_key_id: Some(format!("key_atomic_{suffix}")),
            quota_subject_id: Some(subject.clone()),
            team_id: Some(team_id.clone()),
            ..LedgerRequestMetadata::default()
        };
        let request_a = ledger
            .begin_request_with_metadata(
                &context_a,
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &metadata,
            )
            .await
            .unwrap();
        let request_b = ledger
            .begin_request_with_metadata(
                &context_b,
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &metadata,
            )
            .await
            .unwrap();
        let policy = UsagePolicySnapshot {
            user_id: user_id.clone(),
            username: user_id.clone(),
            api_key_id: metadata.api_key_id.clone(),
            quota_subject_id: Some(subject.clone()),
            quota_subject_aliases: vec![subject.clone()],
            team_id: Some(team_id),
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.75,
                team_daily_limit_usd: 0.75,
                ..ApiKeyPolicy::default()
            },
            quotas: vec![UsageQuotaLimit {
                id: format!("quota_atomic_{suffix}"),
                user_id,
                quota_type: "requests".to_owned(),
                limit: 1.0,
                period_start_ms: 0,
            }],
        };
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let first = {
            let ledger = ledger.clone();
            let barrier = barrier.clone();
            let policy = policy.clone();
            let suffix = suffix.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                begin_usage_attempt(
                    &ledger,
                    &request_a,
                    &format!("att_atomic_pg_{suffix}_a"),
                    &policy,
                    estimate(0.75),
                )
                .await
            })
        };
        let second = {
            let ledger = ledger.clone();
            let barrier = barrier.clone();
            let policy = policy.clone();
            let suffix = suffix.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                begin_usage_attempt(
                    &ledger,
                    &request_b,
                    &format!("att_atomic_pg_{suffix}_b"),
                    &policy,
                    estimate(0.75),
                )
                .await
            })
        };
        let (first, second) = tokio::join!(first, second);
        let first = first.unwrap();
        let second = second.unwrap();
        assert_ne!(first.is_ok(), second.is_ok());
        let rejected = if first.is_err() { first } else { second };
        assert!(matches!(rejected, Err(AppError::QuotaExceeded(_))));

        let LedgerBackend::Postgres(pool) = ledger.backend.as_ref() else {
            unreachable!();
        };
        let reservation = sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT count(*)::bigint,
                    COALESCE(sum(reserved_requests), 0)::bigint,
                    COALESCE(sum(reserved_cost_microunits), 0)::bigint
             FROM modelport_usage_reservations
             WHERE quota_subject_id = $1 AND state = 'reserved'",
        )
        .bind(&subject)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(reservation, (1, 1, 750_000));
        let tenant_reservations = sqlx::query_scalar::<_, i64>(
            "SELECT count(*)::bigint
             FROM modelport_budget_reservations
             WHERE organization_id = $1
               AND project_id = $2
               AND environment_id = $3
               AND state = 'reserved'",
        )
        .bind(tenant.organization_id.as_str())
        .bind(tenant.project_id.as_str())
        .bind(tenant.environment_id.as_str())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(tenant_reservations, 1);
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
        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            UsageEstimate::default(),
            "local-estimate",
            Duration::ZERO,
        );
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_request(&request, &outcome).await.unwrap();

        assert_eq!(ledger.incomplete_requests(&context.tenant).await, 0);
    }

    #[tokio::test]
    async fn memory_ledger_persists_and_filters_operational_dimensions() {
        let ledger = EnterpriseLedger::memory();
        let request = ledger
            .begin_request_with_metadata(
                &context(),
                "gpt-test",
                true,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    request_path: "/v1/chat/completions".to_owned(),
                    traffic_class: "synthetic".to_owned(),
                    tool_use_requested: true,
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome {
            state: "completed",
            status_code: 200,
            terminal_reason: "completed".to_owned(),
            error_message: None,
            estimate: estimate(0.25),
            pricing_evidence: Some(serde_json::to_value(test_pricing_evidence()).unwrap()),
            billing_mode: "upstream-returned".to_owned(),
            chargeable: true,
            latency_ms: 1250,
            first_byte_latency_ms: Some(125),
            tool_outcome: "tool_called".to_owned(),
            tool_repair_attempted: true,
            tool_repair_recovered: true,
            retry_count: 1,
            fallback_from_provider: Some("primary".to_owned()),
        };
        ledger.finalize_request(&request, &outcome).await.unwrap();

        let page = ledger
            .list_requests(&EnterpriseLedgerQuery {
                traffic_class: Some("synthetic".to_owned()),
                ..EnterpriseLedgerQuery::default()
            })
            .await
            .unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.requests[0].request_path, "/v1/chat/completions");
        assert_eq!(page.requests[0].traffic_class, "synthetic");
        assert!(page.requests[0].tool_use_requested);
        assert_eq!(page.requests[0].latency_ms, 1250);
        assert_eq!(page.requests[0].first_byte_latency_ms, Some(125));
        assert_eq!(page.requests[0].tool_outcome, "tool_called");
        assert_eq!(page.requests[0].retry_count, 1);
        assert_eq!(
            page.requests[0].fallback_from_provider.as_deref(),
            Some("primary")
        );

        let business_page = ledger
            .list_requests(&EnterpriseLedgerQuery {
                traffic_class: Some("business".to_owned()),
                ..EnterpriseLedgerQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(business_page.total, 0);
    }

    #[tokio::test]
    async fn current_operational_ledger_is_the_single_usage_and_audit_source() {
        let ledger = EnterpriseLedger::memory();
        let request = ledger
            .begin_request_with_metadata(
                &context(),
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    username: "alice".to_owned(),
                    api_key_id: Some("key_current".to_owned()),
                    quota_subject_id: Some("subject_current".to_owned()),
                    api_key_name: Some("production".to_owned()),
                    api_key_group: Some("core".to_owned()),
                    team_id: Some("team_current".to_owned()),
                    team_name: Some("Core".to_owned()),
                    client_ip: Some("198.51.100.10".to_owned()),
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap();
        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_current"),
                "openai",
                "gpt-test-2026",
                "openai-compat",
                UsageEstimate::default(),
            )
            .await
            .unwrap();
        let usage = UsageEventInput {
            request_id: Some("req_ledger_test".to_owned()),
            attempt_id: Some("att_current".to_owned()),
            resolved_model: "gpt-test-2026".to_owned(),
            provider: "openai".to_owned(),
            protocol: "openai-compat".to_owned(),
            tool_use_requested: false,
            tool_outcome: "not_requested".to_owned(),
            traffic_class: "business".to_owned(),
            tool_repair_attempted: false,
            tool_repair_recovered: false,
            success: true,
            timed_out: false,
            status_code: 200,
            terminal_reason: "completed".to_owned(),
            estimate: estimate(0.25),
            model_pricing: None,
            pricing_evidence: Some(test_pricing_evidence()),
            billing_mode: "upstream-returned".to_owned(),
            chargeable: true,
            latency: Duration::from_millis(40),
            first_byte_latency: Some(Duration::from_millis(10)),
            retry_count: 0,
            fallback_from_provider: None,
            error_message: None,
        };
        let outcome = LedgerOutcome::from_usage(&usage);
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger
            .finalize_request_usage(&request, &usage)
            .await
            .unwrap();

        let rows = ledger.usage_rows().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["apiKeyName"], "production");
        assert_eq!(rows[0]["teamId"], "team_current");
        assert_eq!(rows[0]["clientIp"], "198.51.100.10");
        assert_eq!(rows[0]["provider"], "openai");
        for removed_field in [
            "tokenName",
            "group",
            "channelId",
            "channelName",
            "requestType",
            "detail",
        ] {
            assert!(rows[0].get(removed_field).is_none());
        }

        let management = ledger.management_usage().await.unwrap();
        assert_eq!(management.api_keys["key_current"].requests_today, 1);
        assert_eq!(management.api_keys["key_current"].tokens_today, 120);
        assert_eq!(management.teams["team_current"].requests_today, 1);
        assert_eq!(management.teams["team_current"].daily_spend_usd, 0.25);
        assert_eq!(management.users_24h["usr_test"], 1);

        let policy = UsagePolicySnapshot {
            user_id: "usr_test".to_owned(),
            username: "alice".to_owned(),
            api_key_id: Some("key_replacement".to_owned()),
            quota_subject_id: Some("subject_current".to_owned()),
            quota_subject_aliases: vec!["subject_current".to_owned()],
            team_id: Some("team_current".to_owned()),
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.25,
                ..ApiKeyPolicy::default()
            },
            quotas: vec![UsageQuotaLimit {
                id: "quota_current".to_owned(),
                user_id: "usr_test".to_owned(),
                quota_type: "requests".to_owned(),
                limit: 1.0,
                period_start_ms: 0,
            }],
        };
        assert!(matches!(
            ledger
                .check_usage_policy(&policy, estimate(0.01), true)
                .await,
            Err(AppError::QuotaExceeded(_))
        ));

        ledger
            .record_audit_event(&AuditEventInput {
                activity_type: "config_change".to_owned(),
                actor_id: "usr_admin".to_owned(),
                actor_name: "admin".to_owned(),
                target: "provider:openai".to_owned(),
                message: "更新 Provider".to_owned(),
                severity: "info".to_owned(),
            })
            .await
            .unwrap();
        let (events, total) = ledger.audit_events(10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["target"], "provider:openai");
    }

    #[tokio::test]
    async fn memory_spend_aggregation_accepts_multi_hop_rotation_aliases() {
        let ledger = EnterpriseLedger::memory();
        let request = ledger
            .begin_request_with_metadata(
                &context(),
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    api_key_id: Some("key_a".to_owned()),
                    // This is the shape produced by the 0009 backfill for
                    // requests completed before stable subjects existed.
                    quota_subject_id: Some("key_a".to_owned()),
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap();
        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_rotation_alias"),
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.25),
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            estimate(0.25),
            "upstream-returned",
            Duration::ZERO,
        );
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_request(&request, &outcome).await.unwrap();

        let root_subject = quota_subject_for_seed("key_a");
        let policy = UsagePolicySnapshot {
            user_id: "usr_test".to_owned(),
            username: "alice".to_owned(),
            api_key_id: Some("key_c".to_owned()),
            quota_subject_id: Some(root_subject.clone()),
            quota_subject_aliases: vec![
                "key_a".to_owned(),
                "key_b".to_owned(),
                "key_c".to_owned(),
                root_subject,
            ],
            team_id: None,
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.25,
                ..ApiKeyPolicy::default()
            },
            quotas: Vec::new(),
        };
        assert!(matches!(
            ledger
                .check_usage_policy(&policy, estimate(0.01), true)
                .await,
            Err(AppError::QuotaExceeded(_))
        ));
    }

    #[tokio::test]
    async fn postgres_operations_incident_queries_round_trip() {
        let Ok(database_url) = std::env::var("MODELPORT_TEST_DATABASE_URL") else {
            return;
        };
        let ledger = EnterpriseLedger::postgres_for_tests(&database_url)
            .await
            .unwrap();
        let LedgerBackend::Postgres(pool) = ledger.backend.as_ref() else {
            unreachable!();
        };
        sqlx::query(
            "TRUNCATE TABLE
                modelport_ops_incident_feedback,
                modelport_ops_incident_timeline,
                modelport_ops_incident_evidence,
                modelport_ops_incidents,
                modelport_ops_agent_heartbeats",
        )
        .execute(pool)
        .await
        .unwrap();
        let mut observation = OpsObservation {
            event_key: "provider:availability".to_owned(),
            detector_type: "provider_health".to_owned(),
            severity: OpsSeverity::Sev3,
            title: "provider degraded".to_owned(),
            summary: "one provider is in cooldown".to_owned(),
            affected_scope: json!({ "component": "providers" }),
            evidence: json!({ "unhealthyProviders": ["deepseek"] }),
            observed_at_ms: u64::try_from(now_millis()).unwrap(),
            active: true,
            recovery_criteria: "all providers healthy".to_owned(),
        };
        let opened = ledger
            .upsert_ops_observation(&observation, "key_agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        ledger
            .record_ops_heartbeat(&OpsHeartbeat {
                instance_id: "key_agent".to_owned(),
                agent_version: "0.1.0".to_owned(),
                mode: "read_only".to_owned(),
                rule_set_version: "ops-rules-v1".to_owned(),
                observed_at_ms: observation.observed_at_ms,
                queue_depth: 0,
                interval_seconds: 300,
                analysis_enabled: true,
                selected_model: Some("local_vllm:qwen3".to_owned()),
                model_status: "configured".to_owned(),
                model_last_success_at_ms: Some(observation.observed_at_ms),
            })
            .await
            .unwrap();
        let list = ledger.list_ops_incidents(None, 10).await.unwrap();
        assert_eq!(list.total, 1);
        assert_eq!(list.open, 1);
        assert_eq!(list.agents.len(), 1);
        assert!(list.agents[0].analysis_enabled);
        assert_eq!(
            list.agents[0].selected_model.as_deref(),
            Some("local_vllm:qwen3")
        );
        assert_eq!(list.agents[0].model_status, "configured");
        assert_eq!(
            list.agents[0].model_last_success_at_ms,
            Some(observation.observed_at_ms)
        );
        let detail = ledger.ops_incident_detail(&opened.id).await.unwrap();
        assert_eq!(detail.evidence.len(), 1);
        ledger
            .update_ops_incident_status(
                &opened.id,
                &OpsIncidentStatusUpdate {
                    status: OpsIncidentStatus::Monitoring,
                    reason: "credential recovered; observing the route".to_owned(),
                },
                "usr_admin",
                "admin",
            )
            .await
            .unwrap();
        ledger
            .record_ops_incident_feedback(
                &opened.id,
                &OpsIncidentFeedbackInput {
                    outcome: "true_positive".to_owned(),
                    root_cause_correct: Some(true),
                    recommendation_adopted: Some(true),
                    note: None,
                },
                "usr_admin",
                "admin",
            )
            .await
            .unwrap();
        observation.active = false;
        observation.observed_at_ms = observation.observed_at_ms.saturating_add(1);
        let resolved = ledger
            .upsert_ops_observation(&observation, "key_agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.status, OpsIncidentStatus::Resolved);
        let resolved_list = ledger
            .list_ops_incidents(Some(OpsIncidentStatus::Resolved), 10)
            .await
            .unwrap();
        assert_eq!(resolved_list.items.len(), 1);
        assert_eq!(resolved_list.open, 0);
    }

    #[tokio::test]
    async fn memory_ledger_rejects_invalid_operational_dimensions() {
        let ledger = EnterpriseLedger::memory();
        for metadata in [
            LedgerRequestMetadata {
                request_path: "/v1/unknown".to_owned(),
                ..LedgerRequestMetadata::default()
            },
            LedgerRequestMetadata {
                traffic_class: "unbounded-user-label".to_owned(),
                ..LedgerRequestMetadata::default()
            },
        ] {
            let result = ledger
                .begin_request_with_metadata(
                    &context(),
                    "gpt-test",
                    false,
                    None,
                    TEST_FINGERPRINT,
                    &metadata,
                )
                .await;
            assert!(matches!(result, Err(AppError::InvalidRequest(_))));
        }
    }

    #[test]
    fn provider_attempt_preserves_usage_provenance() {
        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            estimate(0.25),
            "upstream-returned",
            Duration::from_millis(42),
        );

        assert_eq!(outcome.billing_mode, "upstream-returned");
        assert_eq!(outcome.latency_ms, 42);
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
        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            UsageEstimate::default(),
            "local-estimate",
            Duration::ZERO,
        );
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
        assert!(
            EnterpriseLedgerQuery {
                traffic_class: Some("unbounded".to_owned()),
                ..EnterpriseLedgerQuery::default()
            }
            .normalized()
            .is_err()
        );
    }

    #[test]
    fn request_list_sql_uses_contiguous_bound_parameters() {
        for index in 1..=7 {
            assert!(REQUEST_COUNT_SQL.contains(&format!("${index}")));
        }
        assert!(!REQUEST_COUNT_SQL.contains("$8"));

        for index in 1..=10 {
            assert!(REQUEST_LIST_SQL.contains(&format!("${index}")));
        }
        assert!(!REQUEST_LIST_SQL.contains("$11"));
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

        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            UsageEstimate::default(),
            "local-estimate",
            Duration::ZERO,
        );
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
            .begin_request_with_metadata(
                &context,
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    tool_use_requested: true,
                    quota_subject_id: Some("qsub_expired".to_owned()),
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap();
        let usage_policy = UsagePolicySnapshot {
            user_id: "usr_test".to_owned(),
            username: "local-admin".to_owned(),
            api_key_id: None,
            quota_subject_id: Some("qsub_expired".to_owned()),
            quota_subject_aliases: vec!["qsub_expired".to_owned()],
            team_id: None,
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.75,
                ..ApiKeyPolicy::default()
            },
            quotas: Vec::new(),
        };
        ledger
            .begin_attempt_with_pricing(
                &request,
                &AttemptId::from_string("att_expired"),
                "openai",
                "gpt-test",
                "openai-compatible",
                AttemptPricingEvidence {
                    estimate: estimate(0.75),
                    verified: true,
                    usage_policy: &usage_policy,
                },
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
        let detail = ledger
            .request_detail(&request.ledger_id)
            .await
            .unwrap()
            .unwrap();
        assert!(detail.request.latency_ms >= 1);
        assert_eq!(detail.request.tool_outcome, "upstream_or_delivery_error");
        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.reserved_microunits, 0);
        assert_eq!(budget.account.settled_microunits, 0);
        assert_eq!(budget.recent_events[0].event_type, "released");
        assert_eq!(budget.recent_events[0].reserved_delta_microunits, -750_000);
        let LedgerBackend::Memory(inner) = ledger.backend.as_ref() else {
            unreachable!();
        };
        assert_eq!(
            inner.lock().unwrap().usage_reservations[&request.ledger_id].state,
            "released"
        );
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
    async fn memory_usage_hard_limits_atomically_admit_only_one_competing_request() {
        for scope in ["api-key", "team", "user"] {
            let ledger = EnterpriseLedger::memory();
            let subject = format!("qsub_atomic_{scope}");
            let team_id = (scope == "team").then_some("team_atomic");
            let (api_key_policy, quotas) = match scope {
                "api-key" => (
                    ApiKeyPolicy {
                        spend_limit_usd: 0.75,
                        ..ApiKeyPolicy::default()
                    },
                    Vec::new(),
                ),
                "team" => (
                    ApiKeyPolicy {
                        team_daily_limit_usd: 0.75,
                        ..ApiKeyPolicy::default()
                    },
                    Vec::new(),
                ),
                "user" => (
                    ApiKeyPolicy::default(),
                    vec![UsageQuotaLimit {
                        id: "quota_atomic_requests".to_owned(),
                        user_id: "usr_atomic_usage".to_owned(),
                        quota_type: "requests".to_owned(),
                        limit: 1.0,
                        period_start_ms: 0,
                    }],
                ),
                _ => unreachable!(),
            };
            let policy = usage_policy(&subject, team_id, api_key_policy, quotas);
            let request_a =
                begin_usage_request(&ledger, &format!("req_atomic_{scope}_a"), &subject, team_id)
                    .await;
            let request_b =
                begin_usage_request(&ledger, &format!("req_atomic_{scope}_b"), &subject, team_id)
                    .await;
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let first = {
                let ledger = ledger.clone();
                let barrier = barrier.clone();
                let policy = policy.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    begin_usage_attempt(
                        &ledger,
                        &request_a,
                        &format!("att_atomic_{scope}_a"),
                        &policy,
                        estimate(0.75),
                    )
                    .await
                })
            };
            let second = {
                let ledger = ledger.clone();
                let barrier = barrier.clone();
                let policy = policy.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    begin_usage_attempt(
                        &ledger,
                        &request_b,
                        &format!("att_atomic_{scope}_b"),
                        &policy,
                        estimate(0.75),
                    )
                    .await
                })
            };
            let (first, second) = tokio::join!(first, second);
            let first = first.unwrap();
            let second = second.unwrap();
            assert_ne!(first.is_ok(), second.is_ok(), "scope={scope}");
            let rejected = if first.is_err() { first } else { second };
            assert!(
                matches!(rejected, Err(AppError::QuotaExceeded(_))),
                "scope={scope}"
            );
            let LedgerBackend::Memory(inner) = ledger.backend.as_ref() else {
                unreachable!();
            };
            let inner = inner.lock().unwrap();
            assert_eq!(
                inner
                    .usage_reservations
                    .values()
                    .filter(|reservation| reservation.state == "reserved")
                    .count(),
                1,
                "scope={scope}"
            );
        }
    }

    #[tokio::test]
    async fn memory_usage_retry_does_not_duplicate_request_units() {
        let ledger = EnterpriseLedger::memory();
        let subject = "qsub_retry_lineage";
        let policy = usage_policy(
            subject,
            None,
            ApiKeyPolicy::default(),
            vec![
                UsageQuotaLimit {
                    id: "quota_retry_requests".to_owned(),
                    user_id: "usr_atomic_usage".to_owned(),
                    quota_type: "requests".to_owned(),
                    limit: 2.0,
                    period_start_ms: 0,
                },
                UsageQuotaLimit {
                    id: "quota_retry_tokens".to_owned(),
                    user_id: "usr_atomic_usage".to_owned(),
                    quota_type: "tokens".to_owned(),
                    limit: 360.0,
                    period_start_ms: 0,
                },
            ],
        );
        let first_request = begin_usage_request(&ledger, "req_retry_logical", subject, None).await;
        begin_usage_attempt(
            &ledger,
            &first_request,
            "att_retry_first",
            &policy,
            estimate(0.1),
        )
        .await
        .unwrap();
        begin_usage_attempt(
            &ledger,
            &first_request,
            "att_retry_second",
            &policy,
            estimate(0.1),
        )
        .await
        .unwrap();
        let second_request =
            begin_usage_request(&ledger, "req_retry_second_request", subject, None).await;
        begin_usage_attempt(
            &ledger,
            &second_request,
            "att_retry_third",
            &policy,
            estimate(0.1),
        )
        .await
        .unwrap();
        let third_request =
            begin_usage_request(&ledger, "req_retry_rejected_request", subject, None).await;
        assert!(matches!(
            begin_usage_attempt(
                &ledger,
                &third_request,
                "att_retry_rejected",
                &policy,
                estimate(0.1),
            )
            .await,
            Err(AppError::QuotaExceeded(_))
        ));

        let LedgerBackend::Memory(inner) = ledger.backend.as_ref() else {
            unreachable!();
        };
        let inner = inner.lock().unwrap();
        let first = &inner.usage_reservations[&first_request.ledger_id];
        assert_eq!(first.reserved_requests, 1);
        assert_eq!(first.reserved_tokens, 240);
    }

    #[tokio::test]
    async fn memory_usage_settlement_replaces_reservation_with_actual_without_double_counting() {
        let ledger = EnterpriseLedger::memory();
        let subject = "qsub_actual_settlement";
        let policy = usage_policy(
            subject,
            None,
            ApiKeyPolicy {
                spend_limit_usd: 1.0,
                ..ApiKeyPolicy::default()
            },
            Vec::new(),
        );
        let first_request =
            begin_usage_request(&ledger, "req_actual_settlement", subject, None).await;
        let first_attempt = begin_usage_attempt(
            &ledger,
            &first_request,
            "att_actual_settlement",
            &policy,
            estimate(0.75),
        )
        .await
        .unwrap();
        let actual = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            estimate(0.25),
            "upstream-returned",
            Duration::ZERO,
        );
        ledger
            .finalize_attempt(&first_attempt, &actual)
            .await
            .unwrap();
        ledger
            .finalize_request(&first_request, &actual)
            .await
            .unwrap();

        let second_request = begin_usage_request(&ledger, "req_actual_second", subject, None).await;
        begin_usage_attempt(
            &ledger,
            &second_request,
            "att_actual_second",
            &policy,
            estimate(0.75),
        )
        .await
        .unwrap();
        let exhausted_request =
            begin_usage_request(&ledger, "req_actual_exhausted", subject, None).await;
        assert!(matches!(
            begin_usage_attempt(
                &ledger,
                &exhausted_request,
                "att_actual_exhausted",
                &policy,
                estimate(0.000_001),
            )
            .await,
            Err(AppError::QuotaExceeded(_))
        ));

        let LedgerBackend::Memory(inner) = ledger.backend.as_ref() else {
            unreachable!();
        };
        let inner = inner.lock().unwrap();
        let settled = &inner.usage_reservations[&first_request.ledger_id];
        assert_eq!(settled.state, "settled");
        assert_eq!(settled.actual_cost_microunits, 250_000);
    }

    #[tokio::test]
    async fn memory_nonchargeable_terminal_releases_usage_and_tenant_reservations() {
        let ledger = EnterpriseLedger::memory();
        set_local_budget(&ledger, 750_000).await;
        let subject = "qsub_nonchargeable";
        let policy = usage_policy(
            subject,
            None,
            ApiKeyPolicy {
                spend_limit_usd: 0.75,
                ..ApiKeyPolicy::default()
            },
            Vec::new(),
        );
        let request = begin_usage_request(&ledger, "req_nonchargeable", subject, None).await;
        let attempt = begin_usage_attempt(
            &ledger,
            &request,
            "att_nonchargeable",
            &policy,
            estimate(0.75),
        )
        .await
        .unwrap();
        let mut outcome = LedgerOutcome::provider_attempt(
            false,
            503,
            None,
            estimate(0.75),
            "local-estimate",
            Duration::ZERO,
        );
        outcome.chargeable = false;
        outcome.billing_mode = "unreconciled".to_owned();
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_request(&request, &outcome).await.unwrap();

        let replacement =
            begin_usage_request(&ledger, "req_after_nonchargeable", subject, None).await;
        begin_usage_attempt(
            &ledger,
            &replacement,
            "att_after_nonchargeable",
            &policy,
            estimate(0.75),
        )
        .await
        .unwrap();
        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.reserved_microunits, 750_000);
        assert_eq!(budget.account.settled_microunits, 0);
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
        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            estimate(0.625_123),
            "local-estimate",
            Duration::ZERO,
        );

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
    async fn estimate_only_attempt_releases_reservation_without_settlement() {
        let ledger = EnterpriseLedger::memory();
        set_local_budget(&ledger, 1_000_000).await;
        let request = ledger
            .begin_request(&context(), "gpt-test", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        let attempt = ledger
            .begin_attempt(
                &request,
                &AttemptId::from_string("att_estimate_only"),
                "unverified-proxy",
                "gpt-test",
                "openai-compatible",
                estimate(0.75),
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome::provider_attempt(
            true,
            200,
            None,
            unreconciled_estimate(0.625),
            "upstream-returned",
            Duration::ZERO,
        );

        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();

        let budget = ledger
            .budget_view(&EnterpriseBudgetScopeQuery::default())
            .await
            .unwrap();
        assert_eq!(budget.account.reserved_microunits, 0);
        assert_eq!(budget.account.settled_microunits, 0);
        assert_eq!(budget.recent_events[0].event_type, "released");
        assert_eq!(
            budget.recent_events[0].evidence_source,
            "provider-usage-unpriced"
        );
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

    #[tokio::test]
    async fn retention_is_previewable_legal_hold_safe_idempotent_and_keeps_budget_evidence() {
        let ledger = EnterpriseLedger::memory();
        let context = context();
        let request = ledger
            .begin_request_with_metadata(
                &context,
                "gpt-test",
                false,
                None,
                TEST_FINGERPRINT,
                &LedgerRequestMetadata {
                    api_key_id: Some("key_retention".to_owned()),
                    quota_subject_id: Some("key_retention".to_owned()),
                    ..LedgerRequestMetadata::default()
                },
            )
            .await
            .unwrap();
        let attempt_id = AttemptId::from_string("att_retention_evidence");
        let attempt = ledger
            .begin_attempt(
                &request,
                &attempt_id,
                "openai",
                "gpt-test",
                "openai-compatible",
                estimate(0.25),
            )
            .await
            .unwrap();
        let outcome = LedgerOutcome::provider_attempt(
            false,
            500,
            Some("secret provider body".to_owned()),
            estimate(0.2),
            "upstream-returned",
            Duration::ZERO,
        );
        ledger.finalize_attempt(&attempt, &outcome).await.unwrap();
        ledger.finalize_request(&request, &outcome).await.unwrap();
        ledger
            .record_audit_event(&AuditEventInput {
                activity_type: "old_event".to_owned(),
                actor_id: "usr_test".to_owned(),
                actor_name: "tester".to_owned(),
                target: "request".to_owned(),
                message: "old event".to_owned(),
                severity: "info".to_owned(),
            })
            .await
            .unwrap();

        let LedgerBackend::Memory(inner) = ledger.backend.as_ref() else {
            unreachable!();
        };
        let old = now_millis().saturating_sub(10 * MILLIS_PER_DAY as i64);
        {
            let mut inner = inner.lock().unwrap();
            let request_row = inner.requests.get_mut(&request.ledger_id).unwrap();
            request_row.record.created_at_ms = old;
            request_row.last_attempt_id = Some(attempt_id.to_string());
            request_row.provider_id = Some("openai".to_owned());
            request_row.resolved_model = Some("gpt-test".to_owned());
            request_row.provider_protocol = Some("openai-compatible".to_owned());
            let attempt_row = inner.attempts.get_mut(attempt_id.as_str()).unwrap();
            attempt_row.created_at_ms = old;
            attempt_row.error_message = Some("secret provider body".to_owned());
            for (ledger_id, state) in [
                ("retention-settled", "settled"),
                ("retention-open", "reserved"),
            ] {
                inner.usage_reservations.insert(
                    ledger_id.to_owned(),
                    MemoryUsageReservation {
                        reservation_id: format!("urs_{ledger_id}"),
                        quota_subject_id: Some("key_usage_retention".to_owned()),
                        team_id: Some("team_usage_retention".to_owned()),
                        user_id: "usr_usage_retention".to_owned(),
                        reserved_requests: 1,
                        reserved_tokens: 120,
                        reserved_cost_microunits: 200_000,
                        actual_requests: u64::from(state == "settled"),
                        actual_tokens: u64::from(state == "settled") * 100,
                        actual_cost_microunits: i64::from(state == "settled") * 150_000,
                        state: state.to_owned(),
                        evidence_source: Some("upstream-returned".to_owned()),
                        billing_mode: Some("upstream-returned".to_owned()),
                        created_at_ms: old,
                        updated_at_ms: old,
                        terminal_at_ms: (state == "settled").then_some(old),
                    },
                );
            }
            inner.audit_events[0].timestamp = old.to_string();
        }
        let policy = RetentionPolicy {
            request_detail_days: 1,
            user_usage_days: 2,
            audit_days: 3,
            legal_hold: false,
            content_persistence: false,
        };

        let preview = ledger.run_retention(policy, true).await.unwrap();
        assert!(!preview.applied);
        assert_eq!(preview.counts.request_details_redacted, 1);
        assert_eq!(preview.counts.provider_attempts_redacted, 1);
        assert_eq!(preview.counts.user_usage_rows_deidentified, 2);
        assert_eq!(preview.counts.audit_events_deleted, 1);
        {
            let inner = inner.lock().unwrap();
            assert_eq!(
                inner.requests[&request.ledger_id].request_id,
                "req_ledger_test"
            );
            assert!(inner.attempts.contains_key(attempt_id.as_str()));
        }

        let held = ledger
            .run_retention(
                RetentionPolicy {
                    legal_hold: true,
                    ..policy
                },
                false,
            )
            .await
            .unwrap();
        assert!(!held.applied);
        assert_eq!(held.skipped_reason, Some("legal_hold"));

        let applied = ledger.run_retention(policy, false).await.unwrap();
        assert!(applied.applied);
        assert_eq!(applied.counts.provider_attempts_redacted, 1);
        {
            let inner = inner.lock().unwrap();
            let request_row = &inner.requests[&request.ledger_id];
            assert!(
                request_row
                    .request_id
                    .starts_with(RETAINED_REQUEST_ID_PREFIX)
            );
            assert_eq!(
                request_row.request_fingerprint,
                retained_request_fingerprint(&request.ledger_id)
            );
            assert_eq!(request_row.request_fingerprint.len(), 64);
            assert_ne!(request_row.request_fingerprint, TEST_FINGERPRINT);
            assert_eq!(request_row.principal_id, RETAINED_PRINCIPAL_ID);
            assert_eq!(
                request_row.quota_subject_id.as_deref(),
                Some(quota_subject_for_seed("key_retention").as_str())
            );
            assert_eq!(
                request_row.last_attempt_id.as_deref(),
                Some(attempt_id.as_str())
            );
            let attempt_row = &inner.attempts[attempt_id.as_str()];
            assert_eq!(
                attempt_row.terminal_reason.as_deref(),
                Some("retained_financial_evidence")
            );
            assert!(attempt_row.error_message.is_none());
            assert!(inner.budget_reservations.contains_key(attempt_id.as_str()));
            assert_eq!(inner.budget_events.len(), 2);
            assert!(inner.audit_events.is_empty());
            let retained_reservation = &inner.usage_reservations["retention-settled"];
            assert_eq!(retained_reservation.user_id, RETAINED_PRINCIPAL_ID);
            assert!(retained_reservation.team_id.is_none());
            assert_eq!(
                retained_reservation.quota_subject_id.as_deref(),
                Some(quota_subject_for_seed("key_usage_retention").as_str())
            );
            let open_reservation = &inner.usage_reservations["retention-open"];
            assert_eq!(open_reservation.user_id, "usr_usage_retention");
            assert_eq!(
                open_reservation.team_id.as_deref(),
                Some("team_usage_retention")
            );
        }

        let retained_subject = quota_subject_for_seed("key_retention");
        let all_time_policy = UsagePolicySnapshot {
            user_id: "usr_test".to_owned(),
            username: "tester".to_owned(),
            api_key_id: Some("key_replacement".to_owned()),
            quota_subject_id: Some(retained_subject.clone()),
            quota_subject_aliases: vec!["key_retention".to_owned(), retained_subject.clone()],
            team_id: None,
            api_key_policy: ApiKeyPolicy {
                spend_limit_usd: 0.2,
                ..ApiKeyPolicy::default()
            },
            quotas: Vec::new(),
        };
        assert!(matches!(
            ledger
                .check_usage_policy(&all_time_policy, estimate(0.01), true)
                .await,
            Err(AppError::QuotaExceeded(_))
        ));
        let window_policy = UsagePolicySnapshot {
            api_key_policy: ApiKeyPolicy {
                rate_limited: true,
                five_hour_limit_usd: 0.2,
                ..ApiKeyPolicy::default()
            },
            ..all_time_policy
        };
        assert!(
            ledger
                .check_usage_policy(&window_policy, estimate(0.01), true)
                .await
                .is_ok()
        );

        let repeated = ledger.run_retention(policy, false).await.unwrap();
        assert_eq!(repeated.counts, RetentionCounts::default());
    }

    #[tokio::test]
    async fn amount_budget_rejects_unverified_fallback_pricing_before_reservation() {
        let ledger = EnterpriseLedger::memory();
        set_local_budget(&ledger, 1_000_000).await;
        let request = ledger
            .begin_request(&context(), "unknown-model", false, None, TEST_FINGERPRINT)
            .await
            .unwrap();
        let usage_policy = UsagePolicySnapshot::default();
        let result = ledger
            .begin_attempt_with_pricing(
                &request,
                &AttemptId::from_string("att_unverified_pricing"),
                "custom-cloud",
                "unknown-model",
                "openai-compatible",
                AttemptPricingEvidence {
                    estimate: estimate(0.1),
                    verified: false,
                    usage_policy: &usage_policy,
                },
            )
            .await;
        assert!(matches!(result, Err(AppError::PricingUnverified(_))));
        let LedgerBackend::Memory(inner) = ledger.backend.as_ref() else {
            unreachable!();
        };
        let inner = inner.lock().unwrap();
        assert!(inner.attempts.is_empty());
        assert!(inner.budget_reservations.is_empty());
        assert!(inner.budget_events.is_empty());
    }

    #[tokio::test]
    async fn operations_incident_lifecycle_is_deduplicated_and_evidence_driven() {
        let ledger = EnterpriseLedger::memory();
        let observed_at_ms = u64::try_from(now_millis()).unwrap();
        let mut observation = OpsObservation {
            event_key: "readiness:gateway".to_owned(),
            detector_type: "readiness_storage".to_owned(),
            severity: OpsSeverity::Sev1,
            title: "gateway unavailable".to_owned(),
            summary: "database is not ready".to_owned(),
            affected_scope: json!({ "component": "gateway" }),
            evidence: json!({ "databaseReady": false }),
            observed_at_ms,
            active: false,
            recovery_criteria: "all dependencies ready".to_owned(),
        };
        assert!(
            ledger
                .upsert_ops_observation(&observation, "agent", "ops-agent")
                .await
                .unwrap()
                .is_none()
        );

        observation.active = true;
        let opened = ledger
            .upsert_ops_observation(&observation, "agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(opened.status, OpsIncidentStatus::Open);
        let repeated = ledger
            .upsert_ops_observation(&observation, "agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repeated.id, opened.id);
        let detail = ledger.ops_incident_detail(&opened.id).await.unwrap();
        assert_eq!(detail.evidence.len(), 1);
        assert_eq!(detail.incident.occurrence_count, 1);

        ledger
            .update_ops_incident_status(
                &opened.id,
                &OpsIncidentStatusUpdate {
                    status: OpsIncidentStatus::Acknowledged,
                    reason: "operator is investigating".to_owned(),
                },
                "usr_admin",
                "admin",
            )
            .await
            .unwrap();
        observation.active = false;
        observation.observed_at_ms = observation.observed_at_ms.saturating_add(1);
        let resolved = ledger
            .upsert_ops_observation(&observation, "agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.status, OpsIncidentStatus::Resolved);

        observation.active = true;
        observation.observed_at_ms = observation.observed_at_ms.saturating_add(1);
        let reopened = ledger
            .upsert_ops_observation(&observation, "agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reopened.status, OpsIncidentStatus::Open);
        assert_eq!(reopened.occurrence_count, 2);
        ledger
            .record_ops_heartbeat(&OpsHeartbeat {
                instance_id: "modelport-test".to_owned(),
                agent_version: "0.1.0".to_owned(),
                mode: "read_only".to_owned(),
                rule_set_version: "ops-rules-v1".to_owned(),
                observed_at_ms: u64::try_from(now_millis()).unwrap(),
                queue_depth: 2,
                interval_seconds: 300,
                analysis_enabled: true,
                selected_model: Some("local_vllm:qwen".to_owned()),
                model_status: "configured".to_owned(),
                model_last_success_at_ms: Some(u64::try_from(now_millis()).unwrap()),
            })
            .await
            .unwrap();
        let list = ledger.list_ops_incidents(None, 10).await.unwrap();
        assert_eq!(list.total, 1);
        assert_eq!(list.open, 1);
        assert_eq!(list.highest_open_severity, Some(OpsSeverity::Sev1));
        assert_eq!(list.agents.len(), 1);
        assert!(list.agents[0].online);
        assert_eq!(list.agents[0].queue_depth, 2);
    }

    #[tokio::test]
    async fn operations_incident_rejects_manual_resolution_and_records_feedback() {
        let ledger = EnterpriseLedger::memory();
        let observation = OpsObservation {
            event_key: "budget:capacity".to_owned(),
            detector_type: "budget_quota".to_owned(),
            severity: OpsSeverity::Sev3,
            title: "budget warning".to_owned(),
            summary: "one budget is above warning threshold".to_owned(),
            affected_scope: json!({ "component": "budget" }),
            evidence: json!({ "warningAccounts": 1 }),
            observed_at_ms: u64::try_from(now_millis()).unwrap(),
            active: true,
            recovery_criteria: "no account above warning threshold".to_owned(),
        };
        let mut untrusted = observation.clone();
        untrusted.event_key = "custom:run-shell".to_owned();
        assert!(matches!(
            ledger
                .upsert_ops_observation(&untrusted, "agent", "ops-agent")
                .await,
            Err(AppError::Forbidden(_))
        ));
        let incident = ledger
            .upsert_ops_observation(&observation, "agent", "ops-agent")
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            ledger
                .update_ops_incident_status(
                    &incident.id,
                    &OpsIncidentStatusUpdate {
                        status: OpsIncidentStatus::Resolved,
                        reason: "looks fixed".to_owned(),
                    },
                    "usr_admin",
                    "admin",
                )
                .await,
            Err(AppError::InvalidRequest(_))
        ));
        ledger
            .record_ops_incident_feedback(
                &incident.id,
                &OpsIncidentFeedbackInput {
                    outcome: "true_positive".to_owned(),
                    root_cause_correct: Some(true),
                    recommendation_adopted: None,
                    note: Some("useful detector".to_owned()),
                },
                "usr_admin",
                "admin",
            )
            .await
            .unwrap();
        let detail = ledger.ops_incident_detail(&incident.id).await.unwrap();
        assert!(
            detail
                .timeline
                .iter()
                .any(|entry| entry.event_type == "feedback")
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
