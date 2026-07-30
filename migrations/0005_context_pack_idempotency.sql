BEGIN;

CREATE UNIQUE INDEX uq_context_packs_compile_identity
    ON memory_core.context_packs (
        tenant_id,
        attempt_id,
        purpose,
        query_hash,
        compiler_version,
        authority_snapshot_id
    );

COMMIT;
