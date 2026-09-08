BEGIN;

CREATE TABLE memory_core.extraction_operations (
    tenant_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'completed')),
    proposals JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, idempotency_key),
    CHECK (char_length(idempotency_key) BETWEEN 16 AND 128),
    CHECK (char_length(request_hash) = 71),
    CHECK (char_length(btrim(extractor_version)) BETWEEN 1 AND 256),
    CHECK (
        (status = 'pending' AND proposals IS NULL AND completed_at IS NULL)
        OR
        (status = 'completed' AND jsonb_typeof(proposals) = 'array' AND completed_at IS NOT NULL)
    )
);

ALTER TABLE memory_core.extraction_operations ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.extraction_operations FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.extraction_operations
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMIT;
