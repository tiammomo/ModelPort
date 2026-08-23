use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;

pub const RUNTIME_ADAPTER_API_VERSION: &str = "runtime.modelport.io/v1alpha1";
pub const RUNTIME_ADAPTER_CAPABILITIES_KIND: &str = "RuntimeAdapterCapabilities";
pub const RUNTIME_ADAPTER_CAPABILITIES_SCHEMA: &str =
    include_str!("../schemas/runtime-adapter-capabilities-v1alpha1.schema.json");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterCapabilities {
    pub api_version: String,
    pub kind: String,
    pub metadata: RuntimeAdapterMetadata,
    pub spec: RuntimeAdapterCapabilitySpec,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterMetadata {
    pub adapter_id: String,
    pub display_name: String,
    pub adapter_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterCapabilitySpec {
    pub authentication: RuntimeAdapterAuthentication,
    pub runtime_engines: Vec<String>,
    pub inference_protocols: Vec<RuntimeAdapterInferenceProtocol>,
    pub inventory_kinds: Vec<RuntimeAdapterInventoryKind>,
    pub operations: Vec<RuntimeAdapterOperation>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterAuthentication {
    pub schemes: Vec<RuntimeAdapterAuthenticationScheme>,
    pub transport: RuntimeAdapterTransport,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterAuthenticationScheme {
    Bearer,
    MutualTls,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterTransport {
    TlsRequired,
    TlsOrLoopback,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterInferenceProtocol {
    pub id: RuntimeAdapterInferenceProtocolId,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterInferenceProtocolId {
    OpenaiCompatible,
    Anthropic,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterInventoryKind {
    Model,
    ComputeNode,
    Gpu,
    Deployment,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterOperation {
    pub operation_id: RuntimeAdapterOperationId,
    pub method: RuntimeAdapterHttpMethod,
    pub path: String,
    pub side_effect_free: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeAdapterOperationId {
    #[serde(rename = "capabilities.get")]
    CapabilitiesGet,
    #[serde(rename = "health.get")]
    HealthGet,
    #[serde(rename = "inventory.models.list")]
    InventoryModelsList,
    #[serde(rename = "inventory.compute.list")]
    InventoryComputeList,
    #[serde(rename = "inventory.deployments.list")]
    InventoryDeploymentsList,
}

impl RuntimeAdapterOperationId {
    fn expected_path(self) -> &'static str {
        match self {
            Self::CapabilitiesGet => "/runtime-adapter/v1alpha1/capabilities",
            Self::HealthGet => "/runtime-adapter/v1alpha1/health",
            Self::InventoryModelsList => "/runtime-adapter/v1alpha1/inventory/models",
            Self::InventoryComputeList => "/runtime-adapter/v1alpha1/inventory/compute",
            Self::InventoryDeploymentsList => "/runtime-adapter/v1alpha1/inventory/deployments",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum RuntimeAdapterHttpMethod {
    #[serde(rename = "GET")]
    Get,
}

pub fn validate_runtime_adapter_capabilities(
    raw: &str,
) -> Result<RuntimeAdapterCapabilities, AppError> {
    let value: Value = serde_json::from_str(raw)?;
    let schema: Value = serde_json::from_str(RUNTIME_ADAPTER_CAPABILITIES_SCHEMA)?;
    let validator = jsonschema::validator_for(&schema).map_err(|error| {
        AppError::InvalidRequest(format!(
            "embedded Runtime Adapter capability schema is invalid: {error}"
        ))
    })?;
    let failures = validator
        .iter_errors(&value)
        .take(5)
        .map(|error| format!("{} violates {}", error.instance_path(), error.schema_path()))
        .collect::<Vec<_>>();
    if !failures.is_empty() {
        return Err(AppError::InvalidRequest(format!(
            "Runtime Adapter capabilities do not match v1alpha1: {}",
            failures.join("; ")
        )));
    }

    let document: RuntimeAdapterCapabilities = serde_json::from_value(value)?;
    validate_semantics(&document)?;
    Ok(document)
}

fn validate_semantics(document: &RuntimeAdapterCapabilities) -> Result<(), AppError> {
    if document.api_version != RUNTIME_ADAPTER_API_VERSION
        || document.kind != RUNTIME_ADAPTER_CAPABILITIES_KIND
    {
        return Err(AppError::InvalidRequest(
            "unsupported Runtime Adapter capability version or kind".to_owned(),
        ));
    }

    let mut operation_ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for operation in &document.spec.operations {
        if !operation_ids.insert(operation.operation_id) {
            return Err(AppError::InvalidRequest(
                "Runtime Adapter operationId values must be unique".to_owned(),
            ));
        }
        if !paths.insert(operation.path.as_str()) {
            return Err(AppError::InvalidRequest(
                "Runtime Adapter operation paths must be unique".to_owned(),
            ));
        }
        if operation.method != RuntimeAdapterHttpMethod::Get || !operation.side_effect_free {
            return Err(AppError::InvalidRequest(
                "v1alpha1 Runtime Adapter operations must be side-effect-free GET requests"
                    .to_owned(),
            ));
        }
        if operation.path != operation.operation_id.expected_path() {
            return Err(AppError::InvalidRequest(format!(
                "Runtime Adapter operation path does not match {:?}",
                operation.operation_id
            )));
        }
    }
    if !operation_ids.contains(&RuntimeAdapterOperationId::CapabilitiesGet) {
        return Err(AppError::InvalidRequest(
            "Runtime Adapter capabilities.get operation is required".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const QWEN_FIXTURE: &str =
        include_str!("../fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json");

    #[test]
    fn validates_and_round_trips_the_qwen_reference_fixture() {
        let document = validate_runtime_adapter_capabilities(QWEN_FIXTURE).unwrap();
        assert_eq!(document.metadata.adapter_id, "qwen-llama-cpp-reference");
        assert_eq!(document.spec.operations.len(), 5);

        let encoded = serde_json::to_string(&document).unwrap();
        assert_eq!(
            validate_runtime_adapter_capabilities(&encoded).unwrap(),
            document
        );
    }

    #[test]
    fn rejects_unknown_versions_mutations_and_unknown_fields() {
        for (path, replacement) in [
            (
                "/apiVersion",
                Value::String("runtime.modelport.io/v2".to_owned()),
            ),
            (
                "/spec/operations/0/method",
                Value::String("POST".to_owned()),
            ),
            ("/spec/operations/0/sideEffectFree", Value::Bool(false)),
        ] {
            let mut value: Value = serde_json::from_str(QWEN_FIXTURE).unwrap();
            *value.pointer_mut(path).unwrap() = replacement;
            assert!(
                validate_runtime_adapter_capabilities(&value.to_string()).is_err(),
                "{path} must be rejected"
            );
        }

        let mut value: Value = serde_json::from_str(QWEN_FIXTURE).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_owned(), Value::Bool(true));
        assert!(validate_runtime_adapter_capabilities(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_duplicate_operation_ids_and_mismatched_paths() {
        let mut duplicate: Value = serde_json::from_str(QWEN_FIXTURE).unwrap();
        duplicate["spec"]["operations"][1]["operationId"] =
            Value::String("capabilities.get".to_owned());
        duplicate["spec"]["operations"][1]["path"] =
            Value::String("/runtime-adapter/v1alpha1/capabilities".to_owned());
        assert!(validate_runtime_adapter_capabilities(&duplicate.to_string()).is_err());

        let mut wrong_path: Value = serde_json::from_str(QWEN_FIXTURE).unwrap();
        wrong_path["spec"]["operations"][0]["path"] =
            Value::String("/runtime-adapter/v1alpha1/health".to_owned());
        assert!(validate_runtime_adapter_capabilities(&wrong_path.to_string()).is_err());
    }
}
