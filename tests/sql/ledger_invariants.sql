\set ON_ERROR_STOP on

BEGIN;

DO $role$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'memory_app_test') THEN
        CREATE ROLE memory_app_test NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
END
$role$;

GRANT USAGE ON SCHEMA memory_core, memory_search TO memory_app_test;
GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA memory_core TO memory_app_test;
GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA memory_search TO memory_app_test;
GRANT UPDATE (current_version_id, state, revision, updated_at)
    ON memory_core.memory_heads TO memory_app_test;

SET ROLE memory_app_test;
SET LOCAL app.tenant_id = 'tenant-a';

INSERT INTO memory_core.source_events (
    tenant_id, id, source_type, source_ref, content_hash,
    redacted_excerpt, classification, occurred_at, idempotency_key, client_event_id
) VALUES (
    'tenant-a', '00000000-0000-0000-0000-000000000001',
    'conversation_message', 'conversation:test-message-1', 'sha256:source',
    'User prefers explanations before code.', 'personal', now(), 'source-event-1', 'event-1'
);

INSERT INTO memory_core.memory_candidates (
    tenant_id, id, scope_kind, scope_id, memory_type, content,
    confidence, importance, inference_kind, extractor_name,
    extractor_version, policy_version, state
) VALUES (
    'tenant-a', '00000000-0000-0000-0000-000000000002',
    'user', 'user-a', 'preference', 'User prefers explanations before code.',
    1, 0.8, 'explicit', 'deterministic-poc', '1', 'policy-1', 'accepted'
);

INSERT INTO memory_core.candidate_sources (
    tenant_id, candidate_id, source_event_id
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000002',
    '00000000-0000-0000-0000-000000000001'
);

INSERT INTO memory_core.memory_decisions (
    tenant_id, id, candidate_id, outcome, reason_code,
    actor_type, actor_id, policy_version
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000003',
    '00000000-0000-0000-0000-000000000002',
    'accept', 'EXPLICIT_LOW_RISK_PREFERENCE',
    'policy', 'memory-policy', 'policy-1'
);

INSERT INTO memory_core.memory_records (
    tenant_id, id, scope_kind, scope_id, memory_type, semantic_key
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000004',
    'user', 'user-a', 'preference', 'explanation-order'
);

INSERT INTO memory_core.memory_versions (
    tenant_id, id, memory_record_id, version_no, content, content_hash,
    confidence, importance, inference_kind, classification,
    allowed_purposes, retention_policy_id, decision_id
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000005',
    '00000000-0000-0000-0000-000000000004',
    1, 'User prefers explanations before code.', 'sha256:version',
    1, 0.8, 'explicit', 'personal', ARRAY['answer_user'],
    'user-preference-v1',
    '00000000-0000-0000-0000-000000000003'
);

INSERT INTO memory_core.memory_version_sources (
    tenant_id, memory_version_id, source_event_id
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000005',
    '00000000-0000-0000-0000-000000000001'
);

INSERT INTO memory_core.memory_events (
    tenant_id, id, memory_record_id, sequence_no, event_type,
    memory_version_id, actor_type, actor_id, reason_code
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000006',
    '00000000-0000-0000-0000-000000000004',
    1, 'memory_accepted',
    '00000000-0000-0000-0000-000000000005',
    'policy', 'memory-policy', 'EXPLICIT_LOW_RISK_PREFERENCE'
);

INSERT INTO memory_core.memory_heads (
    tenant_id, memory_record_id, current_version_id, state, revision
) VALUES (
    'tenant-a',
    '00000000-0000-0000-0000-000000000004',
    '00000000-0000-0000-0000-000000000005',
    'active', 0
);

DO $assert_tenant_rls$
BEGIN
    BEGIN
        INSERT INTO memory_core.source_events (
            tenant_id, id, source_type, source_ref, content_hash,
            classification, occurred_at, idempotency_key, client_event_id
        ) VALUES (
            'tenant-b', '10000000-0000-0000-0000-000000000001',
            'conversation_message', 'forbidden', 'sha256:forbidden',
            'internal', now(), 'forbidden', 'forbidden-event'
        );
    EXCEPTION WHEN insufficient_privilege THEN
        RETURN;
    END;
    RAISE EXCEPTION 'expected tenant RLS to reject tenant-b insert';
END
$assert_tenant_rls$;

DO $assert_scope_check$
BEGIN
    BEGIN
        INSERT INTO memory_core.memory_records (
            tenant_id, id, scope_kind, scope_id, memory_type, semantic_key
        ) VALUES (
            'tenant-a', '20000000-0000-0000-0000-000000000001',
            'tenant', 'unexpected-id', 'preference', 'invalid-scope'
        );
    EXCEPTION WHEN check_violation THEN
        RETURN;
    END;
    RAISE EXCEPTION 'expected typed scope check to reject tenant scope_id';
END
$assert_scope_check$;

UPDATE memory_core.memory_heads
SET state = 'contested', revision = 1
WHERE tenant_id = 'tenant-a'
  AND memory_record_id = '00000000-0000-0000-0000-000000000004'
  AND revision = 0;

DO $assert_revision_guard$
BEGIN
    BEGIN
        UPDATE memory_core.memory_heads
        SET state = 'active', revision = 3
        WHERE tenant_id = 'tenant-a'
          AND memory_record_id = '00000000-0000-0000-0000-000000000004';
    EXCEPTION WHEN check_violation THEN
        RETURN;
    END;
    RAISE EXCEPTION 'expected revision jump to be rejected';
END
$assert_revision_guard$;

UPDATE memory_core.memory_heads
SET state = 'forgotten', revision = 2
WHERE tenant_id = 'tenant-a'
  AND memory_record_id = '00000000-0000-0000-0000-000000000004'
  AND revision = 1;

DO $assert_no_resurrection$
BEGIN
    BEGIN
        UPDATE memory_core.memory_heads
        SET state = 'active', revision = 3
        WHERE tenant_id = 'tenant-a'
          AND memory_record_id = '00000000-0000-0000-0000-000000000004';
    EXCEPTION WHEN check_violation THEN
        RETURN;
    END;
    RAISE EXCEPTION 'expected forgotten head reactivation to be rejected';
END
$assert_no_resurrection$;

RESET ROLE;
ROLLBACK;

\echo 'ledger invariants OK'
