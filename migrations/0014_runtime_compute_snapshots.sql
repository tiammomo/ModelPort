CREATE TABLE modelport_runtime_compute_snapshots (
    adapter_id TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    observed_at TIMESTAMPTZ NOT NULL,
    observed_at_key TEXT NOT NULL,
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    document JSONB NOT NULL,
    PRIMARY KEY (adapter_id, snapshot_id),
    CONSTRAINT modelport_runtime_compute_snapshots_observation_unique
        UNIQUE (adapter_id, observed_at_key),
    CONSTRAINT modelport_runtime_compute_snapshots_identity_check CHECK (
        length(adapter_id) BETWEEN 1 AND 63
        AND length(snapshot_id) BETWEEN 1 AND 160
        AND length(observed_at_key) = 30
        AND observed_at_key ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{9}Z$'
        AND jsonb_typeof(document) = 'object'
        AND document #>> '{metadata,adapterId}' = adapter_id
        AND document #>> '{metadata,snapshotId}' = snapshot_id
        AND observed_at = (document #>> '{metadata,observedAt}')::timestamptz
        AND observed_at = observed_at_key::timestamptz
    )
);

CREATE INDEX modelport_runtime_compute_snapshots_latest_idx
    ON modelport_runtime_compute_snapshots (adapter_id, observed_at_key DESC, accepted_at DESC);
