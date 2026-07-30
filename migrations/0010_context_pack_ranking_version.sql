BEGIN;

ALTER TABLE memory_core.context_packs
    ADD COLUMN ranking_version TEXT NOT NULL DEFAULT 'legacy-unversioned';

ALTER TABLE memory_core.context_packs
    ADD CONSTRAINT context_pack_ranking_version_length
        CHECK (char_length(ranking_version) BETWEEN 1 AND 128);

COMMIT;
