BEGIN;

-- Keep every conflicting source while one canonical record is contested.
DROP INDEX memory_core.uq_memory_candidates_open_review_target;
CREATE INDEX idx_memory_candidates_open_review_target
    ON memory_core.memory_candidates (tenant_id, target_memory_record_id)
    WHERE state='needs_review' AND target_memory_record_id IS NOT NULL;

COMMIT;
