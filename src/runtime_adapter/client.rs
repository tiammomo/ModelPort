use std::{fmt, net::IpAddr};

use reqwest::Url;

use crate::{
    AppError,
    http::HttpTransport,
    runtime_adapter::{
        RuntimeAdapterAuthenticationScheme, RuntimeAdapterCapabilities,
        RuntimeAdapterComputeInventory, RuntimeAdapterOperationId, RuntimeAdapterTransport,
        validate_runtime_adapter_capabilities, validate_runtime_adapter_compute_inventory,
    },
};

const CAPABILITIES_PATH: &str = "/runtime-adapter/v1alpha1/capabilities";
const COMPUTE_INVENTORY_PATH: &str = "/runtime-adapter/v1alpha1/inventory/compute";
const MAX_BEARER_TOKEN_BYTES: usize = 4096;

/// Trusted operator input for one independent Runtime Adapter endpoint.
///
/// The Bearer credential is intentionally private, is not serializable, and is
/// redacted from `Debug`. Persist only the environment-variable reference that
/// supplies it when configuration integration is added.
#[derive(Clone)]
pub struct RuntimeAdapterClientConfig {
    adapter_id: String,
    base_url: Url,
    bearer_token: String,
}

impl RuntimeAdapterClientConfig {
    pub fn new(
        adapter_id: impl Into<String>,
        base_url: impl AsRef<str>,
        bearer_token: impl Into<String>,
    ) -> Result<Self, AppError> {
        let adapter_id = adapter_id.into();
        validate_adapter_id(&adapter_id)?;
        let base_url = validate_base_url(base_url.as_ref())?;
        let bearer_token = bearer_token.into();
        validate_bearer_token(&bearer_token)?;
        Ok(Self {
            adapter_id,
            base_url,
            bearer_token,
        })
    }

    pub fn adapter_id(&self) -> &str {
        &self.adapter_id
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }
}

impl fmt::Debug for RuntimeAdapterClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeAdapterClientConfig")
            .field("adapter_id", &self.adapter_id)
            .field("base_url", &self.base_url.as_str())
            .field("bearer_token", &"[redacted]")
            .finish()
    }
}

/// A validated point-in-time response from one Runtime Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAdapterComputeObservation {
    pub capabilities: RuntimeAdapterCapabilities,
    pub inventory: RuntimeAdapterComputeInventory,
}

/// Read-only v1alpha1 client for Runtime Adapter Compute discovery.
#[derive(Debug, Clone)]
pub struct RuntimeAdapterClient {
    config: RuntimeAdapterClientConfig,
    transport: HttpTransport,
}

impl RuntimeAdapterClient {
    pub fn new(config: RuntimeAdapterClientConfig) -> Result<Self, AppError> {
        Ok(Self {
            config,
            transport: HttpTransport::new()?,
        })
    }

    /// Fetches capabilities first, then the advertised Compute inventory.
    /// Both documents are schema/semantics validated and identity-bound.
    pub async fn collect_compute_inventory(
        &self,
    ) -> Result<RuntimeAdapterComputeObservation, AppError> {
        let capabilities = self.fetch_capabilities().await?;
        self.validate_compute_support(&capabilities)?;

        let inventory = self
            .get_validated(
                COMPUTE_INVENTORY_PATH,
                validate_runtime_adapter_compute_inventory,
            )
            .await?;
        if inventory.metadata.adapter_id != self.config.adapter_id {
            return Err(protocol_error(
                "Compute inventory adapterId does not match the configured adapter",
            ));
        }

        Ok(RuntimeAdapterComputeObservation {
            capabilities,
            inventory,
        })
    }

    async fn fetch_capabilities(&self) -> Result<RuntimeAdapterCapabilities, AppError> {
        let capabilities = self
            .get_validated(CAPABILITIES_PATH, validate_runtime_adapter_capabilities)
            .await?;
        if capabilities.metadata.adapter_id != self.config.adapter_id {
            return Err(protocol_error(
                "capabilities adapterId does not match the configured adapter",
            ));
        }
        Ok(capabilities)
    }

    async fn get_validated<T>(
        &self,
        path: &str,
        validate: fn(&str) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let url = self.config.base_url.join(path).map_err(|_| {
            AppError::Config("Runtime Adapter endpoint URL could not be constructed".to_owned())
        })?;
        let headers = [(
            "Authorization".to_owned(),
            format!("Bearer {}", self.config.bearer_token),
        )];
        let value = self
            .transport
            .get_json(
                &format!("runtime_adapter_{}", self.config.adapter_id),
                true,
                url.as_str(),
                &headers,
            )
            .await
            .map_err(sanitize_transport_error)?;
        validate(&value.to_string()).map_err(|error| {
            protocol_error(format!("adapter returned an invalid document: {error}"))
        })
    }

    fn validate_compute_support(
        &self,
        capabilities: &RuntimeAdapterCapabilities,
    ) -> Result<(), AppError> {
        if !capabilities
            .spec
            .authentication
            .schemes
            .contains(&RuntimeAdapterAuthenticationScheme::Bearer)
        {
            return Err(protocol_error(
                "adapter does not advertise Bearer authentication",
            ));
        }
        if self.config.base_url.scheme() == "http"
            && capabilities.spec.authentication.transport != RuntimeAdapterTransport::TlsOrLoopback
        {
            return Err(protocol_error(
                "adapter requires TLS but was contacted over loopback HTTP",
            ));
        }
        if !capabilities.spec.operations.iter().any(|operation| {
            operation.operation_id == RuntimeAdapterOperationId::InventoryComputeList
        }) {
            return Err(protocol_error(
                "adapter does not advertise inventory.compute.list",
            ));
        }
        Ok(())
    }
}

fn validate_adapter_id(adapter_id: &str) -> Result<(), AppError> {
    let bytes = adapter_id.as_bytes();
    let edge = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if !(1..=63).contains(&bytes.len())
        || !edge(bytes[0])
        || !edge(bytes[bytes.len() - 1])
        || !bytes
            .iter()
            .all(|byte| edge(*byte) || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AppError::Config(
            "Runtime Adapter ID must match the v1alpha1 identifier format".to_owned(),
        ));
    }
    Ok(())
}

fn validate_base_url(raw: &str) -> Result<Url, AppError> {
    let url = Url::parse(raw)
        .map_err(|_| AppError::Config("Runtime Adapter base URL is invalid".to_owned()))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(AppError::Config(
            "Runtime Adapter base URL must be an origin without credentials, path, query, or fragment"
                .to_owned(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::Config("Runtime Adapter base URL requires a host".to_owned()))?
        .trim_matches(['[', ']']);
    match url.scheme() {
        "https" => {}
        "http" if host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) => {}
        "http" => {
            return Err(AppError::Config(
                "plain HTTP Runtime Adapters must use a literal loopback address".to_owned(),
            ));
        }
        _ => {
            return Err(AppError::Config(
                "Runtime Adapter base URL scheme must be https, or http on literal loopback"
                    .to_owned(),
            ));
        }
    }
    Ok(url)
}

fn validate_bearer_token(token: &str) -> Result<(), AppError> {
    let unpadded = token.trim_end_matches('=');
    if token.len() > MAX_BEARER_TOKEN_BYTES
        || unpadded.is_empty()
        || !unpadded.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        })
    {
        return Err(AppError::Config(
            "Runtime Adapter Bearer credential is not a valid RFC 6750 b64token".to_owned(),
        ));
    }
    Ok(())
}

fn protocol_error(message: impl Into<String>) -> AppError {
    AppError::UpstreamProtocol(format!("Runtime Adapter {}", message.into()))
}

fn sanitize_transport_error(error: AppError) -> AppError {
    match error {
        AppError::Config(_) => {
            AppError::Config("Runtime Adapter request configuration is invalid".to_owned())
        }
        AppError::Forbidden(_) => AppError::Forbidden(
            "Runtime Adapter endpoint was rejected by network policy".to_owned(),
        ),
        AppError::InvalidRequest(_) => {
            AppError::InvalidRequest("Runtime Adapter endpoint is invalid".to_owned())
        }
        AppError::Transport(_) => AppError::Transport("Runtime Adapter request failed".to_owned()),
        AppError::Upstream {
            status,
            retry_after_secs,
            ..
        } => AppError::Upstream {
            status,
            body: "Runtime Adapter response body [redacted]".to_owned(),
            retry_after_secs,
        },
        AppError::UpstreamProtocol(_) => {
            AppError::UpstreamProtocol("Runtime Adapter returned an invalid response".to_owned())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Router,
        extract::State,
        http::{HeaderMap, header::CONTENT_TYPE},
        routing::get,
    };
    use serde_json::Value;
    use tokio::net::TcpListener;

    use super::*;

    const CAPABILITIES: &str =
        include_str!("../../fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json");
    const INVENTORY: &str = include_str!(
        "../../fixtures/runtime-adapters/qwen-llama-cpp-compute-inventory-v1alpha1.json"
    );

    #[derive(Clone)]
    struct AdapterState {
        capabilities: String,
        inventory: String,
        authorizations: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[tokio::test]
    async fn collects_valid_identity_bound_documents_with_bearer_auth() {
        let (base_url, authorizations) = spawn_adapter(CAPABILITIES, INVENTORY).await;
        let secret = "adapter-secret_123";
        let config =
            RuntimeAdapterClientConfig::new("qwen-llama-cpp-reference", &base_url, secret).unwrap();
        assert!(!format!("{config:?}").contains(secret));

        let client = RuntimeAdapterClient::new(config).unwrap();
        assert!(!format!("{client:?}").contains(secret));
        let observation = client.collect_compute_inventory().await.unwrap();

        assert_eq!(
            observation.capabilities.metadata.adapter_id,
            "qwen-llama-cpp-reference"
        );
        assert_eq!(observation.inventory.nodes.len(), 1);
        assert_eq!(
            *authorizations.lock().unwrap(),
            vec![
                Some(format!("Bearer {secret}")),
                Some(format!("Bearer {secret}"))
            ]
        );
    }

    #[tokio::test]
    async fn rejects_missing_capability_before_fetching_inventory() {
        let mut capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
        capabilities["spec"]["operations"]
            .as_array_mut()
            .unwrap()
            .retain(|operation| operation["operationId"] != "inventory.compute.list");
        let (base_url, authorizations) = spawn_adapter(&capabilities.to_string(), INVENTORY).await;
        let client = client(&base_url);

        let error = client.collect_compute_inventory().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not advertise inventory.compute.list")
        );
        assert_eq!(authorizations.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rejects_capability_and_inventory_identity_mismatches() {
        let mut capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
        capabilities["metadata"]["adapterId"] = Value::String("another-adapter".to_owned());
        let (base_url, _) = spawn_adapter(&capabilities.to_string(), INVENTORY).await;
        assert!(
            client(&base_url)
                .collect_compute_inventory()
                .await
                .unwrap_err()
                .to_string()
                .contains("capabilities adapterId")
        );

        let mut inventory: Value = serde_json::from_str(INVENTORY).unwrap();
        inventory["metadata"]["adapterId"] = Value::String("another-adapter".to_owned());
        let (base_url, _) = spawn_adapter(CAPABILITIES, &inventory.to_string()).await;
        assert!(
            client(&base_url)
                .collect_compute_inventory()
                .await
                .unwrap_err()
                .to_string()
                .contains("Compute inventory adapterId")
        );
    }

    #[tokio::test]
    async fn rejects_invalid_inventory_as_an_upstream_protocol_error() {
        let mut inventory: Value = serde_json::from_str(INVENTORY).unwrap();
        inventory["nodes"][0]["gpus"][0]["memory"]["availableBytes"] =
            Value::from(99_999_999_999_u64);
        let (base_url, _) = spawn_adapter(CAPABILITIES, &inventory.to_string()).await;

        assert!(matches!(
            client(&base_url).collect_compute_inventory().await,
            Err(AppError::UpstreamProtocol(_))
        ));
    }

    #[test]
    fn accepts_https_or_literal_loopback_http_only() {
        assert!(
            RuntimeAdapterClientConfig::new("adapter", "https://adapter.example", "token").is_ok()
        );
        assert!(
            RuntimeAdapterClientConfig::new("adapter", "http://127.0.0.1:9090", "token").is_ok()
        );
        for url in [
            "http://adapter.example",
            "http://localhost:9090",
            "ftp://127.0.0.1",
            "https://user@adapter.example",
            "https://adapter.example/prefix",
            "https://adapter.example?token=secret",
        ] {
            assert!(
                RuntimeAdapterClientConfig::new("adapter", url, "token").is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn validates_ids_and_rfc_6750_tokens_without_disclosing_them() {
        for adapter_id in ["", "Uppercase", "trailing-"] {
            assert!(
                RuntimeAdapterClientConfig::new(adapter_id, "https://a.example", "token").is_err()
            );
        }
        for token in ["", "contains space", "contains:colon"] {
            let error =
                RuntimeAdapterClientConfig::new("adapter", "https://a.example", token).unwrap_err();
            assert!(!error.to_string().contains(token) || token.is_empty());
        }
    }

    #[tokio::test]
    async fn enforces_the_advertised_transport_and_redacts_error_bodies() {
        let mut capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
        capabilities["spec"]["authentication"]["transport"] =
            Value::String("tls_required".to_owned());
        let (base_url, _) = spawn_adapter(&capabilities.to_string(), INVENTORY).await;
        assert!(
            client(&base_url)
                .collect_compute_inventory()
                .await
                .unwrap_err()
                .to_string()
                .contains("requires TLS")
        );

        let secret = "adapter-secret_123";
        let app = Router::new().route(
            CAPABILITIES_PATH,
            get(move || async move { (axum::http::StatusCode::BAD_REQUEST, secret) }),
        );
        let base_url = spawn(app).await;
        let error = client(&base_url)
            .collect_compute_inventory()
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(secret));
    }

    fn client(base_url: &str) -> RuntimeAdapterClient {
        RuntimeAdapterClient::new(
            RuntimeAdapterClientConfig::new(
                "qwen-llama-cpp-reference",
                base_url,
                "adapter-secret_123",
            )
            .unwrap(),
        )
        .unwrap()
    }

    async fn spawn_adapter(
        capabilities: &str,
        inventory: &str,
    ) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let authorizations = Arc::new(Mutex::new(Vec::new()));
        let state = AdapterState {
            capabilities: capabilities.to_owned(),
            inventory: inventory.to_owned(),
            authorizations: Arc::clone(&authorizations),
        };
        let app = Router::new()
            .route(CAPABILITIES_PATH, get(capabilities_handler))
            .route(COMPUTE_INVENTORY_PATH, get(inventory_handler))
            .with_state(state);
        (spawn(app).await, authorizations)
    }

    async fn capabilities_handler(
        State(state): State<AdapterState>,
        headers: HeaderMap,
    ) -> ([(axum::http::HeaderName, &'static str); 1], String) {
        record_auth(&state, &headers);
        ([(CONTENT_TYPE, "application/json")], state.capabilities)
    }

    async fn inventory_handler(
        State(state): State<AdapterState>,
        headers: HeaderMap,
    ) -> ([(axum::http::HeaderName, &'static str); 1], String) {
        record_auth(&state, &headers);
        ([(CONTENT_TYPE, "application/json")], state.inventory)
    }

    fn record_auth(state: &AdapterState, headers: &HeaderMap) {
        state.authorizations.lock().unwrap().push(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        );
    }

    async fn spawn(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }
}
