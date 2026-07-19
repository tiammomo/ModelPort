use serde::{Deserialize, Serialize};

use crate::{
    config::ProviderConfig,
    error::AppError,
    http::{Header, HttpTransport},
};

const DEEPSEEK_ANTHROPIC_BASE_URL: &str = "https://api.deepseek.com/anthropic";
const DEEPSEEK_BALANCE_URL: &str = "https://api.deepseek.com/user/balance";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeepSeekBalanceInfo {
    pub currency: String,
    pub total_balance: String,
    pub granted_balance: String,
    pub topped_up_balance: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeepSeekBalance {
    pub is_available: bool,
    pub balance_infos: Vec<DeepSeekBalanceInfo>,
}

pub async fn fetch_balance(
    transport: &HttpTransport,
    provider: &ProviderConfig,
) -> Result<DeepSeekBalance, AppError> {
    if provider.base_url.trim_end_matches('/') != DEEPSEEK_ANTHROPIC_BASE_URL {
        return Err(AppError::InvalidRequest(
            "online balance is available only for the official DeepSeek Anthropic provider"
                .to_owned(),
        ));
    }
    let api_key = provider.api_key()?.ok_or_else(|| {
        AppError::MissingSecret(
            provider
                .api_key_env
                .clone()
                .unwrap_or_else(|| "DEEPSEEK_API_KEY".to_owned()),
        )
    })?;
    let headers: Vec<Header> = vec![
        ("authorization".to_owned(), format!("Bearer {api_key}")),
        ("accept".to_owned(), "application/json".to_owned()),
    ];
    let value = transport.get_json(DEEPSEEK_BALANCE_URL, &headers).await?;
    let balance: DeepSeekBalance = serde_json::from_value(value).map_err(|error| {
        AppError::UpstreamProtocol(format!(
            "DeepSeek balance response does not match the documented schema: {error}"
        ))
    })?;
    if balance.balance_infos.iter().any(|info| {
        !matches!(info.currency.as_str(), "CNY" | "USD")
            || !is_decimal_amount(&info.total_balance)
            || !is_decimal_amount(&info.granted_balance)
            || !is_decimal_amount(&info.topped_up_balance)
    }) {
        return Err(AppError::UpstreamProtocol(
            "DeepSeek balance response contains an unsupported currency or amount".to_owned(),
        ));
    }
    Ok(balance)
}

fn is_decimal_amount(value: &str) -> bool {
    let mut segments = value.split('.');
    let Some(integer) = segments.next() else {
        return false;
    };
    let fraction = segments.next();
    segments.next().is_none()
        && !integer.is_empty()
        && integer.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|fraction| {
            !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_documented_balance_amounts() {
        for value in ["0", "0.00", "110.00", "123456789.123456"] {
            assert!(is_decimal_amount(value));
        }
        for value in ["", ".5", "1.", "-1", "1e3", "1.2.3", "CNY 10"] {
            assert!(!is_decimal_amount(value));
        }
    }
}
