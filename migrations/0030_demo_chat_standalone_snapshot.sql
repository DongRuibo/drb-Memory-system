BEGIN;

-- This compatibility migration is intentionally self-contained. It lets demo-web-api own the
-- Demo schema after repository extraction while remaining safe on databases created by the
-- historical monorepo migrations.
CREATE SCHEMA IF NOT EXISTS demo_chat;

CREATE TABLE IF NOT EXISTS demo_chat.sessions (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    user_id TEXT NOT NULL,
    title TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'guided' CHECK (mode IN ('guided', 'free')),
    guided_step TEXT NOT NULL DEFAULT 'remember',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, id)
);

CREATE INDEX IF NOT EXISTS idx_demo_chat_sessions_recent
    ON demo_chat.sessions (tenant_id, user_id, updated_at DESC)
    WHERE archived_at IS NULL;

CREATE TABLE IF NOT EXISTS demo_chat.messages (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    session_id UUID NOT NULL,
    turn_id UUID,
    client_message_id TEXT,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'completed'
        CHECK (status IN ('pending', 'streaming', 'completed', 'failed', 'cancelled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    FOREIGN KEY (tenant_id, session_id)
        REFERENCES demo_chat.sessions (tenant_id, id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_demo_chat_messages_timeline
    ON demo_chat.messages (tenant_id, session_id, created_at, id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_demo_chat_client_message_id
    ON demo_chat.messages (tenant_id, session_id, client_message_id)
    WHERE client_message_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS demo_chat.turns (
    tenant_id TEXT NOT NULL,
    id UUID NOT NULL,
    session_id UUID NOT NULL,
    user_message_id UUID NOT NULL,
    assistant_message_id UUID NOT NULL,
    task_id UUID NOT NULL,
    attempt_id UUID NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'accepted'
        CHECK (status IN ('accepted', 'processing', 'streaming', 'completed', 'failed', 'cancelled')),
    context_pack_id UUID,
    outcome_receipt_id UUID,
    used_memory_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    used_memory_version_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    memory_effects JSONB NOT NULL DEFAULT '[]'::jsonb,
    prompt_context_snapshot JSONB,
    fallback_runtime_context JSONB,
    model_id TEXT,
    model_request_id_hash TEXT,
    prompt_tokens BIGINT,
    completion_tokens BIGINT,
    total_tokens BIGINT,
    error_code TEXT,
    error_detail TEXT,
    cancel_requested_at TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, id),
    FOREIGN KEY (tenant_id, session_id)
        REFERENCES demo_chat.sessions (tenant_id, id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, user_message_id)
        REFERENCES demo_chat.messages (tenant_id, id),
    FOREIGN KEY (tenant_id, assistant_message_id)
        REFERENCES demo_chat.messages (tenant_id, id),
    UNIQUE (tenant_id, idempotency_key),
    UNIQUE (tenant_id, task_id, attempt_id),
    CONSTRAINT demo_chat_prompt_context_snapshot_shape CHECK (
        prompt_context_snapshot IS NULL
        OR (
            jsonb_typeof(prompt_context_snapshot) = 'object'
            AND prompt_context_snapshot ?& ARRAY['runtime_context', 'memory_context']
            AND jsonb_typeof(prompt_context_snapshot->'runtime_context') = 'object'
            AND jsonb_typeof(prompt_context_snapshot->'memory_context') = 'array'
        )
    ),
    CONSTRAINT demo_chat_fallback_runtime_context_shape CHECK (
        fallback_runtime_context IS NULL
        OR (
            jsonb_typeof(fallback_runtime_context) = 'object'
            AND fallback_runtime_context ?& ARRAY[
                'current_time_utc', 'timezone', 'local_date'
            ]
            AND jsonb_typeof(fallback_runtime_context->'current_time_utc') = 'string'
            AND jsonb_typeof(fallback_runtime_context->'timezone') = 'string'
            AND jsonb_typeof(fallback_runtime_context->'local_date') = 'string'
        )
    ),
    CONSTRAINT demo_chat_fallback_runtime_requires_no_pack CHECK (
        fallback_runtime_context IS NULL OR context_pack_id IS NULL
    )
);

ALTER TABLE demo_chat.turns
    ADD COLUMN IF NOT EXISTS prompt_context_snapshot JSONB;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'demo_chat.turns'::regclass
          AND conname = 'demo_chat_prompt_context_snapshot_shape'
    ) THEN
        ALTER TABLE demo_chat.turns
            ADD CONSTRAINT demo_chat_prompt_context_snapshot_shape CHECK (
                prompt_context_snapshot IS NULL
                OR (
                    jsonb_typeof(prompt_context_snapshot) = 'object'
                    AND prompt_context_snapshot ?& ARRAY['runtime_context', 'memory_context']
                    AND jsonb_typeof(prompt_context_snapshot->'runtime_context') = 'object'
                    AND jsonb_typeof(prompt_context_snapshot->'memory_context') = 'array'
                )
            );
    END IF;
END
$$;

DO $backfill$
BEGIN
    IF to_regprocedure('demo_chat.prompt_context_for_turn(text,uuid)') IS NOT NULL THEN
        EXECUTE $sql$
            UPDATE demo_chat.turns AS turn
            SET prompt_context_snapshot = COALESCE(
                demo_chat.prompt_context_for_turn(turn.tenant_id, turn.context_pack_id),
                CASE
                    WHEN turn.context_pack_id IS NULL
                         AND turn.fallback_runtime_context IS NOT NULL
                    THEN jsonb_build_object(
                        'runtime_context', turn.fallback_runtime_context,
                        'memory_context', '[]'::jsonb
                    )
                END
            )
            WHERE turn.prompt_context_snapshot IS NULL
        $sql$;
    ELSE
        UPDATE demo_chat.turns AS turn
        SET prompt_context_snapshot = jsonb_build_object(
            'runtime_context', turn.fallback_runtime_context,
            'memory_context', '[]'::jsonb
        )
        WHERE turn.prompt_context_snapshot IS NULL
          AND turn.context_pack_id IS NULL
          AND turn.fallback_runtime_context IS NOT NULL;
    END IF;
END
$backfill$;

CREATE UNIQUE INDEX IF NOT EXISTS uq_demo_chat_one_active_turn
    ON demo_chat.turns (tenant_id, session_id)
    WHERE status IN ('accepted', 'processing', 'streaming');
CREATE INDEX IF NOT EXISTS idx_demo_chat_turns_session
    ON demo_chat.turns (tenant_id, session_id, created_at DESC);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'demo_chat.messages'::regclass
          AND conname = 'fk_demo_chat_message_turn'
    ) THEN
        ALTER TABLE demo_chat.messages
            ADD CONSTRAINT fk_demo_chat_message_turn
            FOREIGN KEY (tenant_id, turn_id)
            REFERENCES demo_chat.turns (tenant_id, id);
    END IF;
END
$$;

CREATE TABLE IF NOT EXISTS demo_chat.turn_events (
    tenant_id TEXT NOT NULL,
    turn_id UUID NOT NULL,
    sequence_no BIGINT NOT NULL CHECK (sequence_no > 0),
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, turn_id, sequence_no),
    FOREIGN KEY (tenant_id, turn_id)
        REFERENCES demo_chat.turns (tenant_id, id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_demo_chat_turn_events_created
    ON demo_chat.turn_events (tenant_id, turn_id, created_at);

ALTER TABLE demo_chat.sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.sessions FORCE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.messages FORCE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turns ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turns FORCE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turn_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turn_events FORCE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_policies WHERE schemaname='demo_chat' AND policyname='demo_chat_sessions_tenant_isolation') THEN
        CREATE POLICY demo_chat_sessions_tenant_isolation ON demo_chat.sessions
            USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
            WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_policies WHERE schemaname='demo_chat' AND policyname='demo_chat_messages_tenant_isolation') THEN
        CREATE POLICY demo_chat_messages_tenant_isolation ON demo_chat.messages
            USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
            WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_policies WHERE schemaname='demo_chat' AND policyname='demo_chat_turns_tenant_isolation') THEN
        CREATE POLICY demo_chat_turns_tenant_isolation ON demo_chat.turns
            USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
            WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_policies WHERE schemaname='demo_chat' AND policyname='demo_chat_turn_events_tenant_isolation') THEN
        CREATE POLICY demo_chat_turn_events_tenant_isolation ON demo_chat.turn_events
            USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
            WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
    END IF;
END
$$;

COMMIT;
