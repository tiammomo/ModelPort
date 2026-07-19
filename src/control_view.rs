use std::{collections::BTreeMap, env};

use serde_json::{Value, json};

use crate::{
    control::{PublicApiKey, PublicQuota},
    provider_status::{provider_account_issue, provider_failure_guidance},
    usage::{DAY_MS, UsageCostRecord, usage_cost_for_team},
};

pub(crate) trait ApiKeyViewRecord {
    fn id(&self) -> &str;
    fn user_id(&self) -> &str;
    fn username(&self) -> &str;
    fn name(&self) -> &str;
    fn key_prefix(&self) -> &str;
    fn key_preview(&self) -> &str;
    fn group(&self) -> Option<&str>;
    fn team_id(&self) -> Option<&str>;
    fn team_name(&self) -> Option<&str>;
    fn allowed_models(&self) -> &[String];
    fn allowed_providers(&self) -> &[String];
    fn organization_id(&self) -> &str;
    fn project_id(&self) -> &str;
    fn environment_id(&self) -> &str;
    fn created_at_ms(&self) -> u64;
    fn last_used_at_ms(&self) -> Option<u64>;
    fn expires_at_ms(&self) -> Option<u64>;
    fn status(&self) -> &str;
    fn ip_restricted(&self) -> bool;
    fn allowed_ips(&self) -> &[String];
    fn spend_limit_usd(&self) -> f64;
    fn rate_limited(&self) -> bool;
    fn five_hour_limit_usd(&self) -> f64;
    fn daily_limit_usd(&self) -> f64;
    fn weekly_limit_usd(&self) -> f64;
    fn monthly_limit_usd(&self) -> f64;
}

pub(crate) trait TeamViewRecord {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn slug(&self) -> &str;
    fn description(&self) -> Option<&str>;
    fn status(&self) -> &str;
    fn daily_limit_usd(&self) -> f64;
    fn monthly_limit_usd(&self) -> f64;
    fn allowed_models(&self) -> &[String];
    fn allowed_providers(&self) -> &[String];
    fn created_at_ms(&self) -> u64;
    fn updated_at_ms(&self) -> u64;
}

pub(crate) trait QuotaViewRecord {
    fn id(&self) -> &str;
    fn user_id(&self) -> &str;
    fn username(&self) -> &str;
    fn quota_type(&self) -> &str;
    fn limit(&self) -> f64;
    fn used(&self) -> f64;
    fn period(&self) -> &str;
    fn period_start_ms(&self) -> u64;
    fn period_end_ms(&self) -> u64;
    fn reset_at_ms(&self) -> u64;
}

pub(crate) trait UsageTokenRecord {
    fn input_tokens(&self) -> u64;
    fn output_tokens(&self) -> u64;
    fn cache_write_tokens(&self) -> u64;
    fn cache_read_tokens(&self) -> u64;
}

pub(crate) trait ProviderHealthViewRecord {
    fn provider_id(&self) -> &str;
    fn requests_total(&self) -> u64;
    fn successes_total(&self) -> u64;
    fn failures_total(&self) -> u64;
    fn consecutive_failures(&self) -> u32;
    fn last_success_at_ms(&self) -> Option<u64>;
    fn last_failure_at_ms(&self) -> Option<u64>;
    fn cooldown_until_ms(&self) -> Option<u64>;
    fn last_error(&self) -> Option<&str>;
    fn last_status_code(&self) -> Option<u16>;
}

pub(crate) trait ProviderCredentialHealthViewRecord: ProviderHealthViewRecord {
    fn credential_id(&self) -> &str;
    fn last_used_at_ms(&self) -> Option<u64>;
}

pub(crate) trait ProviderCredentialViewRecord {
    fn id(&self) -> &str;
    fn provider_id(&self) -> &str;
    fn name(&self) -> &str;
    fn api_key_env(&self) -> &str;
    fn base_url(&self) -> Option<&str>;
    fn status(&self) -> &str;
    fn created_at_ms(&self) -> u64;
    fn updated_at_ms(&self) -> u64;
}

pub(crate) fn public_api_key<K, U>(record: &K, usage: &[U], now: u64) -> PublicApiKey
where
    K: ApiKeyViewRecord,
    U: UsageCostRecord + UsageTokenRecord,
{
    let today_start = crate::usage::day_start(now);
    let mut requests_today = 0u64;
    let mut tokens_today = 0u64;
    for usage in usage.iter().filter(|usage| {
        usage.timestamp_ms() >= today_start && usage.api_key_id() == Some(record.id())
    }) {
        requests_today += 1;
        tokens_today = tokens_today
            .saturating_add(usage.input_tokens())
            .saturating_add(usage.output_tokens())
            .saturating_add(usage.cache_write_tokens())
            .saturating_add(usage.cache_read_tokens());
    }

    PublicApiKey {
        id: record.id().to_owned(),
        user_id: record.user_id().to_owned(),
        username: record.username().to_owned(),
        name: record.name().to_owned(),
        key_prefix: record.key_prefix().to_owned(),
        key_preview: record.key_preview().to_owned(),
        group: record.group().map(str::to_owned),
        team_id: record.team_id().map(str::to_owned),
        team_name: record.team_name().map(str::to_owned),
        allowed_models: record.allowed_models().to_vec(),
        allowed_providers: record.allowed_providers().to_vec(),
        organization_id: record.organization_id().to_owned(),
        project_id: record.project_id().to_owned(),
        environment_id: record.environment_id().to_owned(),
        created_at: record.created_at_ms().to_string(),
        last_used_at: record.last_used_at_ms().map(|value| value.to_string()),
        expires_at: record.expires_at_ms().map(|value| value.to_string()),
        status: record.status().to_owned(),
        requests_today,
        tokens_today,
        ip_restricted: record.ip_restricted(),
        allowed_ips: record.allowed_ips().to_vec(),
        spend_limit_usd: record.spend_limit_usd(),
        rate_limited: record.rate_limited(),
        five_hour_limit_usd: record.five_hour_limit_usd(),
        daily_limit_usd: record.daily_limit_usd(),
        weekly_limit_usd: record.weekly_limit_usd(),
        monthly_limit_usd: record.monthly_limit_usd(),
    }
}

pub(crate) fn public_team<T, K, U>(
    record: &T,
    api_keys: &BTreeMap<String, K>,
    usage: &[U],
    now: u64,
) -> Value
where
    T: TeamViewRecord,
    K: ApiKeyViewRecord,
    U: UsageCostRecord + UsageTokenRecord,
{
    let today_start = crate::usage::day_start(now);
    let month_start = now.saturating_sub(30 * DAY_MS);
    let active_api_keys = api_keys
        .values()
        .filter(|key| key.team_id() == Some(record.id()) && key.status() == "active")
        .count();
    let requests_today = usage
        .iter()
        .filter(|event| event.team_id() == Some(record.id()) && event.timestamp_ms() >= today_start)
        .count();

    json!({
        "id": record.id(),
        "name": record.name(),
        "slug": record.slug(),
        "description": record.description(),
        "status": record.status(),
        "dailyLimitUsd": record.daily_limit_usd(),
        "monthlyLimitUsd": record.monthly_limit_usd(),
        "dailySpendUsd": usage_cost_for_team(usage, record.id(), Some(today_start)),
        "monthlySpendUsd": usage_cost_for_team(usage, record.id(), Some(month_start)),
        "allowedModels": record.allowed_models(),
        "allowedProviders": record.allowed_providers(),
        "activeApiKeys": active_api_keys,
        "requestsToday": requests_today,
        "createdAt": record.created_at_ms().to_string(),
        "updatedAt": record.updated_at_ms().to_string(),
    })
}

pub(crate) fn public_quota<Q: QuotaViewRecord>(record: &Q) -> PublicQuota {
    PublicQuota {
        id: record.id().to_owned(),
        user_id: record.user_id().to_owned(),
        username: record.username().to_owned(),
        quota_type: record.quota_type().to_owned(),
        limit: record.limit(),
        used: record.used(),
        period: record.period().to_owned(),
        period_start: record.period_start_ms().to_string(),
        period_end: record.period_end_ms().to_string(),
        reset_at: record.reset_at_ms().to_string(),
    }
}

pub(crate) fn provider_health_row<R: ProviderHealthViewRecord>(record: &R, now: u64) -> Value {
    let in_cooldown = record.cooldown_until_ms().is_some_and(|until| until > now);
    let success_rate = success_rate(record.requests_total(), record.successes_total());
    let (failure_kind, recommended_action) =
        provider_failure_guidance(record.last_status_code(), record.last_error());
    let account_issue = provider_account_issue(record.last_status_code(), record.last_error());
    let recharge_required = account_issue == "insufficient_balance";

    json!({
        "providerId": record.provider_id(),
        "requestsTotal": record.requests_total(),
        "successesTotal": record.successes_total(),
        "failuresTotal": record.failures_total(),
        "consecutiveFailures": record.consecutive_failures(),
        "successRate": success_rate,
        "status": health_status(in_cooldown, record.consecutive_failures()),
        "lastSuccessAt": record.last_success_at_ms().map(|value| value.to_string()),
        "lastFailureAt": record.last_failure_at_ms().map(|value| value.to_string()),
        "cooldownUntil": record.cooldown_until_ms().map(|value| value.to_string()),
        "lastError": record.last_error(),
        "lastStatusCode": record.last_status_code(),
        "failureKind": failure_kind,
        "accountIssue": account_issue,
        "rechargeRequired": recharge_required,
        "rechargeBadge": if recharge_required { Some("等待充值") } else { None },
        "recommendedAction": recommended_action,
    })
}

pub(crate) fn provider_credential_health_row<R: ProviderCredentialHealthViewRecord>(
    record: &R,
    now: u64,
) -> Value {
    let mut row = provider_health_row(record, now);
    if let Some(object) = row.as_object_mut() {
        object.insert("credentialId".to_owned(), json!(record.credential_id()));
        object.insert(
            "lastUsedAt".to_owned(),
            json!(record.last_used_at_ms().map(|value| value.to_string())),
        );
    }
    row
}

pub(crate) fn provider_credential_rows<R: ProviderCredentialViewRecord>(
    credentials: Option<&BTreeMap<String, R>>,
    active_id: Option<&str>,
    health: Option<&BTreeMap<String, Value>>,
) -> Vec<Value> {
    credentials
        .into_iter()
        .flat_map(|items| items.values())
        .map(|record| {
            provider_credential_row_with_health(
                record,
                active_id == Some(record.id()),
                health.and_then(|items| items.get(record.id())),
            )
        })
        .collect()
}

pub(crate) fn provider_credential_row<R: ProviderCredentialViewRecord>(
    record: &R,
    active: bool,
) -> Value {
    provider_credential_row_with_health(record, active, None)
}

fn provider_credential_row_with_health<R: ProviderCredentialViewRecord>(
    record: &R,
    active: bool,
    health: Option<&Value>,
) -> Value {
    json!({
        "id": record.id(),
        "providerId": record.provider_id(),
        "name": record.name(),
        "apiKeyEnv": record.api_key_env(),
        "baseUrl": record.base_url(),
        "status": record.status(),
        "active": active,
        "hasApiKey": env::var(record.api_key_env()).ok().is_some_and(|value| !value.trim().is_empty()),
        "health": health.cloned(),
        "createdAt": record.created_at_ms().to_string(),
        "updatedAt": record.updated_at_ms().to_string(),
    })
}

fn success_rate(requests_total: u64, successes_total: u64) -> f64 {
    if requests_total == 0 {
        0.0
    } else {
        (successes_total as f64 / requests_total as f64) * 100.0
    }
}

fn health_status(in_cooldown: bool, consecutive_failures: u32) -> &'static str {
    if in_cooldown {
        "cooldown"
    } else if consecutive_failures > 0 {
        "degraded"
    } else {
        "healthy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pricing::TokenUsageBreakdown, usage::UsageCostRecord};

    #[derive(Debug, Clone, Default)]
    struct TestApiKey {
        id: String,
        team_id: Option<String>,
        status: String,
    }

    impl ApiKeyViewRecord for TestApiKey {
        fn id(&self) -> &str {
            &self.id
        }
        fn user_id(&self) -> &str {
            "user"
        }
        fn username(&self) -> &str {
            "username"
        }
        fn name(&self) -> &str {
            "key"
        }
        fn key_prefix(&self) -> &str {
            "mp_"
        }
        fn key_preview(&self) -> &str {
            "mp_***"
        }
        fn group(&self) -> Option<&str> {
            None
        }
        fn team_id(&self) -> Option<&str> {
            self.team_id.as_deref()
        }
        fn team_name(&self) -> Option<&str> {
            None
        }
        fn allowed_models(&self) -> &[String] {
            &[]
        }
        fn allowed_providers(&self) -> &[String] {
            &[]
        }
        fn organization_id(&self) -> &str {
            "org_local"
        }
        fn project_id(&self) -> &str {
            "prj_default"
        }
        fn environment_id(&self) -> &str {
            "env_default"
        }
        fn created_at_ms(&self) -> u64 {
            1
        }
        fn last_used_at_ms(&self) -> Option<u64> {
            Some(2)
        }
        fn expires_at_ms(&self) -> Option<u64> {
            None
        }
        fn status(&self) -> &str {
            &self.status
        }
        fn ip_restricted(&self) -> bool {
            false
        }
        fn allowed_ips(&self) -> &[String] {
            &[]
        }
        fn spend_limit_usd(&self) -> f64 {
            0.0
        }
        fn rate_limited(&self) -> bool {
            false
        }
        fn five_hour_limit_usd(&self) -> f64 {
            0.0
        }
        fn daily_limit_usd(&self) -> f64 {
            0.0
        }
        fn weekly_limit_usd(&self) -> f64 {
            0.0
        }
        fn monthly_limit_usd(&self) -> f64 {
            0.0
        }
    }

    #[derive(Debug, Clone)]
    struct TestUsage {
        timestamp_ms: u64,
        api_key_id: Option<String>,
        team_id: Option<String>,
        tokens: TokenUsageBreakdown,
        cost_estimate: f64,
    }

    #[derive(Debug, Clone, Default)]
    struct TestHealth {
        provider_id: String,
        credential_id: Option<String>,
        requests_total: u64,
        successes_total: u64,
        failures_total: u64,
        consecutive_failures: u32,
        last_success_at_ms: Option<u64>,
        last_failure_at_ms: Option<u64>,
        last_used_at_ms: Option<u64>,
        cooldown_until_ms: Option<u64>,
        last_error: Option<String>,
        last_status_code: Option<u16>,
    }

    struct TestCredential {
        id: String,
        api_key_env: String,
    }

    impl ProviderHealthViewRecord for TestHealth {
        fn provider_id(&self) -> &str {
            &self.provider_id
        }
        fn requests_total(&self) -> u64 {
            self.requests_total
        }
        fn successes_total(&self) -> u64 {
            self.successes_total
        }
        fn failures_total(&self) -> u64 {
            self.failures_total
        }
        fn consecutive_failures(&self) -> u32 {
            self.consecutive_failures
        }
        fn last_success_at_ms(&self) -> Option<u64> {
            self.last_success_at_ms
        }
        fn last_failure_at_ms(&self) -> Option<u64> {
            self.last_failure_at_ms
        }
        fn cooldown_until_ms(&self) -> Option<u64> {
            self.cooldown_until_ms
        }
        fn last_error(&self) -> Option<&str> {
            self.last_error.as_deref()
        }
        fn last_status_code(&self) -> Option<u16> {
            self.last_status_code
        }
    }

    impl ProviderCredentialHealthViewRecord for TestHealth {
        fn credential_id(&self) -> &str {
            self.credential_id.as_deref().unwrap_or("main")
        }
        fn last_used_at_ms(&self) -> Option<u64> {
            self.last_used_at_ms
        }
    }

    impl ProviderCredentialViewRecord for TestCredential {
        fn id(&self) -> &str {
            &self.id
        }

        fn provider_id(&self) -> &str {
            "deepseek"
        }

        fn name(&self) -> &str {
            "DeepSeek Account A"
        }

        fn api_key_env(&self) -> &str {
            &self.api_key_env
        }

        fn base_url(&self) -> Option<&str> {
            Some("https://api.deepseek.com/anthropic")
        }

        fn status(&self) -> &str {
            "active"
        }

        fn created_at_ms(&self) -> u64 {
            1_000
        }

        fn updated_at_ms(&self) -> u64 {
            2_000
        }
    }

    impl UsageCostRecord for TestUsage {
        fn timestamp_ms(&self) -> u64 {
            self.timestamp_ms
        }
        fn api_key_id(&self) -> Option<&str> {
            self.api_key_id.as_deref()
        }
        fn team_id(&self) -> Option<&str> {
            self.team_id.as_deref()
        }
        fn cost_estimate(&self) -> f64 {
            self.cost_estimate
        }
    }

    impl UsageTokenRecord for TestUsage {
        fn input_tokens(&self) -> u64 {
            self.tokens.input_tokens
        }
        fn output_tokens(&self) -> u64 {
            self.tokens.output_tokens
        }
        fn cache_write_tokens(&self) -> u64 {
            self.tokens.cache_write_tokens
        }
        fn cache_read_tokens(&self) -> u64 {
            self.tokens.cache_read_tokens
        }
    }

    #[test]
    fn public_api_key_counts_today_requests_and_tokens() {
        let now = DAY_MS * 10 + 1_000;
        let key = TestApiKey {
            id: "key_a".to_owned(),
            status: "active".to_owned(),
            ..TestApiKey::default()
        };
        let usage = vec![
            TestUsage {
                timestamp_ms: DAY_MS * 10,
                api_key_id: Some("key_a".to_owned()),
                team_id: None,
                tokens: TokenUsageBreakdown {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_write_tokens: 3,
                    cache_read_tokens: 4,
                },
                cost_estimate: 0.0,
            },
            TestUsage {
                timestamp_ms: DAY_MS * 9,
                api_key_id: Some("key_a".to_owned()),
                team_id: None,
                tokens: TokenUsageBreakdown {
                    input_tokens: 100,
                    ..TokenUsageBreakdown::default()
                },
                cost_estimate: 0.0,
            },
        ];

        let row = public_api_key(&key, &usage, now);

        assert_eq!(row.requests_today, 1);
        assert_eq!(row.tokens_today, 10);
    }

    #[test]
    fn public_quota_formats_millisecond_fields() {
        struct TestQuota;
        impl QuotaViewRecord for TestQuota {
            fn id(&self) -> &str {
                "quota"
            }
            fn user_id(&self) -> &str {
                "user"
            }
            fn username(&self) -> &str {
                "username"
            }
            fn quota_type(&self) -> &str {
                "requests"
            }
            fn limit(&self) -> f64 {
                10.0
            }
            fn used(&self) -> f64 {
                2.0
            }
            fn period(&self) -> &str {
                "daily"
            }
            fn period_start_ms(&self) -> u64 {
                100
            }
            fn period_end_ms(&self) -> u64 {
                200
            }
            fn reset_at_ms(&self) -> u64 {
                200
            }
        }

        let row = public_quota(&TestQuota);

        assert_eq!(row.period_start, "100");
        assert_eq!(row.reset_at, "200");
    }

    #[test]
    fn provider_health_row_marks_balance_as_recharge_required() {
        let row = provider_health_row(
            &TestHealth {
                provider_id: "deepseek".to_owned(),
                requests_total: 4,
                successes_total: 3,
                failures_total: 1,
                consecutive_failures: 1,
                last_error: Some("Insufficient Balance".to_owned()),
                last_status_code: Some(402),
                ..TestHealth::default()
            },
            1_000,
        );

        assert_eq!(row["successRate"], 75.0);
        assert_eq!(row["status"], "degraded");
        assert_eq!(row["failureKind"], "account");
        assert_eq!(row["accountIssue"], "insufficient_balance");
        assert_eq!(row["rechargeBadge"], "等待充值");
    }

    #[test]
    fn credential_health_row_includes_credential_fields() {
        let row = provider_credential_health_row(
            &TestHealth {
                provider_id: "deepseek".to_owned(),
                credential_id: Some("backup".to_owned()),
                requests_total: 1,
                successes_total: 1,
                last_used_at_ms: Some(900),
                cooldown_until_ms: Some(2_000),
                ..TestHealth::default()
            },
            1_000,
        );

        assert_eq!(row["providerId"], "deepseek");
        assert_eq!(row["credentialId"], "backup");
        assert_eq!(row["lastUsedAt"], "900");
        assert_eq!(row["status"], "cooldown");
    }

    #[test]
    fn provider_credential_rows_attach_active_state_and_health() {
        let mut credentials = BTreeMap::new();
        credentials.insert(
            "account-a".to_owned(),
            TestCredential {
                id: "account-a".to_owned(),
                api_key_env: "MODELPORT_TEST_MISSING_CREDENTIAL".to_owned(),
            },
        );

        let mut health = BTreeMap::new();
        health.insert(
            "account-a".to_owned(),
            json!({
                "status": "degraded",
                "consecutiveFailures": 1,
            }),
        );

        let rows = provider_credential_rows(Some(&credentials), Some("account-a"), Some(&health));
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["id"], "account-a");
        assert_eq!(row["providerId"], "deepseek");
        assert_eq!(row["baseUrl"], "https://api.deepseek.com/anthropic");
        assert_eq!(row["active"], true);
        assert_eq!(row["hasApiKey"], false);
        assert_eq!(row["health"]["status"], "degraded");
        assert_eq!(row["createdAt"], "1000");
        assert_eq!(row["updatedAt"], "2000");
    }
}
