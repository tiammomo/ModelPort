pub(crate) const ACCOUNT_ISSUE_CREDENTIAL_COOLDOWN_SECONDS: u64 = 30 * 60;

pub(crate) fn provider_failure_guidance(
    status_code: Option<u16>,
    error: Option<&str>,
) -> (&'static str, &'static str) {
    let normalized_error = normalized_error_text(error);
    if provider_account_issue(status_code, error) == "insufficient_balance" {
        return (
            "account",
            "上游账号余额不足，请为该渠道充值后重试，或切换到另一个账号/Provider。",
        );
    }
    if status_code == Some(401) || status_code == Some(403) {
        return (
            "account",
            "检查上游 API Key、账号权限或余额，必要时切换账号。",
        );
    }
    if status_code == Some(429) || normalized_error.contains("rate limit") {
        return ("rate_limit", "上游限流中，可等待冷却、降低流量或切换账号。");
    }
    if normalized_error.contains("missing") || normalized_error.contains("api key") {
        return (
            "config",
            "检查 Provider 凭证环境变量是否已配置并重新加载配置。",
        );
    }
    if status_code.is_some_and(|status| status >= 500) {
        return (
            "upstream_unavailable",
            "上游服务临时不可用，可等待恢复或切换 Provider。",
        );
    }
    if error.is_some() {
        return ("unknown", "查看请求日志和上游错误详情。");
    }
    ("none", "")
}

pub(crate) fn provider_account_issue(
    status_code: Option<u16>,
    error: Option<&str>,
) -> &'static str {
    let normalized_error = normalized_error_text(error);
    if normalized_error.contains("insufficient_balance")
        || normalized_error.contains("insufficient balance")
        || normalized_error.contains("insufficient account balance")
        || normalized_error.contains("balance not enough")
        || normalized_error.contains("余额不足")
    {
        return "insufficient_balance";
    }
    if status_code == Some(401) || status_code == Some(403) {
        return "auth";
    }
    "none"
}

pub(crate) fn should_rotate_provider_credential(failure_kind: &str) -> bool {
    matches!(failure_kind, "account" | "rate_limit" | "config")
}

pub(crate) fn provider_failure_reason_label(failure_kind: &str) -> &'static str {
    match failure_kind {
        "account" => "账号异常，检查 API Key 或余额",
        "rate_limit" => "上游限流",
        "config" => "凭证配置异常",
        "upstream_unavailable" => "上游不可用",
        _ => "请求失败",
    }
}

pub(crate) fn cooldown_seconds(consecutive_failures: u32) -> u64 {
    match consecutive_failures {
        0 | 1 => 30,
        2 | 3 => 60,
        4 | 5 => 180,
        _ => 300,
    }
}

pub(crate) fn credential_cooldown_seconds(failure_kind: &str, consecutive_failures: u32) -> u64 {
    let base = cooldown_seconds(consecutive_failures);
    if failure_kind == "account" {
        return base.max(ACCOUNT_ISSUE_CREDENTIAL_COOLDOWN_SECONDS);
    }
    base
}

fn normalized_error_text(error: Option<&str>) -> String {
    error.unwrap_or("").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_balance_variants_before_generic_account_errors() {
        assert_eq!(
            provider_account_issue(
                Some(402),
                Some(r#"{"error":{"message":"Insufficient Balance"}}"#),
            ),
            "insufficient_balance"
        );
        assert_eq!(
            provider_account_issue(Some(500), Some("余额不足，请充值后重试")),
            "insufficient_balance"
        );
        assert_eq!(
            provider_failure_guidance(Some(402), Some("balance not enough")),
            (
                "account",
                "上游账号余额不足，请为该渠道充值后重试，或切换到另一个账号/Provider。",
            )
        );
    }

    #[test]
    fn classifies_common_provider_failures() {
        assert_eq!(provider_failure_guidance(Some(401), None).0, "account");
        assert_eq!(
            provider_failure_guidance(Some(429), Some("too many requests")).0,
            "rate_limit"
        );
        assert_eq!(
            provider_failure_guidance(None, Some("missing api key")).0,
            "config"
        );
        assert_eq!(
            provider_failure_guidance(Some(503), Some("unavailable")).0,
            "upstream_unavailable"
        );
    }

    #[test]
    fn rotation_is_limited_to_recoverable_credential_failures() {
        assert!(should_rotate_provider_credential("account"));
        assert!(should_rotate_provider_credential("rate_limit"));
        assert!(should_rotate_provider_credential("config"));
        assert!(!should_rotate_provider_credential("upstream_unavailable"));
        assert!(!should_rotate_provider_credential("unknown"));
    }

    #[test]
    fn account_failures_have_longer_credential_cooldown() {
        assert_eq!(cooldown_seconds(1), 30);
        assert_eq!(credential_cooldown_seconds("rate_limit", 1), 30);
        assert_eq!(
            credential_cooldown_seconds("account", 1),
            ACCOUNT_ISSUE_CREDENTIAL_COOLDOWN_SECONDS
        );
    }
}
