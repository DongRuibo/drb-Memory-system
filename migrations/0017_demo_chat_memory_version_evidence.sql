BEGIN;

ALTER TABLE demo_chat.turns
    ADD COLUMN used_memory_version_ids JSONB NOT NULL DEFAULT '[]'::jsonb;

COMMIT;
