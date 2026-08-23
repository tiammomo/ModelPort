use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;

pub const RUNTIME_ADAPTER_API_VERSION: &str = "runtime.modelport.io/v1alpha1";
pub const RUNTIME_ADAPTER_CAPABILITIES_KIND: &str = "RuntimeAdapterCapabilities";
pub const RUNTIME_ADAPTER_COMPUTE_INVENTORY_KIND: &str = "RuntimeAdapterComputeInventory";
pub const RUNTIME_ADAPTER_CAPABILITIES_SCHEMA: &str =
    include_str!("../schemas/runtime-adapter-capabilities-v1alpha1.schema.json");
pub const RUNTIME_ADAPTER_COMPUTE_INVENTORY_SCHEMA: &str =
    include_str!("../schemas/runtime-adapter-compute-inventory-v1alpha1.schema.json");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RuntimeAdapterDocument {
    Capabilities(RuntimeAdapterCapabilities),
    ComputeInventory(RuntimeAdapterComputeInventory),
}

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
pub struct RuntimeAdapterComputeInventory {
    pub api_version: String,
    pub kind: String,
    pub metadata: RuntimeAdapterComputeInventoryMetadata,
    pub nodes: Vec<RuntimeAdapterComputeNode>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterComputeInventoryMetadata {
    pub adapter_id: String,
    pub snapshot_id: String,
    pub observed_at: String,
    pub source: RuntimeAdapterObservationSource,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterObservationSource {
    pub collector_id: String,
    pub collector_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterComputeNode {
    pub node_id: String,
    pub id_source: RuntimeAdapterNodeIdSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    pub architecture: String,
    pub operating_system: String,
    pub health: RuntimeAdapterResourceHealth,
    pub gpus: Vec<RuntimeAdapterGpuDevice>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterNodeIdSource {
    MachineId,
    CloudInstanceId,
    OperatorAssigned,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterGpuDevice {
    pub gpu_id: String,
    pub id_source: RuntimeAdapterGpuIdSource,
    pub device_class: RuntimeAdapterGpuDeviceClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_gpu_id: Option<String>,
    pub vendor: RuntimeAdapterGpuVendor,
    pub model: String,
    pub health: RuntimeAdapterResourceHealth,
    pub memory: RuntimeAdapterGpuMemory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_address: Option<String>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterGpuIdSource {
    VendorUuid,
    VendorSerial,
    OperatorAssigned,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterGpuDeviceClass {
    Physical,
    Partition,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterGpuVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdapterResourceHealth {
    Available,
    Degraded,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAdapterGpuMemory {
    pub total_bytes: u64,
    pub available_bytes: u64,
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
    validate_schema(&value, RUNTIME_ADAPTER_CAPABILITIES_SCHEMA, "capabilities")?;

    let document: RuntimeAdapterCapabilities = serde_json::from_value(value)?;
    validate_capability_semantics(&document)?;
    Ok(document)
}

pub fn validate_runtime_adapter_compute_inventory(
    raw: &str,
) -> Result<RuntimeAdapterComputeInventory, AppError> {
    let value: Value = serde_json::from_str(raw)?;
    validate_schema(
        &value,
        RUNTIME_ADAPTER_COMPUTE_INVENTORY_SCHEMA,
        "compute inventory",
    )?;

    let document: RuntimeAdapterComputeInventory = serde_json::from_value(value)?;
    validate_compute_inventory_semantics(&document)?;
    Ok(document)
}

pub fn validate_runtime_adapter_document(raw: &str) -> Result<RuntimeAdapterDocument, AppError> {
    let value: Value = serde_json::from_str(raw)?;
    match value.get("kind").and_then(Value::as_str) {
        Some(RUNTIME_ADAPTER_CAPABILITIES_KIND) => {
            validate_runtime_adapter_capabilities(raw).map(RuntimeAdapterDocument::Capabilities)
        }
        Some(RUNTIME_ADAPTER_COMPUTE_INVENTORY_KIND) => {
            validate_runtime_adapter_compute_inventory(raw)
                .map(RuntimeAdapterDocument::ComputeInventory)
        }
        Some(kind) => Err(AppError::InvalidRequest(format!(
            "unsupported Runtime Adapter document kind `{kind}`"
        ))),
        None => Err(AppError::InvalidRequest(
            "Runtime Adapter document kind is required".to_owned(),
        )),
    }
}

fn validate_schema(value: &Value, raw_schema: &str, label: &str) -> Result<(), AppError> {
    let schema: Value = serde_json::from_str(raw_schema)?;
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .map_err(|error| {
            AppError::InvalidRequest(format!(
                "embedded Runtime Adapter {label} schema is invalid: {error}"
            ))
        })?;
    let failures = validator
        .iter_errors(value)
        .take(5)
        .map(|error| format!("{} violates {}", error.instance_path(), error.schema_path()))
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::InvalidRequest(format!(
            "Runtime Adapter {label} does not match v1alpha1: {}",
            failures.join("; ")
        )))
    }
}

fn validate_capability_semantics(document: &RuntimeAdapterCapabilities) -> Result<(), AppError> {
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

fn validate_compute_inventory_semantics(
    document: &RuntimeAdapterComputeInventory,
) -> Result<(), AppError> {
    if document.api_version != RUNTIME_ADAPTER_API_VERSION
        || document.kind != RUNTIME_ADAPTER_COMPUTE_INVENTORY_KIND
    {
        return Err(AppError::InvalidRequest(
            "unsupported Runtime Adapter compute inventory version or kind".to_owned(),
        ));
    }

    let mut node_ids = BTreeSet::new();
    let mut gpu_ids = BTreeSet::new();
    for node in &document.nodes {
        if !node_ids.insert(node.node_id.as_str()) {
            return Err(AppError::InvalidRequest(
                "Runtime Adapter compute nodeId values must be unique".to_owned(),
            ));
        }

        let devices = node
            .gpus
            .iter()
            .map(|gpu| (gpu.gpu_id.as_str(), gpu.device_class))
            .collect::<BTreeMap<_, _>>();
        if devices.len() != node.gpus.len() {
            return Err(AppError::InvalidRequest(
                "Runtime Adapter gpuId values must be unique".to_owned(),
            ));
        }

        for gpu in &node.gpus {
            if !gpu_ids.insert(gpu.gpu_id.as_str()) {
                return Err(AppError::InvalidRequest(
                    "Runtime Adapter gpuId values must be unique across the snapshot".to_owned(),
                ));
            }
            if gpu.memory.available_bytes > gpu.memory.total_bytes {
                return Err(AppError::InvalidRequest(format!(
                    "GPU `{}` availableBytes must not exceed totalBytes",
                    gpu.gpu_id
                )));
            }
            match (gpu.device_class, gpu.parent_gpu_id.as_deref()) {
                (RuntimeAdapterGpuDeviceClass::Physical, None) => {}
                (RuntimeAdapterGpuDeviceClass::Physical, Some(_)) => {
                    return Err(AppError::InvalidRequest(format!(
                        "physical GPU `{}` must not declare parentGpuId",
                        gpu.gpu_id
                    )));
                }
                (RuntimeAdapterGpuDeviceClass::Partition, Some(parent_gpu_id))
                    if devices.get(parent_gpu_id)
                        == Some(&RuntimeAdapterGpuDeviceClass::Physical) => {}
                (RuntimeAdapterGpuDeviceClass::Partition, Some(_)) => {
                    return Err(AppError::InvalidRequest(format!(
                        "GPU partition `{}` must reference a physical GPU on the same node",
                        gpu.gpu_id
                    )));
                }
                (RuntimeAdapterGpuDeviceClass::Partition, None) => {
                    return Err(AppError::InvalidRequest(format!(
                        "GPU partition `{}` requires parentGpuId",
                        gpu.gpu_id
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const QWEN_FIXTURE: &str =
        include_str!("../fixtures/runtime-adapters/qwen-llama-cpp-capabilities-v1alpha1.json");
    const QWEN_COMPUTE_FIXTURE: &str =
        include_str!("../fixtures/runtime-adapters/qwen-llama-cpp-compute-inventory-v1alpha1.json");

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

    #[test]
    fn validates_and_round_trips_the_qwen_compute_inventory_fixture() {
        let document = validate_runtime_adapter_compute_inventory(QWEN_COMPUTE_FIXTURE).unwrap();
        assert_eq!(document.metadata.adapter_id, "qwen-llama-cpp-reference");
        assert_eq!(document.nodes.len(), 1);
        assert_eq!(document.nodes[0].gpus.len(), 1);

        let encoded = serde_json::to_string(&document).unwrap();
        assert_eq!(
            validate_runtime_adapter_compute_inventory(&encoded).unwrap(),
            document
        );
        assert!(matches!(
            validate_runtime_adapter_document(QWEN_COMPUTE_FIXTURE).unwrap(),
            RuntimeAdapterDocument::ComputeInventory(_)
        ));
    }

    #[test]
    fn rejects_invalid_observation_time_identity_source_and_extensions() {
        for (path, replacement) in [
            (
                "/metadata/observedAt",
                Value::String("2026-08-23 04:00:00".to_owned()),
            ),
            (
                "/nodes/0/gpus/0/idSource",
                Value::String("pci_address".to_owned()),
            ),
        ] {
            let mut value: Value = serde_json::from_str(QWEN_COMPUTE_FIXTURE).unwrap();
            *value.pointer_mut(path).unwrap() = replacement;
            assert!(
                validate_runtime_adapter_compute_inventory(&value.to_string()).is_err(),
                "{path} must be rejected"
            );
        }

        let mut nested_extension: Value = serde_json::from_str(QWEN_COMPUTE_FIXTURE).unwrap();
        nested_extension["nodes"][0]["extensions"]["nested"] =
            serde_json::json!({ "unbounded": true });
        assert!(validate_runtime_adapter_compute_inventory(&nested_extension.to_string()).is_err());
    }

    #[test]
    fn rejects_duplicate_ids_and_impossible_memory() {
        let mut duplicate_node: Value = serde_json::from_str(QWEN_COMPUTE_FIXTURE).unwrap();
        let node = duplicate_node["nodes"][0].clone();
        duplicate_node["nodes"].as_array_mut().unwrap().push(node);
        assert!(validate_runtime_adapter_compute_inventory(&duplicate_node.to_string()).is_err());

        let mut duplicate_gpu: Value = serde_json::from_str(QWEN_COMPUTE_FIXTURE).unwrap();
        let gpu = duplicate_gpu["nodes"][0]["gpus"][0].clone();
        duplicate_gpu["nodes"][0]["gpus"]
            .as_array_mut()
            .unwrap()
            .push(gpu);
        assert!(validate_runtime_adapter_compute_inventory(&duplicate_gpu.to_string()).is_err());

        let mut impossible_memory: Value = serde_json::from_str(QWEN_COMPUTE_FIXTURE).unwrap();
        impossible_memory["nodes"][0]["gpus"][0]["memory"]["availableBytes"] =
            Value::from(25769803777_u64);
        assert!(
            validate_runtime_adapter_compute_inventory(&impossible_memory.to_string()).is_err()
        );
    }

    #[test]
    fn accepts_only_partitions_with_a_same_node_physical_parent() {
        let mut valid: Value = serde_json::from_str(QWEN_COMPUTE_FIXTURE).unwrap();
        let parent_id = valid["nodes"][0]["gpus"][0]["gpuId"].clone();
        let mut partition = valid["nodes"][0]["gpus"][0].clone();
        partition["gpuId"] = Value::String("nvidia:MIG-GPU-partition-01".to_owned());
        partition["deviceClass"] = Value::String("partition".to_owned());
        partition["parentGpuId"] = parent_id;
        valid["nodes"][0]["gpus"]
            .as_array_mut()
            .unwrap()
            .push(partition);
        assert!(validate_runtime_adapter_compute_inventory(&valid.to_string()).is_ok());

        valid["nodes"][0]["gpus"][1]["parentGpuId"] =
            Value::String("nvidia:GPU-missing".to_owned());
        assert!(validate_runtime_adapter_compute_inventory(&valid.to_string()).is_err());
    }
}
