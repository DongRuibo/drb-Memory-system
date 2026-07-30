BEGIN;

CREATE TABLE memory_core.outcome_receipts (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    context_pack_id UUID NOT NULL,
    task_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    purpose TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed', 'cancelled')),
    model_id TEXT NOT NULL,
    model_request_id_hash TEXT,
    latency_ms BIGINT NOT NULL CHECK (latency_ms >= 0),
    prompt_tokens BIGINT CHECK (prompt_tokens IS NULL OR prompt_tokens >= 0),
    completion_tokens BIGINT CHECK (completion_tokens IS NULL OR completion_tokens >= 0),
    total_tokens BIGINT CHECK (total_tokens IS NULL OR total_tokens >= 0),
    error_code TEXT,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, context_pack_id),
    UNIQUE (tenant_id, idempotency_key),
    FOREIGN KEY (tenant_id, context_pack_id)
        REFERENCES memory_core.context_packs (tenant_id, id),
    CHECK (char_length(idempotency_key) BETWEEN 16 AND 128),
    CHECK (char_length(btrim(purpose)) BETWEEN 1 AND 64),
    CHECK (char_length(btrim(model_id)) BETWEEN 1 AND 256),
    CHECK (model_request_id_hash IS NULL OR char_length(model_request_id_hash) = 71),
    CHECK (char_length(request_hash) = 71),
    CHECK (error_code IS NULL OR char_length(btrim(error_code)) BETWEEN 1 AND 128),
    CHECK (outcome <> 'succeeded' OR error_code IS NULL)
);

ALTER TABLE memory_core.outcome_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.outcome_receipts FORCE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON memory_core.outcome_receipts
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMIT;
