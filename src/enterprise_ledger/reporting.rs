//! Read-only ledger views for the console, request evidence, and usage reporting.

use super::*;

impl EnterpriseLedger {
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
}
