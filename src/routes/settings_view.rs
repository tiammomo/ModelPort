use std::net::IpAddr;

use serde_json::{Value, json};

use crate::config::{AppConfig, ConfigIssueSeverity};

use super::{AppState, effective_config, env_u64, provider_rows};

pub(super) fn alias_rows(state: &AppState) -> Vec<Value> {
    let config = effective_config(state);
    config
        .aliases
        .iter()
        .map(|(alias, target)| alias_row(&config, alias, target))
        .collect()
}

pub(super) fn alias_row(config: &AppConfig, alias: &str, target: &str) -> Value {
    let resolved = config.resolve(alias).ok();
    json!({
        "alias": alias,
        "target": target,
        "resolvedProvider": resolved.as_ref().map(|value| value.provider_id.as_str()).unwrap_or(""),
        "resolvedModel": resolved.as_ref().map(|value| value.model.as_str()).unwrap_or(""),
    })
}

pub(super) fn settings_row(state: &AppState) -> Value {
    let config = effective_config(state);
    json!({
        "server": {
            "bindAddress": config.bind_addr.to_string(),
            "maxRequestBodyBytes": config.max_request_body_bytes,
            "maxConcurrentRequests": config.max_concurrent_requests,
        },
        "auth": {
            "enabled": config.auth_token.is_some(),
            "tokenEnvVar": "MODELPORT_AUTH_TOKEN",
            "allowNoAuth": config.auth_token.is_none(),
        },
        "gateway": {
            "defaultProvider": config.default_provider,
            "providerOrder": config.provider_order,
        },
        "rateLimits": {
            "maxConcurrentRequests": config.max_concurrent_requests,
            "maxRequestBodyBytes": config.max_request_body_bytes,
            "requestTimeoutSecs": env_u64("MODELPORT_HTTP_REQUEST_TIMEOUT_SECS", 600),
            "streamIdleTimeoutSecs": env_u64("MODELPORT_HTTP_STREAM_IDLE_TIMEOUT_SECS", 300),
        },
        "runtime": runtime_row(state, &config),
        "setup": setup_row(state, &config),
    })
}

fn runtime_row(state: &AppState, config: &AppConfig) -> Value {
    let base_url = local_base_url(config);
    json!({
        "apiEndpoint": format!("{base_url}/v1/messages"),
        "anthropicEndpoint": format!("{base_url}/v1/messages"),
        "openaiEndpoint": format!("{base_url}/v1/chat/completions"),
        "modelsEndpoint": format!("{base_url}/v1/models"),
        "adminEndpoint": format!("{base_url}/admin"),
        "controlDataPath": state.control.data_path(),
        "authDataPath": state.auth.data_path(),
    })
}

fn setup_row(state: &AppState, config: &AppConfig) -> Value {
    let providers = provider_rows(state);
    let active_provider_count = providers
        .iter()
        .filter(|provider| provider.get("status").and_then(Value::as_str) == Some("active"))
        .count();
    let default_provider = providers.iter().find(|provider| {
        provider
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == config.default_provider)
    });
    let default_provider_active = default_provider
        .and_then(|provider| provider.get("status").and_then(Value::as_str))
        == Some("active");
    let validation_issues = config_issues_json(config);
    let validation_errors = validation_issues
        .iter()
        .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("error"))
        .count();
    let validation_warnings = validation_issues
        .iter()
        .filter(|issue| issue.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();
    let checks = vec![
        setup_check(
            "admin",
            "管理员账号",
            state.auth.active_admin_count() > 0,
            "至少一个活跃管理员",
            "没有活跃管理员",
        ),
        setup_check(
            "auth",
            "API 认证",
            config.auth_token.is_some(),
            "已启用请求认证",
            "未配置 MODELPORT_AUTH_TOKEN",
        ),
        setup_check(
            "providers",
            "供应商凭证",
            active_provider_count > 0,
            format!("{active_provider_count} 个供应商可用"),
            "没有可用供应商",
        ),
        setup_check(
            "defaultProvider",
            "默认供应商",
            default_provider_active,
            format!("{} 可用", config.default_provider),
            format!("{} 不可用", config.default_provider),
        ),
        setup_check(
            "persistence",
            "控制面数据",
            state.control.data_path().is_some() && state.auth.data_path().is_some(),
            "已启用本地持久化",
            "当前运行未配置数据文件",
        ),
        setup_check(
            "config",
            "配置校验",
            validation_errors == 0,
            if validation_warnings == 0 {
                "无配置告警".to_owned()
            } else {
                format!("{validation_warnings} 条配置告警")
            },
            format!("{validation_errors} 条配置错误"),
        ),
    ];
    let ready = checks
        .iter()
        .all(|check| check.get("status").and_then(Value::as_str) != Some("error"));

    json!({
        "ready": ready,
        "activeProviderCount": active_provider_count,
        "defaultProviderReady": default_provider_active,
        "checks": checks,
        "issues": validation_issues,
    })
}

pub(super) fn config_issues_json(config: &AppConfig) -> Vec<Value> {
    config
        .validation_issues()
        .into_iter()
        .map(|issue| {
            json!({
                "severity": match issue.severity {
                    ConfigIssueSeverity::Error => "error",
                    ConfigIssueSeverity::Warning => "warning",
                },
                "message": issue.message,
            })
        })
        .collect()
}

fn setup_check(
    id: &str,
    label: &str,
    ok: bool,
    ok_detail: impl Into<String>,
    error_detail: impl Into<String>,
) -> Value {
    json!({
        "id": id,
        "label": label,
        "status": if ok { "ok" } else { "error" },
        "detail": if ok { ok_detail.into() } else { error_detail.into() },
    })
}

fn local_base_url(config: &AppConfig) -> String {
    let ip = if config.bind_addr.ip().is_unspecified() {
        match config.bind_addr.ip() {
            IpAddr::V4(_) => "127.0.0.1".to_owned(),
            IpAddr::V6(_) => "[::1]".to_owned(),
        }
    } else {
        match config.bind_addr.ip() {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        }
    };
    format!("http://{ip}:{}", config.bind_addr.port())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FidelityMode, MaxTokensField, ProviderConfig, ProviderProtocol};

    #[test]
    fn alias_row_exposes_resolved_provider_and_model() {
        let mut config = test_config();
        config
            .aliases
            .insert("fast".to_owned(), "deepseek:deepseek-v4-flash".to_owned());

        let row = alias_row(&config, "fast", "deepseek:deepseek-v4-flash");

        assert_eq!(row["alias"], "fast");
        assert_eq!(row["target"], "deepseek:deepseek-v4-flash");
        assert_eq!(row["resolvedProvider"], "deepseek");
        assert_eq!(row["resolvedModel"], "deepseek-v4-flash");
    }

    #[test]
    fn local_base_url_uses_loopback_for_unspecified_bind_address() {
        let mut config = test_config();
        config.bind_addr = "0.0.0.0:17878".parse().unwrap();

        assert_eq!(local_base_url(&config), "http://127.0.0.1:17878");
    }

    fn test_config() -> AppConfig {
        let provider = ProviderConfig {
            display_name: "DeepSeek".to_owned(),
            protocol: ProviderProtocol::Anthropic,
            base_url: "https://api.deepseek.com/anthropic".to_owned(),
            api_key_env: Some("DEEPSEEK_API_KEY".to_owned()),
            api_key: Some("test-key".to_owned()),
            api_key_required: true,
            default_model: "deepseek-v4-flash".to_owned(),
            models: vec!["deepseek-v4-flash".to_owned()],
            model_prefixes: vec![],
            passthrough_unknown_models: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            deduplicate_stream_text: false,
            buffer_stream_text: false,
            fidelity_mode: FidelityMode::BestEffort,
            tool_use: Default::default(),
            reasoning: Default::default(),
            sampling: Default::default(),
            token_counting: Default::default(),
            pricing: None,
        };
        AppConfig {
            bind_addr: "127.0.0.1:17878".parse().unwrap(),
            auth_token: Some("token".to_owned()),
            default_provider: "deepseek".to_owned(),
            providers: [("deepseek".to_owned(), provider)].into_iter().collect(),
            provider_order: vec!["deepseek".to_owned()],
            aliases: Default::default(),
            max_request_body_bytes: 1024 * 1024,
            max_concurrent_requests: 64,
        }
    }
}
