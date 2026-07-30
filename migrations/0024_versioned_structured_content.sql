BEGIN;

ALTER TABLE memory_core.memory_candidates
    ADD COLUMN structured_content JSONB;

ALTER TABLE memory_core.memory_candidates
    ADD CONSTRAINT memory_candidates_structured_content_check CHECK (
        structured_content IS NULL
        OR (
            jsonb_typeof(structured_content) = 'object'
            AND structured_content ?& ARRAY['schema_name', 'schema_version', 'payload']
            AND structured_content - 'schema_name' - 'schema_version' - 'payload' = '{}'::jsonb
            AND structured_content->'schema_version' = '1'::jsonb
            AND jsonb_typeof(structured_content->'payload') = 'object'
            AND octet_length(structured_content::text) <= 4096
            AND structured_content->>'schema_name' = CASE memory_type
                WHEN 'preference' THEN 'lazytt.preference'
                WHEN 'profile_fact' THEN 'lazytt.profile_fact'
                WHEN 'semantic_fact' THEN 'lazytt.semantic_fact'
                WHEN 'episodic_summary' THEN 'lazytt.episodic_summary'
                ELSE NULL
            END
        )
    );

ALTER TABLE memory_core.memory_candidates
    ADD CONSTRAINT memory_candidates_erased_structure_check CHECK (
        content_erased_at IS NULL OR structured_content IS NULL
    );

ALTER TABLE memory_core.memory_versions
    ADD COLUMN structured_content JSONB;

ALTER TABLE memory_core.memory_versions
    ADD CONSTRAINT memory_versions_structured_content_check CHECK (
        structured_content IS NULL
        OR (
            jsonb_typeof(structured_content) = 'object'
            AND structured_content ?& ARRAY['schema_name', 'schema_version', 'payload']
            AND structured_content - 'schema_name' - 'schema_version' - 'payload' = '{}'::jsonb
            AND structured_content->'schema_version' = '1'::jsonb
            AND jsonb_typeof(structured_content->'payload') = 'object'
            AND octet_length(structured_content::text) <= 4096
            AND structured_content->>'schema_name' IN (
                'lazytt.preference',
                'lazytt.profile_fact',
                'lazytt.semantic_fact',
                'lazytt.episodic_summary'
            )
        )
    );

ALTER TABLE memory_core.memory_versions
    ADD CONSTRAINT memory_versions_erased_structure_check CHECK (
        content_erased_at IS NULL OR structured_content IS NULL
    );

COMMENT ON COLUMN memory_core.memory_candidates.structured_content IS
    'Optional, schema-v1 validated auxiliary representation proposed for a canonical memory.';
COMMENT ON COLUMN memory_core.memory_versions.structured_content IS
    'Optional immutable structured representation; canonical readable content remains authoritative.';

COMMIT;
