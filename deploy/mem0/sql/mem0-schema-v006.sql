-- Mem0 control-plane schema for the pinned upstream commit:
-- b357a5a1b03c299ec8229c268e63cfac0f7c6566 (Alembic revision 006).
--
-- Run this file as the database owner against a NEW, EMPTY mem0_app database.
-- It is intentionally not idempotent: an existing object must stop deployment so
-- that an administrator can inspect schema drift instead of silently hiding it.

BEGIN;

CREATE TABLE public.alembic_version (
    version_num VARCHAR(32) NOT NULL,
    CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)
);

CREATE TABLE public.users (
    id UUID NOT NULL,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) NOT NULL,
    password_hash TEXT NOT NULL,
    role VARCHAR(20) DEFAULT 'admin' NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT now() NOT NULL,
    last_login_at TIMESTAMP WITH TIME ZONE,
    CONSTRAINT users_pkey PRIMARY KEY (id),
    CONSTRAINT users_email_key UNIQUE (email)
);

CREATE INDEX ix_users_email ON public.users (email);
CREATE UNIQUE INDEX ix_users_only_one_admin
    ON public.users (role)
    WHERE role = 'admin';

CREATE TABLE public.api_keys (
    id UUID NOT NULL,
    key_prefix VARCHAR(12) NOT NULL,
    key_hash TEXT NOT NULL,
    label VARCHAR(255) NOT NULL,
    created_by UUID NOT NULL,
    last_used_at TIMESTAMP WITH TIME ZONE,
    revoked_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT now() NOT NULL,
    CONSTRAINT api_keys_pkey PRIMARY KEY (id),
    CONSTRAINT api_keys_created_by_fkey
        FOREIGN KEY (created_by) REFERENCES public.users (id) ON DELETE CASCADE
);

CREATE TABLE public.request_logs (
    id UUID NOT NULL,
    method VARCHAR(16) NOT NULL,
    path VARCHAR(512) NOT NULL,
    status_code INTEGER NOT NULL,
    latency_ms DOUBLE PRECISION NOT NULL,
    auth_type VARCHAR(32) DEFAULT 'none' NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT now() NOT NULL,
    CONSTRAINT request_logs_pkey PRIMARY KEY (id)
);

CREATE INDEX ix_request_logs_created_at
    ON public.request_logs USING BRIN (created_at);

CREATE TABLE public.settings (
    key VARCHAR(255) NOT NULL,
    value TEXT NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT now() NOT NULL,
    CONSTRAINT settings_pkey PRIMARY KEY (key)
);

CREATE TABLE public.refresh_token_jtis (
    jti UUID NOT NULL,
    user_id UUID NOT NULL,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    used_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT now() NOT NULL,
    CONSTRAINT refresh_token_jtis_pkey PRIMARY KEY (jti),
    CONSTRAINT refresh_token_jtis_user_id_fkey
        FOREIGN KEY (user_id) REFERENCES public.users (id) ON DELETE CASCADE
);

CREATE INDEX ix_refresh_token_jtis_expires_at
    ON public.refresh_token_jtis (expires_at);

INSERT INTO public.alembic_version (version_num) VALUES ('006');

COMMIT;
