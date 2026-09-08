BEGIN;

ALTER TABLE memory_core.projection_operations
    DROP CONSTRAINT projection_operations_status_check;

ALTER TABLE memory_core.projection_operations
    ADD CONSTRAINT projection_operations_status_check
    CHECK (status IN (
        'pending', 'leased', 'unknown', 'ready', 'failed',
        'stale', 'deleted', 'dead_letter'
    ));

ALTER TABLE memory_core.projection_operations
    ADD COLUMN lease_version BIGINT NOT NULL DEFAULT 0 CHECK (lease_version >= 0),
    ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

COMMIT;
