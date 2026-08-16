BEGIN;

CREATE TABLE memory_core.episodes (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    user_id TEXT NOT NULL,
    project_id TEXT,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'closed')),
    last_turn_sequence BIGINT NOT NULL DEFAULT 0 CHECK (last_turn_sequence >= 0),
    scheduled_through_sequence BIGINT NOT NULL DEFAULT 0
        CHECK (scheduled_through_sequence >= 0),
    last_consolidated_turn_sequence BIGINT NOT NULL DEFAULT 0
        CHECK (last_consolidated_turn_sequence >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id),
    CHECK (char_length(btrim(user_id)) BETWEEN 1 AND 256),
    CHECK (project_id IS NULL OR char_length(btrim(project_id)) BETWEEN 1 AND 256),
    CHECK (
        last_consolidated_turn_sequence <= scheduled_through_sequence
        AND scheduled_through_sequence <= last_turn_sequence
    ),
    CHECK (
        (status = 'active' AND closed_at IS NULL)
        OR (status = 'closed' AND closed_at IS NOT NULL)
    )
);

CREATE TABLE memory_core.completed_turns (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    episode_id UUID NOT NULL,
    sequence_no BIGINT NOT NULL CHECK (sequence_no > 0),
    task_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    user_content TEXT NOT NULL,
    assistant_content TEXT NOT NULL,
    verified_tool_results JSONB NOT NULL DEFAULT '[]'::jsonb,
    outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed', 'cancelled')),
    completed_at TIMESTAMPTZ NOT NULL,
    timezone TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, episode_id, sequence_no),
    UNIQUE (tenant_id, episode_id, attempt_id),
    UNIQUE (tenant_id, idempotency_key),
    FOREIGN KEY (tenant_id, episode_id)
        REFERENCES memory_core.episodes (tenant_id, id),
    CHECK (octet_length(user_content) BETWEEN 1 AND 20000),
    CHECK (octet_length(assistant_content) BETWEEN 1 AND 40000),
    CHECK (
        jsonb_typeof(verified_tool_results) = 'array'
        AND jsonb_array_length(verified_tool_results) <= 16
        AND octet_length(verified_tool_results::text) <= 1048576
    ),
    CHECK (char_length(timezone) BETWEEN 1 AND 64),
    CHECK (char_length(idempotency_key) BETWEEN 16 AND 128),
    CHECK (char_length(request_hash) = 71)
);

CREATE TABLE memory_core.episode_consolidations (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    episode_id UUID NOT NULL,
    from_turn_sequence BIGINT NOT NULL CHECK (from_turn_sequence > 0),
    through_turn_sequence BIGINT NOT NULL CHECK (through_turn_sequence >= from_turn_sequence),
    trigger_kind TEXT NOT NULL CHECK (trigger_kind IN ('threshold', 'manual', 'session_end')),
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'leased', 'completed', 'dead_letter')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 8),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    extractor_version TEXT,
    proposals JSONB,
    diagnostics JSONB,
    operation_ids JSONB,
    last_error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, idempotency_key),
    UNIQUE (tenant_id, episode_id, from_turn_sequence, through_turn_sequence),
    FOREIGN KEY (tenant_id, episode_id)
        REFERENCES memory_core.episodes (tenant_id, id),
    CHECK (char_length(idempotency_key) BETWEEN 16 AND 128),
    CHECK (
        (status = 'leased' AND lease_owner IS NOT NULL AND lease_until IS NOT NULL)
        OR (status <> 'leased' AND lease_owner IS NULL AND lease_until IS NULL)
    ),
    CHECK (
        (proposals IS NULL AND diagnostics IS NULL AND extractor_version IS NULL)
        OR (
            jsonb_typeof(proposals) = 'array'
            AND jsonb_array_length(proposals) <= 4
            AND jsonb_typeof(diagnostics) = 'object'
            AND char_length(btrim(extractor_version)) BETWEEN 1 AND 256
        )
    ),
    CHECK (operation_ids IS NULL OR jsonb_typeof(operation_ids) = 'array'),
    CHECK (
        (status = 'completed' AND completed_at IS NOT NULL AND operation_ids IS NOT NULL)
        OR (status <> 'completed' AND completed_at IS NULL)
    )
);

CREATE INDEX idx_episode_consolidations_claim
    ON memory_core.episode_consolidations (tenant_id, status, available_at, lease_until, created_at);
CREATE INDEX idx_completed_turns_episode
    ON memory_core.completed_turns (tenant_id, episode_id, sequence_no);

DO $rls$
DECLARE
    table_name TEXT;
BEGIN
    FOREACH table_name IN ARRAY ARRAY[
        'episodes', 'completed_turns', 'episode_consolidations'
    ] LOOP
        EXECUTE format('ALTER TABLE memory_core.%I ENABLE ROW LEVEL SECURITY', table_name);
        EXECUTE format('ALTER TABLE memory_core.%I FORCE ROW LEVEL SECURITY', table_name);
        EXECUTE format(
            'CREATE POLICY tenant_isolation ON memory_core.%I '
            'USING (tenant_id = nullif(current_setting(''app.tenant_id'', true), '''')) '
            'WITH CHECK (tenant_id = nullif(current_setting(''app.tenant_id'', true), ''''))',
            table_name
        );
    END LOOP;
END
$rls$;

COMMIT;
