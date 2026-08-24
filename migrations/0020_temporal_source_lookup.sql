BEGIN;

CREATE INDEX idx_source_events_temporal_lookup
    ON memory_core.source_events (tenant_id, occurred_at, id);

COMMENT ON INDEX memory_core.idx_source_events_temporal_lookup IS
    'Supports source-time candidate filtering without copying canonical time into a projection.';

COMMIT;
