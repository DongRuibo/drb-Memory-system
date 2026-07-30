BEGIN;

ALTER TABLE memory_core.erasure_requests
    ADD COLUMN idempotency_key TEXT,
    ADD COLUMN request_hash TEXT,
    ADD COLUMN requested_by_hash TEXT,
    ADD COLUMN record_count INTEGER NOT NULL DEFAULT 0 CHECK (record_count >= 0);

UPDATE memory_core.erasure_requests
SET
    idempotency_key='legacy-erasure-' || id::text,
    request_hash='legacy:' || id::text,
    requested_by_hash='legacy'
WHERE idempotency_key IS NULL;

UPDATE memory_core.erasure_requests
SET completed_at=COALESCE(completed_at,created_at)
WHERE status='complete';

ALTER TABLE memory_core.erasure_requests
    ALTER COLUMN idempotency_key SET NOT NULL,
    ALTER COLUMN request_hash SET NOT NULL,
    ALTER COLUMN requested_by_hash SET NOT NULL,
    ADD CONSTRAINT erasure_request_idempotency_length
        CHECK (char_length(idempotency_key) BETWEEN 16 AND 128),
    ADD CONSTRAINT erasure_request_completion_consistent CHECK (
        (status='complete' AND completed_at IS NOT NULL)
        OR (status<>'complete' AND completed_at IS NULL)
    ),
    ADD CONSTRAINT erasure_request_idempotency_unique
        UNIQUE (tenant_id, idempotency_key);

CREATE TABLE memory_core.erasure_request_records (
    tenant_id TEXT NOT NULL,
    erasure_request_id UUID NOT NULL,
    memory_record_id UUID NOT NULL,
    projection_operation_id UUID,
    status TEXT NOT NULL CHECK (status IN ('pending', 'complete', 'failed')),
    last_error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, erasure_request_id, memory_record_id),
    UNIQUE (tenant_id, projection_operation_id),
    FOREIGN KEY (tenant_id, erasure_request_id)
        REFERENCES memory_core.erasure_requests (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id),
    FOREIGN KEY (tenant_id, projection_operation_id)
        REFERENCES memory_core.projection_operations (tenant_id, id),
    CHECK (
        (status='complete' AND completed_at IS NOT NULL)
        OR (status<>'complete' AND completed_at IS NULL)
    )
);

ALTER TABLE memory_core.erasure_request_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.erasure_request_records FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.erasure_request_records
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

ALTER TABLE memory_core.memory_candidates
    ALTER COLUMN content DROP NOT NULL,
    ADD COLUMN content_erased_at TIMESTAMPTZ,
    ADD CONSTRAINT memory_candidate_content_erasure_consistent CHECK (
        (content IS NOT NULL AND content_erased_at IS NULL)
        OR (content IS NULL AND content_erased_at IS NOT NULL)
    );

CREATE OR REPLACE FUNCTION memory_core.guard_memory_head_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW.tenant_id <> OLD.tenant_id OR NEW.memory_record_id <> OLD.memory_record_id THEN
        RAISE EXCEPTION 'memory head identity is immutable'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.revision <> OLD.revision + 1 THEN
        RAISE EXCEPTION 'memory head revision must advance by exactly one'
            USING ERRCODE = '23514';
    END IF;

    IF OLD.state = 'erased' THEN
        RAISE EXCEPTION 'erased memory head is terminal'
            USING ERRCODE = '23514';
    END IF;

    IF OLD.state = 'forgotten' AND NEW.state <> 'erased' THEN
        RAISE EXCEPTION 'forgotten memory head may only transition to erased'
            USING ERRCODE = '23514';
    END IF;

    IF OLD.state = 'expired'
       AND NEW.state NOT IN ('expired', 'forgotten', 'erased') THEN
        RAISE EXCEPTION 'expired memory head cannot become active or contested'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.state IN ('active', 'contested', 'expired')
       AND NEW.current_version_id IS NULL THEN
        RAISE EXCEPTION 'readable or reviewable head requires a current version'
            USING ERRCODE = '23514';
    END IF;

    NEW.updated_at := now();
    RETURN NEW;
END
$function$;

COMMIT;
