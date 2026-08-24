BEGIN;

ALTER TABLE memory_core.source_events
    ADD COLUMN redaction_policy_version TEXT NOT NULL DEFAULT 'legacy-unversioned',
    ADD COLUMN security_flags TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];

COMMENT ON COLUMN memory_core.source_events.redaction_policy_version IS
    'Version of the boundary policy that produced redacted_excerpt.';
COMMENT ON COLUMN memory_core.source_events.security_flags IS
    'Non-secret detection labels such as email or cn_phone; raw matched values are never stored.';

COMMIT;
