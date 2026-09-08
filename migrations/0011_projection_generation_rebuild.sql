BEGIN;

CREATE TABLE memory_core.projection_generations (
    tenant_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('active', 'building', 'ready', 'canary', 'standby', 'retired', 'failed')
    ),
    source_generation TEXT,
    expected_records INTEGER NOT NULL DEFAULT 0 CHECK (expected_records >= 0),
    ready_records INTEGER NOT NULL DEFAULT 0 CHECK (ready_records >= 0),
    failed_records INTEGER NOT NULL DEFAULT 0 CHECK (failed_records >= 0),
    idempotency_key TEXT,
    requested_by_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    activated_at TIMESTAMPTZ,
    retired_at TIMESTAMPTZ,
    last_error_code TEXT,
    PRIMARY KEY (tenant_id, generation),
    UNIQUE (tenant_id, idempotency_key),
    CHECK (length(btrim(generation)) BETWEEN 1 AND 128),
    CHECK (
        (status = 'active' AND activated_at IS NOT NULL)
        OR status <> 'active'
    )
);

CREATE UNIQUE INDEX uq_projection_generation_active
    ON memory_core.projection_generations (tenant_id)
    WHERE status = 'active';

CREATE UNIQUE INDEX uq_projection_generation_canary
    ON memory_core.projection_generations (tenant_id)
    WHERE status = 'canary';

CREATE UNIQUE INDEX uq_projection_generation_inflight
    ON memory_core.projection_generations (tenant_id)
    WHERE status IN ('building', 'ready', 'canary');

CREATE TABLE memory_core.projection_generation_transitions (
    tenant_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('activate', 'finalize', 'rollback', 'abort')),
    target_generation TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    result_generation TEXT NOT NULL,
    result_snapshot JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, idempotency_key),
    FOREIGN KEY (tenant_id, target_generation)
        REFERENCES memory_core.projection_generations (tenant_id, generation),
    FOREIGN KEY (tenant_id, result_generation)
        REFERENCES memory_core.projection_generations (tenant_id, generation)
);

INSERT INTO memory_core.projection_generations (
    tenant_id,
    generation,
    status,
    expected_records,
    ready_records,
    activated_at,
    completed_at
)
SELECT
    tenants.tenant_id,
    'demo-v0-generation-1',
    'active',
    counts.expected_records,
    counts.ready_records,
    now(),
    now()
FROM (
    SELECT tenant_id FROM memory_core.memory_records
    UNION
    SELECT tenant_id FROM memory_core.projection_operations
) tenants
CROSS JOIN LATERAL (
    SELECT
        count(*) FILTER (WHERE h.state = 'active')::integer AS expected_records,
        count(*) FILTER (
            WHERE h.state = 'active'
              AND EXISTS (
                  SELECT 1
                  FROM memory_core.projection_operations po
                  WHERE po.tenant_id = h.tenant_id
                    AND po.memory_record_id = h.memory_record_id
                    AND po.memory_version_id = h.current_version_id
                    AND po.generation = 'demo-v0-generation-1'
                    AND po.status = 'ready'
              )
        )::integer AS ready_records
    FROM memory_core.memory_heads h
    WHERE h.tenant_id = tenants.tenant_id
) counts
ON CONFLICT (tenant_id, generation) DO NOTHING;

ALTER TABLE memory_core.projection_generations ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.projection_generations FORCE ROW LEVEL SECURITY;
ALTER TABLE memory_core.projection_generation_transitions ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.projection_generation_transitions FORCE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON memory_core.projection_generations
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

CREATE POLICY tenant_isolation ON memory_core.projection_generation_transitions
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

ALTER TABLE memory_core.context_packs
    ADD COLUMN projection_generation TEXT NOT NULL DEFAULT 'demo-v0-generation-1';

COMMIT;
