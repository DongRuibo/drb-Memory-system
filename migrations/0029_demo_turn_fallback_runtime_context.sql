BEGIN;

ALTER TABLE demo_chat.turns
    ADD COLUMN fallback_runtime_context JSONB,
    ADD CONSTRAINT demo_chat_fallback_runtime_context_shape
        CHECK (
            fallback_runtime_context IS NULL
            OR (
                jsonb_typeof(fallback_runtime_context) = 'object'
                AND fallback_runtime_context ?& ARRAY[
                    'current_time_utc', 'timezone', 'local_date'
                ]
                AND jsonb_typeof(fallback_runtime_context->'current_time_utc') = 'string'
                AND jsonb_typeof(fallback_runtime_context->'timezone') = 'string'
                AND jsonb_typeof(fallback_runtime_context->'local_date') = 'string'
            )
        ),
    ADD CONSTRAINT demo_chat_fallback_runtime_requires_no_pack
        CHECK (fallback_runtime_context IS NULL OR context_pack_id IS NULL);

COMMIT;
