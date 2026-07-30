BEGIN;

ALTER TABLE memory_core.projection_generations
    ADD COLUMN index_provider TEXT NOT NULL DEFAULT 'mem0',
    ADD COLUMN embedding_provider TEXT NOT NULL DEFAULT 'local-poc',
    ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'poc-deterministic-embedding',
    ADD COLUMN embedding_revision TEXT NOT NULL DEFAULT 'poc-v1',
    ADD COLUMN embedding_dimensions INTEGER NOT NULL DEFAULT 1536,
    ADD COLUMN embedding_normalization TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN embedding_instruction_version TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN semantic_embedding BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN model_manifest_hash TEXT NOT NULL DEFAULT
        '6b52338a626d559e3540f50554bdaf2c0178d94bfbc9221fc6de682f9bad522f';

ALTER TABLE memory_core.projection_generations
    ADD CONSTRAINT projection_generation_index_provider_not_blank CHECK (
        length(btrim(index_provider)) BETWEEN 1 AND 128
    ),
    ADD CONSTRAINT projection_generation_embedding_provider_not_blank CHECK (
        length(btrim(embedding_provider)) BETWEEN 1 AND 128
    ),
    ADD CONSTRAINT projection_generation_embedding_model_not_blank CHECK (
        length(btrim(embedding_model)) BETWEEN 1 AND 256
    ),
    ADD CONSTRAINT projection_generation_embedding_revision_not_blank CHECK (
        length(btrim(embedding_revision)) BETWEEN 1 AND 256
    ),
    ADD CONSTRAINT projection_generation_embedding_dimensions_bounded CHECK (
        embedding_dimensions BETWEEN 1 AND 4096
    ),
    ADD CONSTRAINT projection_generation_embedding_normalization_known CHECK (
        embedding_normalization IN ('l2', 'none', 'provider_default')
    ),
    ADD CONSTRAINT projection_generation_embedding_instruction_not_blank CHECK (
        length(btrim(embedding_instruction_version)) BETWEEN 1 AND 256
    ),
    ADD CONSTRAINT projection_generation_manifest_hash_not_blank CHECK (
        length(btrim(model_manifest_hash)) BETWEEN 16 AND 128
    );

COMMIT;
