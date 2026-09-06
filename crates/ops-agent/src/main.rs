mod rules;

use std::{env, net::SocketAddr, path::Path, str::FromStr, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use axum::{Json, Router, extract::State, routing::get};
use modelport_ops_protocol::{OpsAgentConfiguration, OpsHeartbeat, OpsObservation, OpsSnapshot};
use reqwest::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_QUEUE_DEPTH: i64 = 10_000;

#[derive(Clone)]
struct Config {
    base_url: String,
    api_key: String,
    mode: String,
    interval: Duration,
    bind: SocketAddr,
    spool_path: String,
    webhook_url: Option<String>,
    model_api_key: String,
}

impl Config {
    fn from_env() -> Result<Self> {
        let mode = value("MODELPORT_OPS_MODE", "disabled");
        if !matches!(
            mode.as_str(),
            "disabled" | "replay" | "shadow" | "read_only"
        ) {
            bail!("MODELPORT_OPS_MODE must be disabled, replay, shadow, or read_only");
        }
        let interval_seconds = value("MODELPORT_OPS_INTERVAL_SECONDS", "300")
            .parse::<u64>()
            .context("MODELPORT_OPS_INTERVAL_SECONDS must be an integer")?
            .clamp(10, 3_600);
        let api_key = env::var("MODELPORT_OPS_API_KEY").unwrap_or_default();
        if mode != "disabled" && api_key.trim().is_empty() {
            bail!("MODELPORT_OPS_API_KEY is required unless the agent is disabled");
        }
        Ok(Self {
            base_url: value("MODELPORT_OPS_BASE_URL", "http://modelport:38082")
                .trim_end_matches('/')
                .to_owned(),
            api_key,
            mode,
            interval: Duration::from_secs(interval_seconds),
            bind: value("MODELPORT_OPS_BIND", "0.0.0.0:38083")
                .parse()
                .context("MODELPORT_OPS_BIND must be a socket address")?,
            spool_path: value(
                "MODELPORT_OPS_SPOOL_PATH",
                "/var/lib/modelport-ops/spool.sqlite",
            ),
            webhook_url: env::var("MODELPORT_OPS_WEBHOOK_URL")
                .ok()
                .filter(|url| !url.trim().is_empty()),
            model_api_key: env::var("MODELPORT_OPS_MODEL_API_KEY").unwrap_or_default(),
        })
    }
}

#[derive(Debug, Default)]
struct RuntimeStatus {
    last_success_at_ms: Option<u64>,
    last_error: Option<String>,
    queue_depth: u64,
    model_status: String,
    model_last_success_at_ms: Option<u64>,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    status: Arc<RwLock<RuntimeStatus>>,
    spool: SqlitePool,
    client: Client,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "modelport_ops_agent=info".into()),
        )
        .init();
    let config = Arc::new(Config::from_env()?);
    let spool = open_spool(&config.spool_path).await?;
    let state = AppState {
        config: config.clone(),
        status: Arc::new(RwLock::new(RuntimeStatus::default())),
        spool,
        client: Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    };
    let worker_state = state.clone();
    tokio::spawn(async move {
        run_loop(worker_state).await;
    });
    let app = Router::new()
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    info!(bind = %config.bind, mode = %config.mode, "ModelPort operations agent started");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn open_spool(path: &str) -> Result<SqlitePool> {
    if let Some(parent) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let options =
        SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?.create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS observation_queue (
            queue_id TEXT PRIMARY KEY,
            body TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0
         )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS model_analysis_cache (
            cache_key TEXT PRIMARY KEY,
            model TEXT NOT NULL,
            analysis TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
         )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS webhook_deliveries (
            signature TEXT PRIMARY KEY,
            delivered_at_ms INTEGER NOT NULL
         )",
    )
    .execute(&pool)
    .await?;
    Ok(pool)
}

async fn run_loop(state: AppState) {
    if state.config.mode == "disabled" {
        info!("operations agent disabled by configuration");
        return;
    }
    let mut interval = tokio::time::interval(state.config.interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(error) = run_cycle(&state).await {
            error!(error = %error, "operations agent cycle failed");
            let mut status = state.status.write().await;
            status.last_error = Some(bounded_error(&error));
        }
    }
}

async fn run_cycle(state: &AppState) -> Result<()> {
    let snapshot = state
        .client
        .get(format!(
            "{}/internal/ops/v1/snapshot",
            state.config.base_url
        ))
        .bearer_auth(&state.config.api_key)
        .send()
        .await?
        .error_for_status()?
        .json::<OpsSnapshot>()
        .await?;
    if !snapshot.agent_configuration.enabled {
        info!("operations agent is awaiting explicit administrator enablement");
        return send_heartbeat(state, &snapshot.agent_configuration, "disabled").await;
    }
    let mut observations = rules::evaluate(&snapshot);
    let active = observations.iter().filter(|item| item.active).count();
    info!(mode = %state.config.mode, active, "evaluated deterministic operations rules");
    if state.config.mode == "read_only" {
        for observation in &mut observations {
            if observation.active {
                attach_model_analysis(state, &snapshot.agent_configuration, observation).await;
            }
        }
        for observation in observations {
            enqueue(&state.spool, &observation).await?;
        }
        trim_queue(&state.spool).await?;
        flush_queue(state).await?;
    }
    send_heartbeat(state, &snapshot.agent_configuration, &state.config.mode).await
}

async fn send_heartbeat(
    state: &AppState,
    configuration: &OpsAgentConfiguration,
    effective_mode: &str,
) -> Result<()> {
    let queue_depth = queue_depth(&state.spool).await?;
    let (model_status, model_last_success_at_ms) = {
        let status = state.status.read().await;
        let model_status = if !configuration.analysis_enabled {
            "disabled".to_owned()
        } else if !configuration.model_ready {
            "error".to_owned()
        } else if state.config.model_api_key.trim().is_empty() {
            "missing_credential".to_owned()
        } else if status.model_status.is_empty() || status.model_status == "disabled" {
            "configured".to_owned()
        } else {
            status.model_status.clone()
        };
        (model_status, status.model_last_success_at_ms)
    };
    let heartbeat = OpsHeartbeat {
        // The server replaces this hint with the authenticated API-key ID so
        // one compromised Agent key cannot create unbounded heartbeat rows.
        instance_id: "self".to_owned(),
        agent_version: VERSION.to_owned(),
        mode: effective_mode.to_owned(),
        rule_set_version: rules::RULE_SET_VERSION.to_owned(),
        observed_at_ms: now_millis(),
        queue_depth,
        interval_seconds: state.config.interval.as_secs(),
        analysis_enabled: configuration.analysis_enabled,
        selected_model: configuration.selected_model.clone(),
        model_status,
        model_last_success_at_ms,
    };
    state
        .client
        .post(format!(
            "{}/internal/ops/v1/heartbeats",
            state.config.base_url
        ))
        .bearer_auth(&state.config.api_key)
        .json(&heartbeat)
        .send()
        .await?
        .error_for_status()?;
    let mut status = state.status.write().await;
    status.last_success_at_ms = Some(now_millis());
    status.last_error = None;
    status.queue_depth = queue_depth;
    Ok(())
}

async fn attach_model_analysis(
    state: &AppState,
    configuration: &OpsAgentConfiguration,
    observation: &mut OpsObservation,
) {
    if !configuration.analysis_enabled || !configuration.model_ready {
        return;
    }
    let Some(model) = configuration.selected_model.as_deref() else {
        return;
    };
    if state.config.model_api_key.trim().is_empty() {
        let mut status = state.status.write().await;
        status.model_status = "missing_credential".to_owned();
        return;
    }
    let cache_key = match model_analysis_cache_key(model, observation) {
        Ok(value) => value,
        Err(error) => {
            warn!(error = %error, "failed to calculate operations analysis cache key");
            return;
        }
    };
    let cached = sqlx::query_scalar::<_, String>(
        "SELECT analysis FROM model_analysis_cache WHERE cache_key = ?1",
    )
    .bind(&cache_key)
    .fetch_optional(&state.spool)
    .await;
    let analysis = match cached {
        Ok(Some(value)) => Some(value),
        Ok(None) => match request_model_analysis(state, model, observation).await {
            Ok(value) => {
                if let Err(error) = sqlx::query(
                    "INSERT OR REPLACE INTO model_analysis_cache
                        (cache_key, model, analysis, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
                )
                .bind(&cache_key)
                .bind(model)
                .bind(&value)
                .bind(i64::try_from(now_millis()).unwrap_or(i64::MAX))
                .execute(&state.spool)
                .await
                {
                    warn!(error = %error, "failed to cache operations model analysis");
                }
                let _ = sqlx::query(
                    "DELETE FROM model_analysis_cache WHERE cache_key IN (
                        SELECT cache_key FROM model_analysis_cache
                        ORDER BY created_at_ms DESC LIMIT -1 OFFSET 1000
                     )",
                )
                .execute(&state.spool)
                .await;
                let mut status = state.status.write().await;
                status.model_status = "configured".to_owned();
                status.model_last_success_at_ms = Some(now_millis());
                Some(value)
            }
            Err(error) => {
                warn!(error = %error, model, "optional operations model analysis failed");
                let mut status = state.status.write().await;
                status.model_status = "error".to_owned();
                None
            }
        },
        Err(error) => {
            warn!(error = %error, "failed to read operations analysis cache");
            None
        }
    };
    let Some(analysis) = analysis else {
        return;
    };
    if let Some(evidence) = observation.evidence.as_object_mut() {
        evidence.insert(
            "modelAnalysis".to_owned(),
            json!({
                "advisoryOnly": true,
                "model": model,
                "local": configuration.selected_model_local,
                "content": analysis,
            }),
        );
    }
}

fn model_analysis_cache_key(model: &str, observation: &OpsObservation) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(observation.event_key.as_bytes());
    hasher.update(observation.detector_type.as_bytes());
    hasher.update(observation.severity.as_str().as_bytes());
    hasher.update(serde_json::to_vec(&observation.affected_scope)?);
    hasher.update(serde_json::to_vec(&observation.evidence)?);
    Ok(format!("oma_{:x}", hasher.finalize()))
}

async fn request_model_analysis(
    state: &AppState,
    model: &str,
    observation: &OpsObservation,
) -> Result<String> {
    let facts = json!({
        "eventKey": observation.event_key,
        "detectorType": observation.detector_type,
        "severity": observation.severity,
        "summary": observation.summary,
        "affectedScope": observation.affected_scope,
        "evidence": observation.evidence,
        "recoveryCriteria": observation.recovery_criteria,
    });
    let response = state
        .client
        .post(format!("{}/v1/chat/completions", state.config.base_url))
        .bearer_auth(&state.config.model_api_key)
        .json(&json!({
            "model": model,
            "stream": false,
            "temperature": 0,
            "max_tokens": 600,
            "messages": [
                {
                    "role": "system",
                    "content": "你是 ModelPort 只读运维诊断助手。只能依据给定的脱敏事实回答，不得臆测密钥、提示词或用户内容，不得声称已执行操作。用简洁中文给出：可能原因、验证步骤、建议动作和风险；明确区分事实与推断。"
                },
                {
                    "role": "user",
                    "content": serde_json::to_string(&facts)?
                }
            ]
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("model response did not contain assistant text")?;
    Ok(content
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .take(4_000)
        .collect())
}

async fn enqueue(pool: &SqlitePool, observation: &OpsObservation) -> Result<()> {
    let body = serde_json::to_string(observation)?;
    let mut hasher = Sha256::new();
    hasher.update(observation.event_key.as_bytes());
    hasher.update(observation.detector_type.as_bytes());
    hasher.update(observation.severity.as_str().as_bytes());
    hasher.update([u8::from(observation.active)]);
    hasher.update(serde_json::to_vec(&observation.evidence)?);
    let queue_id = format!("opq_{:x}", hasher.finalize());
    sqlx::query(
        "INSERT OR IGNORE INTO observation_queue (queue_id, body, created_at_ms)
         VALUES (?1, ?2, ?3)",
    )
    .bind(queue_id)
    .bind(body)
    .bind(i64::try_from(now_millis()).unwrap_or(i64::MAX))
    .execute(pool)
    .await?;
    Ok(())
}

async fn flush_queue(state: &AppState) -> Result<()> {
    let rows = sqlx::query(
        "SELECT queue_id, body FROM observation_queue ORDER BY created_at_ms LIMIT 100",
    )
    .fetch_all(&state.spool)
    .await?;
    for row in rows {
        let queue_id: String = row.try_get("queue_id")?;
        let body: String = row.try_get("body")?;
        let observation: OpsObservation = serde_json::from_str(&body)?;
        let response = state
            .client
            .post(format!(
                "{}/internal/ops/v1/observations",
                state.config.base_url
            ))
            .bearer_auth(&state.config.api_key)
            .json(&observation)
            .send()
            .await?;
        let status = response.status();
        if status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS {
            warn!(%status, %queue_id, "dropping permanently rejected operations observation");
            sqlx::query("DELETE FROM observation_queue WHERE queue_id = ?1")
                .bind(&queue_id)
                .execute(&state.spool)
                .await?;
            continue;
        }
        if !status.is_success() {
            sqlx::query("UPDATE observation_queue SET attempts = attempts + 1 WHERE queue_id = ?1")
                .bind(&queue_id)
                .execute(&state.spool)
                .await?;
            bail!("observation API returned {status}");
        }
        let accepted: Value = response.json().await.unwrap_or_else(|_| json!({}));
        sqlx::query("DELETE FROM observation_queue WHERE queue_id = ?1")
            .bind(&queue_id)
            .execute(&state.spool)
            .await?;
        if observation.active {
            send_webhook_once(state, &observation, &accepted).await;
        }
    }
    Ok(())
}

async fn send_webhook_once(state: &AppState, observation: &OpsObservation, accepted: &Value) {
    let Some(url) = state.config.webhook_url.as_deref() else {
        return;
    };
    let Some(incident) = accepted.get("incident") else {
        return;
    };
    let signature = format!(
        "{}:{}:{}",
        incident
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        incident
            .get("occurrenceCount")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        observation.severity.as_str(),
    );
    if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM webhook_deliveries WHERE signature = ?1")
        .bind(&signature)
        .fetch_one(&state.spool)
        .await
        .is_ok_and(|count| count > 0)
    {
        return;
    }
    let payload = json!({
        "schemaVersion": "modelport.ops.webhook.v1",
        "eventKey": observation.event_key,
        "severity": observation.severity,
        "title": observation.title,
        "summary": observation.summary,
        "affectedScope": observation.affected_scope,
        "incident": accepted.get("incident"),
    });
    let delivery = state
        .client
        .post(url)
        .json(&payload)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    if let Err(error) = delivery {
        warn!(error = %error, "operations webhook delivery failed");
        return;
    }
    if let Err(error) = sqlx::query(
        "INSERT OR IGNORE INTO webhook_deliveries (signature, delivered_at_ms)
         VALUES (?1, ?2)",
    )
    .bind(signature)
    .bind(i64::try_from(now_millis()).unwrap_or(i64::MAX))
    .execute(&state.spool)
    .await
    {
        warn!(error = %error, "failed to record operations webhook delivery");
    }
}

async fn trim_queue(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "DELETE FROM observation_queue WHERE queue_id IN (
            SELECT queue_id FROM observation_queue
            ORDER BY created_at_ms DESC LIMIT -1 OFFSET ?1
         )",
    )
    .bind(MAX_QUEUE_DEPTH)
    .execute(pool)
    .await?;
    Ok(())
}

async fn queue_depth(pool: &SqlitePool) -> Result<u64> {
    let count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM observation_queue")
        .fetch_one(pool)
        .await?;
    Ok(u64::try_from(count).unwrap_or_default())
}

async fn livez() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "modelport-ops-agent",
        "version": VERSION,
    }))
}

async fn readyz(State(state): State<AppState>) -> (axum::http::StatusCode, Json<Value>) {
    if state.config.mode == "disabled" {
        return (
            axum::http::StatusCode::OK,
            Json(json!({ "status": "disabled" })),
        );
    }
    let status = state.status.read().await;
    let fresh = status.last_success_at_ms.is_some_and(|timestamp| {
        now_millis().saturating_sub(timestamp)
            <= u64::try_from(state.config.interval.as_millis())
                .unwrap_or(u64::MAX)
                .saturating_mul(3)
    });
    let code = if fresh {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": if fresh { "ready" } else { "not_ready" },
            "mode": state.config.mode,
            "lastSuccessAtMs": status.last_success_at_ms,
            "lastError": status.last_error,
            "queueDepth": status.queue_depth,
            "ruleSetVersion": rules::RULE_SET_VERSION,
        })),
    )
}

fn value(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn bounded_error(error: &anyhow::Error) -> String {
    error.to_string().chars().take(500).collect()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_spool_deduplicates_identical_observation() {
        let path = std::env::temp_dir().join(format!(
            "modelport-ops-spool-{}-{}.sqlite",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let pool = open_spool(path.to_str().unwrap()).await.unwrap();
        let observation = OpsObservation {
            event_key: "test:event".to_owned(),
            detector_type: "test".to_owned(),
            severity: modelport_ops_protocol::OpsSeverity::Sev4,
            title: "test".to_owned(),
            summary: "test".to_owned(),
            affected_scope: json!({}),
            evidence: json!({ "value": 1 }),
            observed_at_ms: now_millis(),
            active: true,
            recovery_criteria: "test".to_owned(),
        };
        enqueue(&pool, &observation).await.unwrap();
        enqueue(&pool, &observation).await.unwrap();
        assert_eq!(queue_depth(&pool).await.unwrap(), 1);
        pool.close().await;
        let _ = tokio::fs::remove_file(path).await;
    }

    #[test]
    fn model_analysis_cache_key_ignores_observation_time_but_tracks_facts() {
        let mut observation = OpsObservation {
            event_key: "provider:availability".to_owned(),
            detector_type: "provider_health".to_owned(),
            severity: modelport_ops_protocol::OpsSeverity::Sev3,
            title: "provider degraded".to_owned(),
            summary: "one provider is unavailable".to_owned(),
            affected_scope: json!({ "provider": "local_vllm" }),
            evidence: json!({ "unavailable": 1 }),
            observed_at_ms: 1,
            active: true,
            recovery_criteria: "provider recovers".to_owned(),
        };
        let first = model_analysis_cache_key("local_vllm:qwen", &observation).unwrap();
        observation.observed_at_ms = 2;
        let repeated = model_analysis_cache_key("local_vllm:qwen", &observation).unwrap();
        assert_eq!(first, repeated);
        observation.evidence = json!({ "unavailable": 2 });
        let changed = model_analysis_cache_key("local_vllm:qwen", &observation).unwrap();
        assert_ne!(first, changed);
    }

    #[tokio::test]
    async fn optional_model_analysis_uses_the_explicit_model_and_separate_key() {
        async fn completion(
            headers: axum::http::HeaderMap,
            Json(body): Json<Value>,
        ) -> Json<Value> {
            assert_eq!(
                headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer model-key")
            );
            assert_eq!(body["model"], "local_vllm:qwen3");
            Json(json!({
                "choices": [{ "message": { "content": "先验证本地运行时，再检查凭证。" } }]
            }))
        }

        let app = Router::new().route("/v1/chat/completions", axum::routing::post(completion));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let path = std::env::temp_dir().join(format!(
            "modelport-ops-analysis-{}-{}.sqlite",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let spool = open_spool(path.to_str().unwrap()).await.unwrap();
        let state = AppState {
            config: Arc::new(Config {
                base_url: format!("http://{address}"),
                api_key: "control-key".to_owned(),
                mode: "read_only".to_owned(),
                interval: Duration::from_secs(300),
                bind: "127.0.0.1:0".parse().unwrap(),
                spool_path: path.to_string_lossy().into_owned(),
                webhook_url: None,
                model_api_key: "model-key".to_owned(),
            }),
            status: Arc::new(RwLock::new(RuntimeStatus::default())),
            spool: spool.clone(),
            client: Client::new(),
        };
        let observation = OpsObservation {
            event_key: "provider:availability".to_owned(),
            detector_type: "provider_health".to_owned(),
            severity: modelport_ops_protocol::OpsSeverity::Sev3,
            title: "provider degraded".to_owned(),
            summary: "one local provider is unavailable".to_owned(),
            affected_scope: json!({ "provider": "local_vllm" }),
            evidence: json!({ "unavailable": 1 }),
            observed_at_ms: now_millis(),
            active: true,
            recovery_criteria: "provider recovers".to_owned(),
        };

        let analysis = request_model_analysis(&state, "local_vllm:qwen3", &observation)
            .await
            .unwrap();

        assert_eq!(analysis, "先验证本地运行时，再检查凭证。");
        server.abort();
        spool.close().await;
        let _ = tokio::fs::remove_file(path).await;
    }
}
