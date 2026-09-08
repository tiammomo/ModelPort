#![allow(dead_code)] // The scheduler/admin integration follows this reviewed storage slice.

use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;

use super::{EnterpriseLedger, LedgerBackend, now_millis};
use crate::{
    AppError,
    runtime_adapter::{
        RuntimeAdapterComputeInventory, is_valid_runtime_adapter_id,
        validate_runtime_adapter_compute_inventory,
    },
};

const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const MAX_STALE_AFTER_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone)]
pub(super) struct MemoryComputeSnapshot {
    document: RuntimeAdapterComputeInventory,
    value: Value,
    observed_at_key: String,
    observed_at_ms: i64,
    accepted_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeComputeSnapshotWrite {
    Inserted,
    Idempotent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuntimeComputeFreshness {
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeComputeInventoryState {
    pub(crate) freshness: RuntimeComputeFreshness,
    pub(crate) inventory: Option<RuntimeAdapterComputeInventory>,
    pub(crate) observed_at_ms: Option<i64>,
    pub(crate) accepted_at_ms: Option<i64>,
    pub(crate) age_ms: Option<u64>,
}

struct PreparedSnapshot {
    document: RuntimeAdapterComputeInventory,
    value: Value,
    observed_at_key: String,
    observed_at_ms: i64,
    accepted_at_ms: i64,
}

impl EnterpriseLedger {
    pub(crate) async fn persist_runtime_compute_inventory(
        &self,
        inventory: &RuntimeAdapterComputeInventory,
    ) -> Result<RuntimeComputeSnapshotWrite, AppError> {
        let snapshot = prepare_snapshot(inventory, now_millis())?;
        match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => {
                let mut ledger = ledger.lock().expect("enterprise ledger lock poisoned");
                let conflicting = ledger.runtime_compute_snapshots.values().find(|existing| {
                    existing.document.metadata.adapter_id == snapshot.document.metadata.adapter_id
                        && (existing.document.metadata.snapshot_id
                            == snapshot.document.metadata.snapshot_id
                            || existing.observed_at_key == snapshot.observed_at_key)
                });
                if let Some(existing) = conflicting {
                    if existing.value == snapshot.value
                        && existing.document.metadata.snapshot_id
                            == snapshot.document.metadata.snapshot_id
                    {
                        return Ok(RuntimeComputeSnapshotWrite::Idempotent);
                    }
                    return Err(snapshot_conflict());
                }
                let key = (
                    snapshot.document.metadata.adapter_id.clone(),
                    snapshot.document.metadata.snapshot_id.clone(),
                );
                ledger.runtime_compute_snapshots.insert(
                    key,
                    MemoryComputeSnapshot {
                        document: snapshot.document,
                        value: snapshot.value,
                        observed_at_key: snapshot.observed_at_key,
                        observed_at_ms: snapshot.observed_at_ms,
                        accepted_at_ms: snapshot.accepted_at_ms,
                    },
                );
                Ok(RuntimeComputeSnapshotWrite::Inserted)
            }
            LedgerBackend::Postgres(pool) => {
                let inserted = sqlx::query_scalar::<_, String>(
                    "INSERT INTO modelport_runtime_compute_snapshots (
                        adapter_id, snapshot_id, observed_at, observed_at_key,
                        accepted_at, document
                     ) VALUES ($1, $2, $3::timestamptz, $4,
                        to_timestamp($5::double precision / 1000.0), $6)
                     ON CONFLICT DO NOTHING
                     RETURNING snapshot_id",
                )
                .bind(&snapshot.document.metadata.adapter_id)
                .bind(&snapshot.document.metadata.snapshot_id)
                .bind(&snapshot.document.metadata.observed_at)
                .bind(&snapshot.observed_at_key)
                .bind(snapshot.accepted_at_ms)
                .bind(&snapshot.value)
                .fetch_optional(pool)
                .await?;
                if inserted.is_some() {
                    return Ok(RuntimeComputeSnapshotWrite::Inserted);
                }

                let existing = sqlx::query(
                    "SELECT snapshot_id, document
                     FROM modelport_runtime_compute_snapshots
                     WHERE adapter_id = $1
                       AND (snapshot_id = $2 OR observed_at_key = $3)
                     LIMIT 1",
                )
                .bind(&snapshot.document.metadata.adapter_id)
                .bind(&snapshot.document.metadata.snapshot_id)
                .bind(&snapshot.observed_at_key)
                .fetch_optional(pool)
                .await?;
                if existing.is_some_and(|row| {
                    row.try_get::<String, _>("snapshot_id").ok()
                        == Some(snapshot.document.metadata.snapshot_id.clone())
                        && row.try_get::<Value, _>("document").ok() == Some(snapshot.value)
                }) {
                    Ok(RuntimeComputeSnapshotWrite::Idempotent)
                } else {
                    Err(snapshot_conflict())
                }
            }
        }
    }

    pub(crate) async fn latest_runtime_compute_inventory(
        &self,
        adapter_id: &str,
        stale_after: Duration,
    ) -> Result<RuntimeComputeInventoryState, AppError> {
        validate_query(adapter_id, stale_after)?;
        let latest = match self.backend.as_ref() {
            LedgerBackend::Memory(ledger) => ledger
                .lock()
                .expect("enterprise ledger lock poisoned")
                .runtime_compute_snapshots
                .values()
                .filter(|snapshot| snapshot.document.metadata.adapter_id == adapter_id)
                .max_by_key(|snapshot| (&snapshot.observed_at_key, snapshot.accepted_at_ms))
                .cloned(),
            LedgerBackend::Postgres(pool) => {
                let row = sqlx::query(
                    "SELECT document, observed_at_key,
                        (EXTRACT(EPOCH FROM accepted_at) * 1000)::bigint AS accepted_at_ms
                     FROM modelport_runtime_compute_snapshots
                     WHERE adapter_id = $1
                     ORDER BY observed_at_key DESC, accepted_at DESC
                     LIMIT 1",
                )
                .bind(adapter_id)
                .fetch_optional(pool)
                .await?;
                row.map(|row| -> Result<MemoryComputeSnapshot, AppError> {
                    let value: Value = row.try_get("document")?;
                    let document = validate_runtime_adapter_compute_inventory(&value.to_string())
                        .map_err(|_| {
                        AppError::Database(
                            "stored Runtime Adapter Compute snapshot is invalid".to_owned(),
                        )
                    })?;
                    let stored_key: String = row.try_get("observed_at_key")?;
                    let (observed_at_ms, expected_key) =
                        parse_observed_at(&document.metadata.observed_at)?;
                    if stored_key != expected_key {
                        return Err(AppError::Database(
                            "stored Runtime Adapter observation key is invalid".to_owned(),
                        ));
                    }
                    Ok(MemoryComputeSnapshot {
                        document,
                        value,
                        observed_at_key: stored_key,
                        observed_at_ms,
                        accepted_at_ms: row.try_get("accepted_at_ms")?,
                    })
                })
                .transpose()?
            }
        };
        Ok(project_freshness(latest, stale_after, now_millis()))
    }
}

fn prepare_snapshot(
    inventory: &RuntimeAdapterComputeInventory,
    accepted_at_ms: i64,
) -> Result<PreparedSnapshot, AppError> {
    let value = serde_json::to_value(inventory)?;
    let document = validate_runtime_adapter_compute_inventory(&value.to_string())?;
    let (observed_at_ms, observed_at_key) = parse_observed_at(&document.metadata.observed_at)?;
    if observed_at_ms > accepted_at_ms.saturating_add(MAX_FUTURE_SKEW_MS) {
        return Err(AppError::InvalidRequest(
            "Runtime Adapter Compute observedAt exceeds the server clock-skew allowance".to_owned(),
        ));
    }
    Ok(PreparedSnapshot {
        document,
        value,
        observed_at_key,
        observed_at_ms,
        accepted_at_ms,
    })
}

fn parse_observed_at(observed_at: &str) -> Result<(i64, String), AppError> {
    let observed_at = DateTime::parse_from_rfc3339(observed_at)
        .map_err(|_| AppError::InvalidRequest("Compute observedAt is invalid".to_owned()))?;
    Ok((
        observed_at.timestamp_millis(),
        observed_at
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Nanos, true),
    ))
}

fn validate_query(adapter_id: &str, stale_after: Duration) -> Result<(), AppError> {
    let stale_after_ms = i64::try_from(stale_after.as_millis()).unwrap_or(i64::MAX);
    if !is_valid_runtime_adapter_id(adapter_id) {
        return Err(AppError::InvalidRequest(
            "Runtime Adapter ID is invalid".to_owned(),
        ));
    }
    if !(1..=MAX_STALE_AFTER_MS).contains(&stale_after_ms) {
        return Err(AppError::InvalidRequest(
            "Runtime Adapter stale-after policy must be between 1 millisecond and 7 days"
                .to_owned(),
        ));
    }
    Ok(())
}

fn project_freshness(
    latest: Option<MemoryComputeSnapshot>,
    stale_after: Duration,
    now_ms: i64,
) -> RuntimeComputeInventoryState {
    let Some(snapshot) = latest else {
        return RuntimeComputeInventoryState {
            freshness: RuntimeComputeFreshness::Unavailable,
            inventory: None,
            observed_at_ms: None,
            accepted_at_ms: None,
            age_ms: None,
        };
    };
    let age_ms = u64::try_from(now_ms.saturating_sub(snapshot.observed_at_ms)).unwrap_or_default();
    let freshness = if u128::from(age_ms) <= stale_after.as_millis() {
        RuntimeComputeFreshness::Fresh
    } else {
        RuntimeComputeFreshness::Stale
    };
    RuntimeComputeInventoryState {
        freshness,
        inventory: Some(snapshot.document),
        observed_at_ms: Some(snapshot.observed_at_ms),
        accepted_at_ms: Some(snapshot.accepted_at_ms),
        age_ms: Some(age_ms),
    }
}

fn snapshot_conflict() -> AppError {
    AppError::StateConflict(
        "Runtime Adapter snapshot identity or observedAt was reused with different content"
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    const INVENTORY: &str = include_str!(
        "../../fixtures/runtime-adapters/qwen-llama-cpp-compute-inventory-v1alpha1.json"
    );

    #[tokio::test]
    async fn memory_snapshots_are_append_only_idempotent_and_latest_by_observation() {
        let ledger = EnterpriseLedger::memory();
        let newest = current_fixture("snapshot:qwen-newest");
        assert_eq!(
            ledger
                .persist_runtime_compute_inventory(&newest)
                .await
                .unwrap(),
            RuntimeComputeSnapshotWrite::Inserted
        );
        assert_eq!(
            ledger
                .persist_runtime_compute_inventory(&newest)
                .await
                .unwrap(),
            RuntimeComputeSnapshotWrite::Idempotent
        );

        let mut older = newest.clone();
        older.metadata.snapshot_id = "snapshot:qwen-older".to_owned();
        older.metadata.observed_at = "2026-08-23T03:59:00Z".to_owned();
        ledger
            .persist_runtime_compute_inventory(&older)
            .await
            .unwrap();
        let latest = ledger
            .latest_runtime_compute_inventory(
                "qwen-llama-cpp-reference",
                Duration::from_secs(7 * 24 * 60 * 60),
            )
            .await
            .unwrap();
        assert_eq!(latest.freshness, RuntimeComputeFreshness::Fresh);
        assert_eq!(
            latest.inventory.unwrap().metadata.snapshot_id,
            newest.metadata.snapshot_id
        );
        assert!(latest.observed_at_ms.is_some());
        assert!(latest.accepted_at_ms.is_some());
        assert!(latest.age_ms.is_some());
    }

    #[tokio::test]
    async fn memory_snapshots_reject_conflicts_and_excessive_future_time() {
        let ledger = EnterpriseLedger::memory();
        let inventory = fixture();
        ledger
            .persist_runtime_compute_inventory(&inventory)
            .await
            .unwrap();
        let mut conflict = inventory.clone();
        conflict.nodes[0].gpus[0].memory.available_bytes -= 1;
        assert!(matches!(
            ledger.persist_runtime_compute_inventory(&conflict).await,
            Err(AppError::StateConflict(_))
        ));

        let mut future = inventory;
        future.metadata.snapshot_id = "snapshot:qwen-future".to_owned();
        future.metadata.observed_at = "2999-01-01T00:00:00Z".to_owned();
        assert!(matches!(
            ledger.persist_runtime_compute_inventory(&future).await,
            Err(AppError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn freshness_is_server_owned_and_unavailable_without_a_snapshot() {
        let ledger = EnterpriseLedger::memory();
        let unavailable = ledger
            .latest_runtime_compute_inventory("missing-adapter", Duration::from_secs(120))
            .await
            .unwrap();
        assert_eq!(unavailable.freshness, RuntimeComputeFreshness::Unavailable);
        assert!(unavailable.inventory.is_none());

        ledger
            .persist_runtime_compute_inventory(&fixture())
            .await
            .unwrap();
        let stale = ledger
            .latest_runtime_compute_inventory("qwen-llama-cpp-reference", Duration::from_millis(1))
            .await
            .unwrap();
        assert_eq!(stale.freshness, RuntimeComputeFreshness::Stale);
        assert!(
            !serde_json::to_value(stale.inventory.unwrap())
                .unwrap()
                .to_string()
                .contains("freshness")
        );
        assert!(
            ledger
                .latest_runtime_compute_inventory("qwen-llama-cpp-reference", Duration::ZERO,)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn postgres_state_runtime_compute_inventory_snapshots_round_trip() {
        let Ok(database_url) = std::env::var("MODELPORT_TEST_DATABASE_URL") else {
            return;
        };
        let ledger = EnterpriseLedger::postgres_for_tests(&database_url)
            .await
            .unwrap();
        let inventory =
            current_fixture(&format!("snapshot:test-{}", uuid::Uuid::new_v4().simple()));
        let first = ledger
            .persist_runtime_compute_inventory(&inventory)
            .await
            .unwrap();
        assert!(matches!(
            first,
            RuntimeComputeSnapshotWrite::Inserted | RuntimeComputeSnapshotWrite::Idempotent
        ));
        assert_eq!(
            ledger
                .persist_runtime_compute_inventory(&inventory)
                .await
                .unwrap(),
            RuntimeComputeSnapshotWrite::Idempotent
        );
        let LedgerBackend::Postgres(pool) = ledger.backend.as_ref() else {
            unreachable!();
        };
        let stored = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM modelport_runtime_compute_snapshots
             WHERE adapter_id = $1 AND snapshot_id = $2",
        )
        .bind(&inventory.metadata.adapter_id)
        .bind(&inventory.metadata.snapshot_id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(stored, 1);
        let latest = ledger
            .latest_runtime_compute_inventory(
                "qwen-llama-cpp-reference",
                Duration::from_secs(7 * 24 * 60 * 60),
            )
            .await
            .unwrap();
        assert_eq!(latest.inventory.unwrap(), inventory);

        let mut conflict = inventory.clone();
        conflict.nodes[0].gpus[0].memory.available_bytes -= 1;
        assert!(matches!(
            ledger.persist_runtime_compute_inventory(&conflict).await,
            Err(AppError::StateConflict(_))
        ));
        assert!(
            sqlx::query(
                "INSERT INTO modelport_runtime_compute_snapshots (
                    adapter_id, snapshot_id, observed_at, observed_at_key, document
                 ) VALUES ('wrong-adapter', $1, $2::timestamptz, $3, $4)",
            )
            .bind(&inventory.metadata.snapshot_id)
            .bind(&inventory.metadata.observed_at)
            .bind(
                parse_observed_at(&inventory.metadata.observed_at)
                    .unwrap()
                    .1
            )
            .bind(serde_json::to_value(&inventory).unwrap())
            .execute(pool)
            .await
            .is_err()
        );
    }

    fn fixture() -> RuntimeAdapterComputeInventory {
        validate_runtime_adapter_compute_inventory(INVENTORY).unwrap()
    }

    fn current_fixture(snapshot_id: &str) -> RuntimeAdapterComputeInventory {
        let mut inventory = fixture();
        inventory.metadata.snapshot_id = snapshot_id.to_owned();
        inventory.metadata.observed_at =
            DateTime::<Utc>::from(SystemTime::now()).to_rfc3339_opts(SecondsFormat::Millis, true);
        inventory
    }

    #[test]
    fn canonical_observation_keys_preserve_instants_and_nanosecond_order() {
        let (_, offset) = parse_observed_at("2026-08-23T12:00:00.000000001+08:00").unwrap();
        let (_, utc) = parse_observed_at("2026-08-23T04:00:00.000000001Z").unwrap();
        let (_, later) = parse_observed_at("2026-08-23T04:00:00.000000002Z").unwrap();
        assert_eq!(offset, utc);
        assert!(later > utc);
    }
}
