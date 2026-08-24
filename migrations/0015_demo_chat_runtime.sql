BEGIN;

CREATE SCHEMA IF NOT EXISTS demo_chat;

CREATE TABLE demo_chat.sessions (
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

CREATE INDEX idx_demo_chat_sessions_recent
    ON demo_chat.sessions (tenant_id, user_id, updated_at DESC)
    WHERE archived_at IS NULL;

CREATE TABLE demo_chat.messages (
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
        REFERENCES demo_chat.sessions (tenant_id, id) ON DELETE CASCADE,
    UNIQUE NULLS NOT DISTINCT (tenant_id, session_id, client_message_id)
);

CREATE INDEX idx_demo_chat_messages_timeline
    ON demo_chat.messages (tenant_id, session_id, created_at, id);

CREATE TABLE demo_chat.turns (
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
    memory_effects JSONB NOT NULL DEFAULT '[]'::jsonb,
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
    UNIQUE (tenant_id, task_id, attempt_id)
);

ALTER TABLE demo_chat.messages
    ADD CONSTRAINT fk_demo_chat_message_turn
    FOREIGN KEY (tenant_id, turn_id)
    REFERENCES demo_chat.turns (tenant_id, id);

CREATE UNIQUE INDEX uq_demo_chat_one_active_turn
    ON demo_chat.turns (tenant_id, session_id)
    WHERE status IN ('accepted', 'processing', 'streaming');

CREATE INDEX idx_demo_chat_turns_session
    ON demo_chat.turns (tenant_id, session_id, created_at DESC);

CREATE TABLE demo_chat.turn_events (
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

CREATE INDEX idx_demo_chat_turn_events_created
    ON demo_chat.turn_events (tenant_id, turn_id, created_at);

ALTER TABLE demo_chat.sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.sessions FORCE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.messages FORCE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turns ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turns FORCE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turn_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE demo_chat.turn_events FORCE ROW LEVEL SECURITY;

CREATE POLICY demo_chat_sessions_tenant_isolation ON demo_chat.sessions
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
CREATE POLICY demo_chat_messages_tenant_isolation ON demo_chat.messages
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
CREATE POLICY demo_chat_turns_tenant_isolation ON demo_chat.turns
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));
CREATE POLICY demo_chat_turn_events_tenant_isolation ON demo_chat.turn_events
    USING (tenant_id = nullif(current_setting('app.tenant_id', true), ''))
    WITH CHECK (tenant_id = nullif(current_setting('app.tenant_id', true), ''));

COMMIT;
