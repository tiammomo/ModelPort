//! Optional operations observations and incident queries. The shared ledger
//! retains transaction, request, budget, and lease ownership.

use super::*;

impl EnterpriseLedger {
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
}
