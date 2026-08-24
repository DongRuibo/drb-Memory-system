BEGIN;

ALTER TABLE memory_core.context_packs
    ADD COLUMN consumed_at TIMESTAMPTZ,
    ADD COLUMN consume_idempotency_key TEXT,
    ADD COLUMN consume_request_hash TEXT,
    ADD CONSTRAINT context_pack_consumption_all_or_none CHECK (
        (consumed_at IS NULL
            AND consume_idempotency_key IS NULL
            AND consume_request_hash IS NULL)
        OR
        (consumed_at IS NOT NULL
            AND consume_idempotency_key IS NOT NULL
            AND consume_request_hash IS NOT NULL)
    ),
    ADD CONSTRAINT context_pack_consumed_after_creation CHECK (
        consumed_at IS NULL OR consumed_at >= created_at
    ),
    ADD CONSTRAINT context_pack_consume_key_length CHECK (
        consume_idempotency_key IS NULL
        OR char_length(consume_idempotency_key) BETWEEN 16 AND 128
    );

CREATE UNIQUE INDEX uq_context_packs_consume_idempotency
    ON memory_core.context_packs (tenant_id, consume_idempotency_key)
    WHERE consume_idempotency_key IS NOT NULL;

COMMIT;
