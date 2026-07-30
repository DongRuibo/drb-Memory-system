BEGIN;

CREATE TABLE memory_core.retrieval_runs (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    task_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    purpose TEXT NOT NULL,
    query_hash TEXT NOT NULL CHECK (char_length(query_hash) = 71),
    requested_limit INTEGER NOT NULL CHECK (requested_limit BETWEEN 1 AND 50),
    candidate_limit INTEGER NOT NULL CHECK (candidate_limit BETWEEN requested_limit AND 200),
    reference_time TIMESTAMPTZ NOT NULL,
    timezone TEXT NOT NULL CHECK (char_length(timezone) BETWEEN 1 AND 64),
    temporal_start TIMESTAMPTZ,
    temporal_end TIMESTAMPTZ,
    temporal_precision TEXT,
    temporal_source_expression TEXT,
    projection_generation TEXT NOT NULL,
    ranking_version TEXT NOT NULL,
    degraded BOOLEAN NOT NULL DEFAULT false,
    degradation_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    CHECK (
        (temporal_start IS NULL AND temporal_end IS NULL
            AND temporal_precision IS NULL AND temporal_source_expression IS NULL)
        OR
        (temporal_start IS NOT NULL AND temporal_end IS NOT NULL
            AND temporal_precision IS NOT NULL AND temporal_source_expression IS NOT NULL
            AND temporal_start < temporal_end)
    )
);

CREATE TABLE memory_core.retrieval_candidate_receipts (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    retrieval_run_id UUID NOT NULL,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('tenant', 'user', 'project', 'agent')),
    scope_id TEXT,
    channel TEXT NOT NULL CHECK (channel IN ('provider', 'bm25', 'lexical', 'temporal')),
    memory_record_id UUID NOT NULL,
    memory_version_id UUID NOT NULL,
    channel_rank INTEGER NOT NULL CHECK (channel_rank BETWEEN 1 AND 200),
    raw_score DOUBLE PRECISION NOT NULL CHECK (raw_score >= 0),
    channel_weight DOUBLE PRECISION NOT NULL CHECK (channel_weight > 0),
    rrf_contribution DOUBLE PRECISION NOT NULL CHECK (rrf_contribution > 0),
    fused_score DOUBLE PRECISION CHECK (fused_score BETWEEN 0 AND 1),
    preauthorized BOOLEAN NOT NULL,
    rerank_score DOUBLE PRECISION CHECK (rerank_score BETWEEN 0 AND 1),
    postauthorized BOOLEAN NOT NULL,
    final_rank INTEGER CHECK (final_rank BETWEEN 1 AND 50),
    included BOOLEAN NOT NULL,
    exclusion_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    UNIQUE (tenant_id, retrieval_run_id, channel, memory_version_id),
    FOREIGN KEY (tenant_id, retrieval_run_id)
        REFERENCES memory_core.retrieval_runs (tenant_id, id) ON DELETE CASCADE,
    CHECK ((scope_kind = 'tenant' AND scope_id IS NULL) OR (scope_kind <> 'tenant' AND scope_id IS NOT NULL)),
    CHECK (NOT postauthorized OR preauthorized),
    CHECK (NOT included OR (postauthorized AND final_rank IS NOT NULL)),
    CHECK ((included AND exclusion_reason IS NULL) OR (NOT included AND exclusion_reason IS NOT NULL))
);

CREATE INDEX idx_retrieval_runs_attempt
    ON memory_core.retrieval_runs (tenant_id, attempt_id, created_at DESC);

CREATE INDEX idx_retrieval_candidate_memory
    ON memory_core.retrieval_candidate_receipts
       (tenant_id, memory_record_id, created_at DESC, retrieval_run_id);

ALTER TABLE memory_core.retrieval_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.retrieval_runs FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.retrieval_runs
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

ALTER TABLE memory_core.retrieval_candidate_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE memory_core.retrieval_candidate_receipts FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON memory_core.retrieval_candidate_receipts
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMENT ON TABLE memory_core.retrieval_runs IS
    'Content-minimized audit receipts for retrieval executions; raw query text is never stored.';

COMMENT ON TABLE memory_core.retrieval_candidate_receipts IS
    'Per-channel ranking, authorization and final inclusion evidence for Recall Explain.';

COMMIT;
