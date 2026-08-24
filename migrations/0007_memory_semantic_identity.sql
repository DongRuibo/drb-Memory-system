BEGIN;

ALTER TABLE memory_core.memory_records
    ADD COLUMN semantic_key TEXT,
    ADD COLUMN identity_retired_at TIMESTAMPTZ;

WITH ranked AS (
    SELECT
        r.tenant_id,
        r.id,
        COALESCE(NULLIF(v.normalized_key, ''), r.id::text) AS base_key,
        row_number() OVER (
            PARTITION BY
                r.tenant_id,
                r.scope_kind,
                COALESCE(r.scope_id, ''),
                r.memory_type,
                COALESCE(NULLIF(v.normalized_key, ''), r.id::text)
            ORDER BY
                CASE WHEN h.state = 'active' THEN 0 ELSE 1 END,
                r.created_at,
                r.id
        ) AS duplicate_rank,
        h.state
    FROM memory_core.memory_records r
    JOIN memory_core.memory_heads h
      ON h.tenant_id=r.tenant_id AND h.memory_record_id=r.id
    LEFT JOIN memory_core.memory_versions v
      ON v.tenant_id=h.tenant_id AND v.id=h.current_version_id
)
UPDATE memory_core.memory_records r
SET
    semantic_key = CASE
        WHEN ranked.duplicate_rank = 1 THEN ranked.base_key
        ELSE ranked.base_key || '#legacy-' || r.id::text
    END,
    identity_retired_at = CASE
        WHEN ranked.state IN ('forgotten', 'erased') THEN now()
        ELSE NULL
    END
FROM ranked
WHERE r.tenant_id=ranked.tenant_id AND r.id=ranked.id;

ALTER TABLE memory_core.memory_records
    ALTER COLUMN semantic_key SET NOT NULL,
    ADD CONSTRAINT memory_record_semantic_key_not_blank CHECK (
        length(btrim(semantic_key)) BETWEEN 1 AND 512
    ),
    ADD CONSTRAINT memory_record_retirement_after_creation CHECK (
        identity_retired_at IS NULL OR identity_retired_at >= created_at
    );

CREATE UNIQUE INDEX uq_memory_records_live_semantic_identity
    ON memory_core.memory_records (
        tenant_id,
        scope_kind,
        COALESCE(scope_id, ''),
        memory_type,
        semantic_key
    )
    WHERE identity_retired_at IS NULL;

ALTER TABLE memory_core.memory_candidates
    ADD COLUMN target_memory_record_id UUID,
    ADD CONSTRAINT memory_candidate_target_record_fk
        FOREIGN KEY (tenant_id, target_memory_record_id)
        REFERENCES memory_core.memory_records (tenant_id, id);

UPDATE memory_core.memory_candidates c
SET target_memory_record_id=v.memory_record_id
FROM memory_core.memory_decisions d
JOIN memory_core.memory_versions v
  ON v.tenant_id=d.tenant_id AND v.decision_id=d.id
WHERE d.tenant_id=c.tenant_id
  AND d.candidate_id=c.id
  AND c.target_memory_record_id IS NULL;

CREATE TABLE memory_core.memory_ingest_results (
    tenant_id TEXT NOT NULL,
    source_event_id UUID NOT NULL,
    candidate_id UUID NOT NULL,
    memory_record_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    result_kind TEXT NOT NULL CHECK (result_kind IN ('created', 'source_attached')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, source_event_id),
    FOREIGN KEY (tenant_id, source_event_id)
        REFERENCES memory_core.source_events (tenant_id, id),
    FOREIGN KEY (tenant_id, candidate_id)
        REFERENCES memory_core.memory_candidates (tenant_id, id),
    FOREIGN KEY (tenant_id, memory_record_id, memory_version_id)
        REFERENCES memory_core.memory_versions (tenant_id, memory_record_id, id),
    FOREIGN KEY (tenant_id, operation_id)
        REFERENCES memory_core.projection_operations (tenant_id, id)
);

ALTER TABLE memory_core.memory_ingest_results ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.memory_ingest_results FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.memory_ingest_results
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMIT;
