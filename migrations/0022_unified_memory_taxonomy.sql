BEGIN;

LOCK TABLE memory_core.memory_candidates, memory_core.memory_records
    IN SHARE ROW EXCLUSIVE MODE;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM memory_core.memory_records
        WHERE memory_type = 'procedural_candidate'
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = 'check_violation',
            MESSAGE = 'taxonomy migration blocked: procedural_candidate exists as a formal Memory Record';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM memory_core.memory_candidates
        WHERE memory_type = 'procedural_candidate' AND state = 'accepted'
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = 'check_violation',
            MESSAGE = 'taxonomy migration blocked: an accepted procedural_candidate requires manual disposition';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM memory_core.memory_records legacy
        JOIN memory_core.memory_records canonical
          ON canonical.tenant_id = legacy.tenant_id
         AND canonical.scope_kind = legacy.scope_kind
         AND canonical.scope_id IS NOT DISTINCT FROM legacy.scope_id
         AND canonical.semantic_key = legacy.semantic_key
         AND canonical.memory_type = 'semantic_fact'
         AND canonical.identity_retired_at IS NULL
        WHERE legacy.memory_type = 'project_fact'
          AND legacy.identity_retired_at IS NULL
    ) THEN
        RAISE EXCEPTION USING
            ERRCODE = 'unique_violation',
            MESSAGE = 'taxonomy migration blocked: project_fact to semantic_fact creates a live semantic identity collision';
    END IF;
END $$;

CREATE TABLE memory_core.memory_type_reclassifications (
    tenant_id TEXT NOT NULL,
    object_kind TEXT NOT NULL CHECK (object_kind IN ('candidate', 'record')),
    object_id UUID NOT NULL,
    from_memory_type TEXT NOT NULL,
    to_memory_type TEXT,
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'TYPE_SCOPE_DIMENSION_REMOVED',
        'TYPE_MOVED_TO_PROCEDURE_PIPELINE'
    )),
    taxonomy_version TEXT NOT NULL CHECK (taxonomy_version = 'memory-taxonomy-v2'),
    reclassified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, object_kind, object_id, taxonomy_version),
    CHECK (
        (reason_code = 'TYPE_SCOPE_DIMENSION_REMOVED'
            AND from_memory_type = 'project_fact' AND to_memory_type = 'semantic_fact')
        OR
        (reason_code = 'TYPE_MOVED_TO_PROCEDURE_PIPELINE'
            AND from_memory_type = 'procedural_candidate' AND to_memory_type IS NULL)
    )
);

INSERT INTO memory_core.memory_type_reclassifications (
    tenant_id, object_kind, object_id, from_memory_type, to_memory_type,
    reason_code, taxonomy_version
)
SELECT tenant_id, 'candidate', id, 'project_fact', 'semantic_fact',
       'TYPE_SCOPE_DIMENSION_REMOVED', 'memory-taxonomy-v2'
FROM memory_core.memory_candidates
WHERE memory_type = 'project_fact';

INSERT INTO memory_core.memory_type_reclassifications (
    tenant_id, object_kind, object_id, from_memory_type, to_memory_type,
    reason_code, taxonomy_version
)
SELECT tenant_id, 'record', id, 'project_fact', 'semantic_fact',
       'TYPE_SCOPE_DIMENSION_REMOVED', 'memory-taxonomy-v2'
FROM memory_core.memory_records
WHERE memory_type = 'project_fact';

INSERT INTO memory_core.memory_type_reclassifications (
    tenant_id, object_kind, object_id, from_memory_type, to_memory_type,
    reason_code, taxonomy_version
)
SELECT tenant_id, 'candidate', id, 'procedural_candidate', NULL,
       'TYPE_MOVED_TO_PROCEDURE_PIPELINE', 'memory-taxonomy-v2'
FROM memory_core.memory_candidates
WHERE memory_type = 'procedural_candidate';

ALTER TABLE memory_core.memory_candidates
    DROP CONSTRAINT memory_candidates_memory_type_check;
ALTER TABLE memory_core.memory_records
    DROP CONSTRAINT memory_records_memory_type_check;

UPDATE memory_core.memory_candidates
SET memory_type = 'semantic_fact'
WHERE memory_type = 'project_fact';

UPDATE memory_core.memory_records
SET memory_type = 'semantic_fact'
WHERE memory_type = 'project_fact';

UPDATE memory_core.memory_candidates
SET state = 'needs_review', revision = revision + 1
WHERE memory_type = 'procedural_candidate' AND state = 'pending';

ALTER TABLE memory_core.memory_candidates
    ADD CONSTRAINT memory_candidates_memory_type_check CHECK (memory_type IN (
        'preference', 'profile_fact', 'semantic_fact',
        'episodic_summary', 'procedural_candidate'
    )),
    ADD CONSTRAINT memory_candidates_procedural_not_formal_check CHECK (
        memory_type <> 'procedural_candidate' OR state IN ('needs_review', 'rejected')
    );

ALTER TABLE memory_core.memory_records
    ADD CONSTRAINT memory_records_memory_type_check CHECK (memory_type IN (
        'preference', 'profile_fact', 'semantic_fact', 'episodic_summary'
    ));

CREATE INDEX idx_memory_type_reclassifications_object
    ON memory_core.memory_type_reclassifications
       (tenant_id, object_kind, object_id, reclassified_at DESC);

ALTER TABLE memory_core.memory_type_reclassifications ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.memory_type_reclassifications FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.memory_type_reclassifications
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMENT ON TABLE memory_core.memory_type_reclassifications IS
    'Content-free audit evidence for memory-taxonomy-v2 reclassification and procedural routing.';

COMMIT;
