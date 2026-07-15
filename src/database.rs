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

pub(crate) fn enterprise_mode_enabled() -> Result<bool, AppError> {
    parse_flag(
        "MODELPORT_ENTERPRISE_MODE",
        non_empty_env("MODELPORT_ENTERPRISE_MODE").as_deref(),
    )
}

pub(crate) fn validate_configuration() -> Result<(), AppError> {
    let enterprise_mode = non_empty_env("MODELPORT_ENTERPRISE_MODE");
    let control_url = database_url();
    let ledger_url = non_empty_env("MODELPORT_ENTERPRISE_DATABASE_URL");
    let tls_mode = non_empty_env("MODELPORT_DATABASE_TLS_MODE");
    let max_connections = non_empty_env("MODELPORT_DATABASE_MAX_CONNECTIONS");
    let min_connections = non_empty_env("MODELPORT_DATABASE_MIN_CONNECTIONS");
    let acquire_timeout = non_empty_env("MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS");
    validate_values(DatabaseValidationValues {
        enterprise_mode: enterprise_mode.as_deref(),
        control_url: control_url.as_deref(),
        ledger_url: ledger_url.as_deref(),
        tls_mode: tls_mode.as_deref(),
        max_connections: max_connections.as_deref(),
        min_connections: min_connections.as_deref(),
        acquire_timeout: acquire_timeout.as_deref(),
    })
}

#[derive(Default)]
struct DatabaseValidationValues<'a> {
    enterprise_mode: Option<&'a str>,
    control_url: Option<&'a str>,
    ledger_url: Option<&'a str>,
    tls_mode: Option<&'a str>,
    max_connections: Option<&'a str>,
    min_connections: Option<&'a str>,
    acquire_timeout: Option<&'a str>,
}

fn validate_values(values: DatabaseValidationValues<'_>) -> Result<(), AppError> {
    let enterprise = parse_flag("MODELPORT_ENTERPRISE_MODE", values.enterprise_mode)?;

    if enterprise && values.control_url.is_none() {
        return Err(AppError::Config(
            "MODELPORT_ENTERPRISE_MODE requires MODELPORT_DATABASE_URL so auth and control state do not fall back to files"
                .to_owned(),
        ));
    }
    if let Some(url) = values.control_url {
        validate_database_url("MODELPORT_DATABASE_URL", url)?;
    }
    if let Some(url) = values.ledger_url {
        validate_database_url("MODELPORT_ENTERPRISE_DATABASE_URL", url)?;
    }

    ssl_mode_for(values.tls_mode, enterprise)?;

    let max_connections =
        parse_optional_u32("MODELPORT_DATABASE_MAX_CONNECTIONS", values.max_connections)?
            .unwrap_or(DEFAULT_POOL_SIZE);
    if max_connections == 0 {
        return Err(AppError::Config(
            "MODELPORT_DATABASE_MAX_CONNECTIONS must be at least 1".to_owned(),
        ));
    }
    let min_connections =
        parse_optional_u32("MODELPORT_DATABASE_MIN_CONNECTIONS", values.min_connections)?
            .unwrap_or(0);
    if min_connections > max_connections {
        return Err(AppError::Config(
            "MODELPORT_DATABASE_MIN_CONNECTIONS must not exceed MODELPORT_DATABASE_MAX_CONNECTIONS"
                .to_owned(),
        ));
    }
    let acquire_timeout = parse_optional_u64(
        "MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS",
        values.acquire_timeout,
    )?
    .unwrap_or(DEFAULT_ACQUIRE_TIMEOUT_SECS);
    if acquire_timeout == 0 {
        return Err(AppError::Config(
            "MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS must be at least 1".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) async fn connect_pool(
    database_url: &str,
    max_connections: Option<u32>,
) -> Result<PgPool, AppError> {
    let enterprise = enterprise_mode_enabled()?;
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
    if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
        return Err(AppError::Config(
            "PostgreSQL connection URL must use the postgres:// or postgresql:// scheme".to_owned(),
        ));
    }
    PgConnectOptions::from_str(database_url)
        .map(|options| options.ssl_mode(ssl_mode).application_name("modelport"))
        .map_err(|_| {
            AppError::Config(
                "invalid PostgreSQL connection URL; verify its scheme, encoded credentials, host, port, and database name"
                    .to_owned(),
            )
        })
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

fn validate_database_url(name: &str, value: &str) -> Result<(), AppError> {
    if !value.starts_with("postgres://") && !value.starts_with("postgresql://") {
        return Err(AppError::Config(format!(
            "{name} must use a postgres:// or postgresql:// connection URL"
        )));
    }
    PgConnectOptions::from_str(value).map(|_| ()).map_err(|_| {
        AppError::Config(format!(
            "{name} is not a valid PostgreSQL connection URL; verify its scheme, encoded credentials, host, port, and database name"
        ))
    })
}

fn parse_flag(name: &str, value: Option<&str>) -> Result<bool, AppError> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("") | Some("0" | "false" | "no" | "off") => Ok(false),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some(_) => Err(AppError::Config(format!(
            "{name} must be one of 1, 0, true, false, yes, no, on, or off"
        ))),
    }
}

fn parse_optional_u32(name: &str, value: Option<&str>) -> Result<Option<u32>, AppError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| AppError::Config(format!("{name} must be a non-negative integer")))
        })
        .transpose()
}

fn parse_optional_u64(name: &str, value: Option<&str>) -> Result<Option<u64>, AppError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| AppError::Config(format!("{name} must be a non-negative integer")))
        })
        .transpose()
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
    fn connection_options_apply_the_enterprise_tls_policy() {
        let url = "postgres://modelport:secret@db.example:5432/modelport";
        assert!(matches!(
            connect_options(url, true).unwrap().get_ssl_mode(),
            PgSslMode::VerifyFull
        ));
        assert!(matches!(
            connect_options(url, false).unwrap().get_ssl_mode(),
            PgSslMode::Prefer
        ));
    }

    #[test]
    fn database_url_redaction_preserves_target_but_not_password() {
        let redacted = redact_database_url("postgres://alice:secret@db:5432/modelport");
        assert_eq!(redacted, "postgres://alice:<redacted>@db:5432/modelport");
        assert!(!redacted.contains("secret"));
    }

    #[test]
    fn deployment_values_reject_invalid_flags_and_pool_numbers() {
        assert!(parse_flag("MODELPORT_ENTERPRISE_MODE", Some("maybe")).is_err());
        assert!(parse_flag("MODELPORT_ENTERPRISE_MODE", Some("yes")).unwrap());
        assert!(parse_optional_u32("MODELPORT_DATABASE_MAX_CONNECTIONS", Some("many")).is_err());
        assert!(parse_optional_u64("MODELPORT_DATABASE_ACQUIRE_TIMEOUT_SECS", Some("-1")).is_err());
    }

    #[test]
    fn database_url_validation_never_echoes_a_secret() {
        let error = validate_database_url(
            "MODELPORT_DATABASE_URL",
            "not-postgres://alice:super-secret@db/modelport",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("MODELPORT_DATABASE_URL"));
        assert!(!error.contains("super-secret"));
    }

    #[test]
    fn enterprise_deployment_requires_control_database_and_strict_tls() {
        let missing_database = validate_values(DatabaseValidationValues {
            enterprise_mode: Some("1"),
            ..DatabaseValidationValues::default()
        })
        .unwrap_err()
        .to_string();
        assert!(missing_database.contains("MODELPORT_DATABASE_URL"));

        let weak_tls = validate_values(DatabaseValidationValues {
            enterprise_mode: Some("1"),
            control_url: Some("postgres://modelport:secret@db:5432/modelport"),
            tls_mode: Some("disable"),
            ..DatabaseValidationValues::default()
        })
        .unwrap_err()
        .to_string();
        assert!(weak_tls.contains("verify-full"));
        assert!(!weak_tls.contains("secret"));
    }

    #[test]
    fn deployment_rejects_inconsistent_pool_bounds() {
        let error = validate_values(DatabaseValidationValues {
            max_connections: Some("4"),
            min_connections: Some("5"),
            ..DatabaseValidationValues::default()
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("MIN_CONNECTIONS"));

        assert!(
            validate_values(DatabaseValidationValues {
                max_connections: Some("16"),
                min_connections: Some("2"),
                acquire_timeout: Some("10"),
                ..DatabaseValidationValues::default()
            })
            .is_ok()
        );
    }
}
