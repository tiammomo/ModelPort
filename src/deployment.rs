use crate::{
    AppError, database,
    enterprise_ledger::EnterpriseLedger,
    oidc::OidcService,
    routes::{TrustedProxyConfig, validate_allowed_origins_from_env},
};

pub(crate) fn validate_environment() -> Result<(), AppError> {
    database::validate_configuration()?;
    EnterpriseLedger::validate_configuration()?;
    OidcService::validate_configuration()?;
    TrustedProxyConfig::from_env()?;
    validate_allowed_origins_from_env()
}
