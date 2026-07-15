CREATE TABLE modelport_organizations (
    organization_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE modelport_projects (
    organization_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, project_id),
    FOREIGN KEY (organization_id)
        REFERENCES modelport_organizations (organization_id)
        ON DELETE RESTRICT
);

CREATE TABLE modelport_environments (
    organization_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    environment_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, project_id, environment_id),
    FOREIGN KEY (organization_id, project_id)
        REFERENCES modelport_projects (organization_id, project_id)
        ON DELETE RESTRICT
);

INSERT INTO modelport_organizations (organization_id, display_name)
VALUES ('org_local', 'Local organization')
ON CONFLICT (organization_id) DO NOTHING;

INSERT INTO modelport_projects (organization_id, project_id, display_name)
VALUES ('org_local', 'prj_default', 'Default project')
ON CONFLICT (organization_id, project_id) DO NOTHING;

INSERT INTO modelport_environments (
    organization_id,
    project_id,
    environment_id,
    display_name
)
VALUES ('org_local', 'prj_default', 'env_default', 'Default environment')
ON CONFLICT (organization_id, project_id, environment_id) DO NOTHING;

CREATE TABLE modelport_gateway_requests (
    ledger_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    organization_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    environment_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    client_protocol TEXT NOT NULL,
    requested_model TEXT NOT NULL,
    stream BOOLEAN NOT NULL,
    state TEXT NOT NULL DEFAULT 'started',
    status_code INTEGER,
    terminal_reason TEXT,
    error_message TEXT,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_write_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cost_amount_microunits BIGINT NOT NULL DEFAULT 0,
    currency CHAR(3) NOT NULL DEFAULT 'USD',
    billing_mode TEXT,
    chargeable BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT modelport_gateway_requests_state_check
        CHECK (state IN ('started', 'completed', 'failed', 'cancelled')),
    CONSTRAINT modelport_gateway_requests_status_code_check
        CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599),
    CONSTRAINT modelport_gateway_requests_usage_check
        CHECK (
            input_tokens >= 0
            AND output_tokens >= 0
            AND cache_write_tokens >= 0
            AND cache_read_tokens >= 0
            AND cost_amount_microunits >= 0
        ),
    CONSTRAINT modelport_gateway_requests_tenant_ledger_unique
        UNIQUE (organization_id, project_id, environment_id, ledger_id),
    FOREIGN KEY (organization_id, project_id, environment_id)
        REFERENCES modelport_environments (
            organization_id,
            project_id,
            environment_id
        )
        ON DELETE RESTRICT
);

CREATE INDEX modelport_gateway_requests_tenant_created_idx
    ON modelport_gateway_requests (
        organization_id,
        project_id,
        environment_id,
        created_at DESC
    );

CREATE INDEX modelport_gateway_requests_correlation_idx
    ON modelport_gateway_requests (
        organization_id,
        project_id,
        environment_id,
        request_id
    );

CREATE INDEX modelport_gateway_requests_incomplete_idx
    ON modelport_gateway_requests (updated_at)
    WHERE state = 'started';

CREATE TABLE modelport_provider_attempts (
    attempt_id TEXT PRIMARY KEY,
    request_ledger_id TEXT NOT NULL,
    organization_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    environment_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    resolved_model TEXT NOT NULL,
    provider_protocol TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'started',
    status_code INTEGER,
    terminal_reason TEXT,
    error_message TEXT,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_write_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cost_amount_microunits BIGINT NOT NULL DEFAULT 0,
    currency CHAR(3) NOT NULL DEFAULT 'USD',
    billing_mode TEXT,
    chargeable BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT modelport_provider_attempts_state_check
        CHECK (state IN ('started', 'completed', 'failed', 'cancelled')),
    CONSTRAINT modelport_provider_attempts_status_code_check
        CHECK (status_code IS NULL OR status_code BETWEEN 100 AND 599),
    CONSTRAINT modelport_provider_attempts_usage_check
        CHECK (
            input_tokens >= 0
            AND output_tokens >= 0
            AND cache_write_tokens >= 0
            AND cache_read_tokens >= 0
            AND cost_amount_microunits >= 0
        ),
    FOREIGN KEY (
        organization_id,
        project_id,
        environment_id,
        request_ledger_id
    ) REFERENCES modelport_gateway_requests (
        organization_id,
        project_id,
        environment_id,
        ledger_id
    ) ON DELETE RESTRICT
);

CREATE INDEX modelport_provider_attempts_request_idx
    ON modelport_provider_attempts (
        organization_id,
        project_id,
        environment_id,
        request_ledger_id,
        created_at
    );

CREATE INDEX modelport_provider_attempts_tenant_created_idx
    ON modelport_provider_attempts (
        organization_id,
        project_id,
        environment_id,
        created_at DESC
    );

CREATE INDEX modelport_provider_attempts_incomplete_idx
    ON modelport_provider_attempts (updated_at)
    WHERE state = 'started';
