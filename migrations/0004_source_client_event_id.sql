BEGIN;

ALTER TABLE memory_core.source_events
    ADD COLUMN client_event_id TEXT;

UPDATE memory_core.source_events
SET client_event_id = id::text
WHERE client_event_id IS NULL;

ALTER TABLE memory_core.source_events
    ALTER COLUMN client_event_id SET NOT NULL;

CREATE UNIQUE INDEX uq_source_events_client_event
    ON memory_core.source_events (tenant_id, client_event_id);

COMMIT;
