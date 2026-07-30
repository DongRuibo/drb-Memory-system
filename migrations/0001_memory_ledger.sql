BEGIN;

CREATE SCHEMA IF NOT EXISTS memory_core;
CREATE SCHEMA IF NOT EXISTS memory_search;

CREATE TABLE memory_core.source_events (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    source_type TEXT NOT NULL CHECK (source_type IN (
        'conversation_message', 'tool_result', 'task_result',
        'user_correction', 'imported_record', 'reflection'
    )),
    source_ref TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    redacted_excerpt TEXT,
    classification TEXT NOT NULL CHECK (classification IN (
        'public', 'internal', 'personal', 'sensitive', 'secret'
    )),
    consent_ref TEXT,
    occurred_at TIMESTAMPTZ NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    idempotency_key TEXT NOT NULL,
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, idempotency_key),
    UNIQUE (tenant_id, source_type, source_ref, content_hash)
);

CREATE TABLE memory_core.memory_candidates (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('tenant', 'user', 'project', 'agent')),
    scope_id TEXT,
    memory_type TEXT NOT NULL CHECK (memory_type IN (
        'preference', 'profile_fact', 'project_fact',
        'episodic_summary', 'procedural_candidate'
    )),
    content TEXT NOT NULL,
    normalized_key TEXT,
    confidence DOUBLE PRECISION NOT NULL CHECK (confidence BETWEEN 0 AND 1),
    importance DOUBLE PRECISION NOT NULL CHECK (importance BETWEEN 0 AND 1),
    inference_kind TEXT NOT NULL CHECK (inference_kind IN ('explicit', 'derived')),
    extractor_name TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'needs_review', 'accepted', 'rejected')),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    CHECK (
        (scope_kind = 'tenant' AND scope_id IS NULL)
        OR (scope_kind <> 'tenant' AND scope_id IS NOT NULL AND length(btrim(scope_id)) > 0)
    )
);

CREATE TABLE memory_core.candidate_sources (
    tenant_id TEXT NOT NULL,
    candidate_id UUID NOT NULL,
    source_event_id UUID NOT NULL,
    PRIMARY KEY (tenant_id, candidate_id, source_event_id),
    FOREIGN KEY (tenant_id, candidate_id)
        REFERENCES memory_core.memory_candidates (tenant_id, id),
    FOREIGN KEY (tenant_id, source_event_id)
        REFERENCES memory_core.source_events (tenant_id, id)
);

CREATE TABLE memory_core.memory_decisions (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    candidate_id UUID,
    outcome TEXT NOT NULL CHECK (outcome IN ('accept', 'reject', 'needs_review')),
    reason_code TEXT NOT NULL,
    actor_type TEXT NOT NULL CHECK (actor_type IN ('policy', 'user', 'reviewer', 'system')),
    actor_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    FOREIGN KEY (tenant_id, candidate_id)
        REFERENCES memory_core.memory_candidates (tenant_id, id)
);

CREATE UNIQUE INDEX uq_memory_decisions_candidate
    ON memory_core.memory_decisions (tenant_id, candidate_id)
    WHERE candidate_id IS NOT NULL AND outcome IN ('accept', 'reject');

CREATE TABLE memory_core.memory_records (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('tenant', 'user', 'project', 'agent')),
    scope_id TEXT,
    memory_type TEXT NOT NULL CHECK (memory_type IN (
        'preference', 'profile_fact', 'project_fact',
        'episodic_summary', 'procedural_candidate'
    )),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    CHECK (
        (scope_kind = 'tenant' AND scope_id IS NULL)
        OR (scope_kind <> 'tenant' AND scope_id IS NOT NULL AND length(btrim(scope_id)) > 0)
    )
);

CREATE TABLE memory_core.memory_versions (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    memory_record_id UUID NOT NULL,
    version_no INTEGER NOT NULL CHECK (version_no > 0),
    content TEXT,
    content_hash TEXT NOT NULL,
    normalized_key TEXT,
    confidence DOUBLE PRECISION NOT NULL CHECK (confidence BETWEEN 0 AND 1),
    importance DOUBLE PRECISION NOT NULL CHECK (importance BETWEEN 0 AND 1),
    inference_kind TEXT NOT NULL CHECK (inference_kind IN ('explicit', 'derived')),
    effective_from TIMESTAMPTZ,
    effective_to TIMESTAMPTZ,
    classification TEXT NOT NULL CHECK (classification IN (
        'public', 'internal', 'personal', 'sensitive'
    )),
    allowed_purposes TEXT[] NOT NULL DEFAULT '{}',
    consent_ref TEXT,
    retention_policy_id TEXT NOT NULL,
    retain_until TIMESTAMPTZ,
    decision_id UUID NOT NULL,
    content_erased_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, memory_record_id, version_no),
    UNIQUE (tenant_id, memory_record_id, id),
    FOREIGN KEY (tenant_id, memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id),
    FOREIGN KEY (tenant_id, decision_id)
        REFERENCES memory_core.memory_decisions (tenant_id, id),
    CHECK (effective_to IS NULL OR effective_from IS NULL OR effective_to > effective_from),
    CHECK (
        (content IS NOT NULL AND content_erased_at IS NULL)
        OR (content IS NULL AND content_erased_at IS NOT NULL)
    )
);

CREATE TABLE memory_core.memory_version_sources (
    tenant_id TEXT NOT NULL,
    memory_version_id UUID NOT NULL,
    source_event_id UUID NOT NULL,
    PRIMARY KEY (tenant_id, memory_version_id, source_event_id),
    FOREIGN KEY (tenant_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, id),
    FOREIGN KEY (tenant_id, source_event_id)
        REFERENCES memory_core.source_events (tenant_id, id)
);

CREATE TABLE memory_core.memory_events (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    memory_record_id UUID NOT NULL,
    sequence_no BIGINT NOT NULL CHECK (sequence_no > 0),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'memory_accepted', 'memory_corrected', 'memory_contested',
        'memory_contest_resolved', 'memory_expired', 'memory_forgotten',
        'memory_erased'
    )),
    memory_version_id UUID,
    actor_type TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, memory_record_id, sequence_no),
    FOREIGN KEY (tenant_id, memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, id)
);

CREATE TABLE memory_core.memory_heads (
    tenant_id TEXT NOT NULL,
    memory_record_id UUID NOT NULL,
    current_version_id UUID,
    state TEXT NOT NULL CHECK (state IN ('active', 'contested', 'expired', 'forgotten', 'erased')),
    revision BIGINT NOT NULL CHECK (revision >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, memory_record_id),
    FOREIGN KEY (tenant_id, memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_record_id, current_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, memory_record_id, id),
    CHECK (current_version_id IS NOT NULL OR state IN ('forgotten', 'erased'))
);

CREATE TABLE memory_core.tombstones (
    tenant_id TEXT NOT NULL,
    memory_record_id UUID NOT NULL,
    tombstone_revision BIGINT NOT NULL CHECK (tombstone_revision >= 0),
    minimum_generation TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, memory_record_id),
    FOREIGN KEY (tenant_id, memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id)
);

CREATE TABLE memory_core.projection_operations (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    memory_record_id UUID NOT NULL,
    memory_version_id UUID,
    head_revision BIGINT NOT NULL CHECK (head_revision >= 0),
    provider TEXT NOT NULL,
    generation TEXT NOT NULL,
    operation_type TEXT NOT NULL CHECK (operation_type IN ('upsert', 'delete')),
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'leased', 'ready', 'failed', 'stale', 'deleted', 'dead_letter')),
    provider_object_id TEXT,
    provider_revision TEXT,
    content_hash TEXT,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    last_error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, provider, idempotency_key),
    FOREIGN KEY (tenant_id, memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, id)
);

CREATE INDEX idx_projection_operations_claim
    ON memory_core.projection_operations (status, available_at, lease_until);

CREATE TABLE memory_core.outbox_events (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    aggregate_revision BIGINT NOT NULL CHECK (aggregate_revision >= 0),
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    ordering_key TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'leased', 'processed', 'dead_letter')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX idx_outbox_claim
    ON memory_core.outbox_events (status, available_at, lease_until);

CREATE TABLE memory_core.context_packs (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    task_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    purpose TEXT NOT NULL,
    authority_snapshot_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    compiler_version TEXT NOT NULL,
    query_hash TEXT NOT NULL,
    tokenizer_id TEXT NOT NULL,
    token_budget INTEGER NOT NULL CHECK (token_budget >= 0),
    estimated_tokens INTEGER NOT NULL CHECK (estimated_tokens >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    invalidated_at TIMESTAMPTZ,
    invalidation_reason TEXT,
    PRIMARY KEY (tenant_id, id),
    CHECK (expires_at > created_at)
);

CREATE TABLE memory_core.context_pack_items (
    tenant_id TEXT NOT NULL,
    context_pack_id UUID NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    memory_record_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    section_kind TEXT NOT NULL,
    inclusion_reason TEXT NOT NULL,
    content_snapshot TEXT,
    source_refs JSONB NOT NULL,
    PRIMARY KEY (tenant_id, context_pack_id, position),
    UNIQUE (tenant_id, context_pack_id, memory_version_id),
    FOREIGN KEY (tenant_id, context_pack_id)
        REFERENCES memory_core.context_packs (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_record_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, memory_record_id, id)
);

CREATE TABLE memory_core.usage_receipts (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    context_pack_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    result TEXT NOT NULL CHECK (result IN ('considered', 'injected', 'omitted')),
    reason_code TEXT NOT NULL,
    retrieval_score DOUBLE PRECISION,
    rank INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    FOREIGN KEY (tenant_id, context_pack_id)
        REFERENCES memory_core.context_packs (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, id)
);

CREATE TABLE memory_core.erasure_requests (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('memory_record', 'user', 'project')),
    target_id TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'processing', 'complete', 'failed')),
    deadline_at TIMESTAMPTZ NOT NULL,
    handler_status JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id)
);

CREATE TABLE memory_core.audit_events (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    actor_type TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    request_id TEXT NOT NULL,
    trace_id TEXT,
    reason_code TEXT NOT NULL,
    policy_version TEXT,
    details JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id)
);

CREATE TABLE memory_search.retrieval_documents (
    tenant_id TEXT NOT NULL,
    memory_record_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    namespace_key TEXT NOT NULL,
    generation TEXT NOT NULL,
    head_revision BIGINT NOT NULL CHECK (head_revision >= 0),
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    search_vector TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple', content)) STORED,
    indexed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, memory_version_id, generation),
    FOREIGN KEY (tenant_id, memory_record_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, memory_record_id, id)
);

CREATE INDEX idx_retrieval_documents_fts
    ON memory_search.retrieval_documents USING GIN (search_vector);

DO $rls$
DECLARE
    table_name TEXT;
BEGIN
    FOREACH table_name IN ARRAY ARRAY[
        'source_events', 'memory_candidates', 'candidate_sources',
        'memory_decisions', 'memory_records', 'memory_versions',
        'memory_version_sources', 'memory_events', 'memory_heads',
        'tombstones', 'projection_operations', 'outbox_events',
        'context_packs', 'context_pack_items', 'usage_receipts',
        'erasure_requests', 'audit_events'
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

    ALTER TABLE memory_search.retrieval_documents ENABLE ROW LEVEL SECURITY;
    ALTER TABLE memory_search.retrieval_documents FORCE ROW LEVEL SECURITY;
    CREATE POLICY tenant_isolation ON memory_search.retrieval_documents
        USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
        WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
END
$rls$;

COMMIT;
