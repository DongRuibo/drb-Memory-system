BEGIN;

ALTER TABLE memory_core.memory_ingest_results
    DROP CONSTRAINT memory_ingest_results_result_kind_check,
    ADD CONSTRAINT memory_ingest_results_result_kind_check
        CHECK (result_kind IN ('created', 'source_attached', 'needs_review'));

CREATE UNIQUE INDEX uq_memory_candidates_open_review_target
    ON memory_core.memory_candidates (tenant_id, target_memory_record_id)
    WHERE state='needs_review' AND target_memory_record_id IS NOT NULL;

CREATE TABLE memory_core.memory_review_results (
    tenant_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    candidate_id UUID NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('accept', 'reject')),
    memory_record_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, idempotency_key),
    UNIQUE (tenant_id, operation_id),
    FOREIGN KEY (tenant_id, candidate_id)
        REFERENCES memory_core.memory_candidates (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_record_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, memory_record_id, id),
    FOREIGN KEY (tenant_id, operation_id)
        REFERENCES memory_core.projection_operations (tenant_id, id),
    CHECK (char_length(idempotency_key) BETWEEN 16 AND 128)
);

ALTER TABLE memory_core.memory_review_results ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.memory_review_results FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.memory_review_results
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

CREATE INDEX idx_outbox_events_ordering
    ON memory_core.outbox_events
        (tenant_id, ordering_key, created_at, id)
    WHERE status IN ('pending', 'leased');

COMMIT;
