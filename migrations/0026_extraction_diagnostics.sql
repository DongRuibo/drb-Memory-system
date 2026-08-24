BEGIN;

ALTER TABLE memory_core.extraction_operations
    ADD COLUMN diagnostics JSONB;

UPDATE memory_core.extraction_operations
SET diagnostics = jsonb_build_object(
    'model_candidate_count', NULL,
    'valid_model_candidate_count', jsonb_array_length(proposals),
    'filtered_reason_counts', '{}'::jsonb,
    'coverage_added_count', 0,
    'coverage_enriched_count', 0
)
WHERE status = 'completed';

ALTER TABLE memory_core.extraction_operations
    ADD CONSTRAINT extraction_operations_diagnostics_state
    CHECK (
        (status = 'pending' AND diagnostics IS NULL)
        OR
        (status = 'completed' AND jsonb_typeof(diagnostics) = 'object')
    );

COMMIT;
