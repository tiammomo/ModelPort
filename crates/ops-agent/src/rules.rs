use modelport_ops_protocol::{OpsObservation, OpsSeverity, OpsSnapshot};
use serde_json::{Value, json};

pub const RULE_SET_VERSION: &str = "ops-rules-v1";

pub fn evaluate(snapshot: &OpsSnapshot) -> Vec<OpsObservation> {
    vec![
        readiness(snapshot),
        providers(snapshot),
        request_anomalies(snapshot),
        budget(snapshot),
        ledger(snapshot),
        post_change(snapshot),
    ]
}

#[allow(clippy::too_many_arguments)]
fn observation(
    snapshot: &OpsSnapshot,
    event_key: &str,
    detector_type: &str,
    severity: OpsSeverity,
    title: &str,
    summary: String,
    active: bool,
    affected_scope: Value,
    evidence: Value,
    recovery_criteria: &str,
) -> OpsObservation {
    OpsObservation {
        event_key: event_key.to_owned(),
        detector_type: detector_type.to_owned(),
        severity,
        title: title.to_owned(),
        summary,
        affected_scope,
        evidence,
        observed_at_ms: snapshot.captured_at_ms,
        active,
        recovery_criteria: recovery_criteria.to_owned(),
    }
}

fn readiness(snapshot: &OpsSnapshot) -> OpsObservation {
    let active = !snapshot.gateway_ready;
    observation(
        snapshot,
        "readiness:gateway",
        "readiness_storage",
        if snapshot.database_ready {
            OpsSeverity::Sev2
        } else {
            OpsSeverity::Sev1
        },
        "ModelPort 无法接收受治理请求",
        if active {
            "至少一个 fail-closed 依赖未就绪，网关当前不可安全接单。".to_owned()
        } else {
            "所有 fail-closed 依赖均已恢复。".to_owned()
        },
        active,
        json!({ "component": "gateway" }),
        json!({
            "gatewayReady": snapshot.gateway_ready,
            "databaseReady": snapshot.database_ready,
            "authReady": snapshot.auth_ready,
            "controlReady": snapshot.control_ready,
            "governanceReady": snapshot.governance_ready,
            "draining": snapshot.draining,
            "degradedLedgerOperations": snapshot.degraded_ledger_operations,
        }),
        "数据库、认证、控制面、治理存储和账本操作全部就绪，且网关不处于 draining。",
    )
}

fn providers(snapshot: &OpsSnapshot) -> OpsObservation {
    let unhealthy = snapshot
        .provider_health
        .iter()
        .filter_map(|(provider, health)| {
            let status = health
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let recharge = health
                .get("rechargeRequired")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (matches!(status, "degraded" | "cooldown" | "unavailable") || recharge).then(|| {
                json!({
                    "providerId": provider,
                    "status": status,
                    "rechargeRequired": recharge,
                    "consecutiveFailures": health.get("consecutiveFailures"),
                    "failureKind": health.get("failureKind"),
                })
            })
        })
        .collect::<Vec<_>>();
    let active = !unhealthy.is_empty();
    observation(
        snapshot,
        "provider:availability",
        "provider_health",
        if unhealthy.len() == snapshot.provider_health.len() && active {
            OpsSeverity::Sev2
        } else {
            OpsSeverity::Sev3
        },
        "Provider 路由质量下降",
        if active {
            format!(
                "{} 个 Provider 处于降级、冷却或账户异常状态。",
                unhealthy.len()
            )
        } else {
            "已记录的 Provider 均无运行时异常。".to_owned()
        },
        active,
        json!({ "component": "providers" }),
        json!({ "unhealthyProviders": unhealthy }),
        "所有已记录 Provider 恢复 healthy，且不再需要充值。",
    )
}

fn request_anomalies(snapshot: &OpsSnapshot) -> OpsObservation {
    let requests = &snapshot.requests;
    let failures = requests
        .server_errors
        .saturating_add(requests.protocol_failures)
        .saturating_add(requests.stream_failures);
    let ratio = if requests.total_requests == 0 {
        0.0
    } else {
        failures as f64 / requests.total_requests as f64
    };
    let active = requests.total_requests >= 20 && ratio >= 0.05;
    observation(
        snapshot,
        "requests:failure-ratio",
        "request_anomaly",
        if ratio >= 0.25 {
            OpsSeverity::Sev2
        } else {
            OpsSeverity::Sev3
        },
        "请求失败率超出运行基线",
        format!(
            "最近 {} 秒共 {} 次请求，受控失败率为 {:.1}%。",
            requests.window_seconds,
            requests.total_requests,
            ratio * 100.0
        ),
        active,
        json!({ "component": "request-path" }),
        json!({
            "windowSeconds": requests.window_seconds,
            "totalRequests": requests.total_requests,
            "serverErrors": requests.server_errors,
            "protocolFailures": requests.protocol_failures,
            "streamFailures": requests.stream_failures,
            "failureRatio": ratio,
            "averageLatencyMs": requests.average_latency_ms,
        }),
        "至少 20 次请求的滚动窗口内，受控失败率低于 5%。",
    )
}

fn budget(snapshot: &OpsSnapshot) -> OpsObservation {
    let exhausted = snapshot.ledger.budget_accounts_exhausted;
    let warning = snapshot.ledger.budget_accounts_at_or_above_80_percent;
    let active = exhausted > 0 || warning > 0;
    observation(
        snapshot,
        "budget:capacity",
        "budget_quota",
        if exhausted > 0 {
            OpsSeverity::Sev2
        } else {
            OpsSeverity::Sev3
        },
        "预算容量需要关注",
        if exhausted > 0 {
            format!("{exhausted} 个预算账户已耗尽，{warning} 个达到或超过 80%。")
        } else {
            format!("{warning} 个预算账户达到或超过 80%。")
        },
        active,
        json!({ "component": "budget" }),
        json!({ "warningAccounts": warning, "exhaustedAccounts": exhausted }),
        "没有预算账户耗尽或达到 80% 告警线。",
    )
}

fn ledger(snapshot: &OpsSnapshot) -> OpsObservation {
    let health = &snapshot.ledger;
    let stale_reservation = health.oldest_open_reservation_age_ms >= 10 * 60 * 1_000;
    let active = health.unreconciled_requests > 0 || stale_reservation;
    observation(
        snapshot,
        "ledger:finalization-backlog",
        "ledger_backlog",
        if health.unreconciled_requests > 0 {
            OpsSeverity::Sev2
        } else {
            OpsSeverity::Sev3
        },
        "账本终态积压",
        format!(
            "未对账请求 {}，开放用量预留 {}，最老预留 {} ms。",
            health.unreconciled_requests,
            health.open_usage_reservations,
            health.oldest_open_reservation_age_ms
        ),
        active,
        json!({ "component": "enterprise-ledger" }),
        json!({
            "unreconciledRequests": health.unreconciled_requests,
            "openUsageReservations": health.open_usage_reservations,
            "oldestOpenReservationAgeMs": health.oldest_open_reservation_age_ms,
            "pendingFinalizers": snapshot.pending_finalizers,
        }),
        "最近 24 小时没有 unreconciled 请求，且开放用量预留年龄低于 10 分钟。",
    )
}

fn post_change(snapshot: &OpsSnapshot) -> OpsObservation {
    let recent = snapshot.recent_change_at_ms.is_some_and(|changed_at| {
        snapshot.captured_at_ms.saturating_sub(changed_at) <= 15 * 60 * 1_000
    });
    let active = recent && !snapshot.gateway_ready;
    observation(
        snapshot,
        "change:verification",
        "post_change_verification",
        OpsSeverity::Sev2,
        "变更后验证失败",
        if active {
            "最近 15 分钟内发生配置或高风险变更，当前就绪性未通过。".to_owned()
        } else {
            "最近变更窗口没有观察到就绪性回退。".to_owned()
        },
        active,
        json!({ "component": "change-management" }),
        json!({
            "recentChangeAtMs": snapshot.recent_change_at_ms,
            "gatewayReady": snapshot.gateway_ready,
        }),
        "变更后连续观测到网关恢复就绪，或变更已超过 15 分钟验证窗口。",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use modelport_ops_protocol::{OpsAgentConfiguration, OpsLedgerHealth, OpsRequestWindow};

    use super::*;

    fn healthy_snapshot() -> OpsSnapshot {
        OpsSnapshot {
            captured_at_ms: 1_000_000,
            gateway_ready: true,
            database_ready: true,
            auth_ready: true,
            control_ready: true,
            governance_ready: true,
            draining: false,
            pending_finalizers: 0,
            degraded_ledger_operations: Vec::new(),
            provider_health: BTreeMap::new(),
            requests: OpsRequestWindow {
                window_seconds: 300,
                ..Default::default()
            },
            ledger: OpsLedgerHealth::default(),
            recent_change_at_ms: None,
            agent_configuration: OpsAgentConfiguration::default(),
        }
    }

    #[test]
    fn evaluates_all_six_event_groups_and_recovers_cleanly() {
        let observations = evaluate(&healthy_snapshot());
        assert_eq!(observations.len(), 6);
        assert!(observations.iter().all(|observation| !observation.active));
    }

    #[test]
    fn failure_ratio_requires_minimum_volume() {
        let mut snapshot = healthy_snapshot();
        snapshot.requests.total_requests = 19;
        snapshot.requests.server_errors = 19;
        assert!(!request_anomalies(&snapshot).active);
        snapshot.requests.total_requests = 20;
        assert!(request_anomalies(&snapshot).active);
    }
}
