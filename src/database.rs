use std::{env, str::FromStr, time::Duration};

use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};

use crate::AppError;

const DEFAULT_POOL_SIZE: u32 = 16;
const DEFAULT_ACQUIRE_TIMEOUT_SECS: u64 = 10;

pub(crate) fn database_url() -> Option<String> {
    non_empty_env("MODELPORT_DATABASE_URL")
}

pub(crate) fn enterprise_database_url() -> Option<String> {
    non_empty_env("MODELPORT_ENTERPRISE_DATABASE_URL").or_else(database_url)
}

pub(crate) fn enterprise_mode_enabled() -> bool {
    matches!(
        env::var("MODELPORT_ENTERPRISE_MODE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) async fn connect_pool(
    database_url: &str,
    max_connections: Option<u32>,
    enterprise: bool,
) -> Result<PgPool, AppError> {
    let max_connections = max_connections
        .or_else(|| env_u32("MODELPORT_DATABASE_MAX_CONNECTIONS"))
        .unwrap_or(DEFAULT_POOL_SIZE)
        .max(1);
    let min_connections = env_u32("MODELPORT_DATABASE_MIN_CONNECTIONS")
        .unwrap_or(0)
        .min(max_connections);
    let acquire_timeout = Duration::from_secs(
        env_u64("MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_ACQUIRE_TIMEOUT_SECS)
            .max(1),
    );
    let options = connect_options(database_url, enterprise)?;

    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(min_connections)
        .acquire_timeout(acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(300)))
        .max_lifetime(Some(Duration::from_secs(1_800)))
        .connect_with(options)
        .await
        .map_err(AppError::from)
}

fn connect_options(database_url: &str, enterprise: bool) -> Result<PgConnectOptions, AppError> {
    let ssl_mode = configured_ssl_mode(enterprise)?;
    PgConnectOptions::from_str(database_url)
        .map(|options| options.ssl_mode(ssl_mode).application_name("modelport"))
        .map_err(|error| AppError::Config(format!("invalid PostgreSQL URL: {error}")))
}

fn configured_ssl_mode(enterprise: bool) -> Result<PgSslMode, AppError> {
    let configured = non_empty_env("MODELPORT_DATABASE_TLS_MODE");
    ssl_mode_for(configured.as_deref(), enterprise)
}

fn ssl_mode_for(configured: Option<&str>, enterprise: bool) -> Result<PgSslMode, AppError> {
    if enterprise
        && configured.is_some_and(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "verify-full" | "verify_full"
            )
        })
    {
        return Err(AppError::Config(
            "MODELPORT_ENTERPRISE_MODE requires MODELPORT_DATABASE_TLS_MODE=verify-full".to_owned(),
        ));
    }
    let mode = configured
        .map(parse_ssl_mode)
        .transpose()?
        .unwrap_or(if enterprise {
            PgSslMode::VerifyFull
        } else {
            PgSslMode::Prefer
        });

    Ok(mode)
}

fn parse_ssl_mode(value: &str) -> Result<PgSslMode, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "disable" => Ok(PgSslMode::Disable),
        "allow" => Ok(PgSslMode::Allow),
        "prefer" => Ok(PgSslMode::Prefer),
        "require" => Ok(PgSslMode::Require),
        "verify-ca" | "verify_ca" => Ok(PgSslMode::VerifyCa),
        "verify-full" | "verify_full" => Ok(PgSslMode::VerifyFull),
        _ => Err(AppError::Config(format!(
            "invalid MODELPORT_DATABASE_TLS_MODE={value:?}; expected disable, allow, prefer, require, verify-ca, or verify-full"
        ))),
    }
}

pub(crate) fn redact_database_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "postgres://<redacted>".to_owned();
    };
    let Some((userinfo, host)) = rest.split_once('@') else {
        return format!("{scheme}://{rest}");
    };
    let username = userinfo.split(':').next().unwrap_or("modelport");
    format!("{scheme}://{username}:<redacted>@{host}")
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_u32(name: &str) -> Option<u32> {
    non_empty_env(name).and_then(|value| value.parse().ok())
}

fn env_u64(name: &str) -> Option<u64> {
    non_empty_env(name).and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssl_mode_parser_accepts_strict_modes() {
        assert!(matches!(
            parse_ssl_mode("verify-ca").unwrap(),
            PgSslMode::VerifyCa
        ));
        assert!(matches!(
            parse_ssl_mode("verify_full").unwrap(),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn ssl_mode_parser_rejects_unknown_values() {
        assert!(parse_ssl_mode("sometimes").is_err());
    }

    #[test]
    fn enterprise_ssl_policy_defaults_strict_and_rejects_weaker_modes() {
        assert!(matches!(
            ssl_mode_for(None, true).unwrap(),
            PgSslMode::VerifyFull
        ));
        assert!(ssl_mode_for(Some("require"), true).is_err());
        assert!(matches!(
            ssl_mode_for(Some("verify-full"), true).unwrap(),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn database_url_redaction_preserves_target_but_not_password() {
        let redacted = redact_database_url("postgres://alice:secret@db:5432/modelport");
        assert_eq!(redacted, "postgres://alice:<redacted>@db:5432/modelport");
        assert!(!redacted.contains("secret"));
    }
}
