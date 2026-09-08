-- 管理员在 lazytt_memory_mem0 执行；只读验收，不创建或修改对象。
-- 与候选镜像使用同一份完整原生结构基线；升级上游版本时须重新迁移空库并审阅基线差异。
DO $$
DECLARE
  item record;
  role_name text;
  table_name text;
  privilege_name text;
  expected_tables text[] := ARRAY['alembic_version','api_keys','refresh_token_jtis','request_logs','settings','users'];
BEGIN
  IF current_database() <> 'lazytt_memory_mem0' THEN
    RAISE EXCEPTION 'Wrong database';
  END IF;
  IF (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()) <> 'lazytt_memory_mem0_owner' THEN
    RAISE EXCEPTION 'Wrong database owner';
  END IF;
  FOREACH role_name IN ARRAY ARRAY['lazytt_memory_mem0_owner','lazytt_memory_mem0_runtime'] LOOP
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname=role_name
      AND rolcanlogin AND rolconnlimit=8 AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls) THEN
      RAISE EXCEPTION 'Invalid role privileges';
    END IF;
    IF EXISTS (SELECT FROM pg_database WHERE NOT datistemplate AND datname<>current_database()
        AND has_database_privilege(role_name,oid,'CONNECT')) THEN
      RAISE EXCEPTION 'Role cross-database CONNECT remains';
    END IF;
    IF EXISTS (SELECT FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.member WHERE r.rolname=role_name) THEN
      RAISE EXCEPTION 'Unexpected role membership';
    END IF;
  END LOOP;
  IF has_schema_privilege('lazytt_memory_mem0_runtime','public','CREATE')
    OR has_database_privilege('lazytt_memory_mem0_runtime',current_database(),'TEMP') THEN
    RAISE EXCEPTION 'Runtime has schema creation privileges';
  END IF;
  IF EXISTS (SELECT FROM pg_database WHERE NOT datistemplate AND datname<>current_database()
      AND has_database_privilege('lazytt_memory_mem0_runtime',oid,'CONNECT')) THEN
    RAISE EXCEPTION 'Runtime cross-database CONNECT remains';
  END IF;
  IF (SELECT array_agg(tablename::text ORDER BY tablename) FROM pg_tables WHERE schemaname='public') IS DISTINCT FROM expected_tables THEN
    RAISE EXCEPTION 'Unexpected table set';
  END IF;
  IF (SELECT array_agg(version_num::text) FROM public.alembic_version) IS DISTINCT FROM ARRAY['006'] THEN
    RAISE EXCEPTION 'Wrong native migration head';
  END IF;
  -- Baseline: pristine Mem0 b357a5a1b03c299ec8229c268e63cfac0f7c6566, Alembic 001..006, PostgreSQL 18.4.
  -- This is read-only acceptance, never an alternative schema creation or stamp script.
  PERFORM set_config('search_path', 'public,pg_catalog', true);
  IF EXISTS (
    WITH expected(table_name,column_name,type_name,not_null,default_expr) AS (VALUES
      ('alembic_version', 'version_num', 'character varying(32)', 'true', ''),
      ('api_keys', 'id', 'uuid', 'true', ''),
      ('api_keys', 'key_prefix', 'character varying(12)', 'true', ''),
      ('api_keys', 'key_hash', 'text', 'true', ''),
      ('api_keys', 'label', 'character varying(255)', 'true', ''),
      ('api_keys', 'created_by', 'uuid', 'true', ''),
      ('api_keys', 'last_used_at', 'timestamp with time zone', 'false', ''),
      ('api_keys', 'revoked_at', 'timestamp with time zone', 'false', ''),
      ('api_keys', 'created_at', 'timestamp with time zone', 'true', 'now()'),
      ('refresh_token_jtis', 'jti', 'uuid', 'true', ''),
      ('refresh_token_jtis', 'user_id', 'uuid', 'true', ''),
      ('refresh_token_jtis', 'expires_at', 'timestamp with time zone', 'true', ''),
      ('refresh_token_jtis', 'used_at', 'timestamp with time zone', 'false', ''),
      ('refresh_token_jtis', 'created_at', 'timestamp with time zone', 'true', 'now()'),
      ('request_logs', 'id', 'uuid', 'true', ''),
      ('request_logs', 'method', 'character varying(16)', 'true', ''),
      ('request_logs', 'path', 'character varying(512)', 'true', ''),
      ('request_logs', 'status_code', 'integer', 'true', ''),
      ('request_logs', 'latency_ms', 'double precision', 'true', ''),
      ('request_logs', 'auth_type', 'character varying(32)', 'true', '''none''::character varying'),
      ('request_logs', 'created_at', 'timestamp with time zone', 'true', 'now()'),
      ('settings', 'key', 'character varying(255)', 'true', ''),
      ('settings', 'value', 'text', 'true', ''),
      ('settings', 'updated_at', 'timestamp with time zone', 'true', 'now()'),
      ('users', 'id', 'uuid', 'true', ''),
      ('users', 'name', 'character varying(255)', 'true', ''),
      ('users', 'email', 'character varying(255)', 'true', ''),
      ('users', 'password_hash', 'text', 'true', ''),
      ('users', 'role', 'character varying(20)', 'true', '''admin''::character varying'),
      ('users', 'created_at', 'timestamp with time zone', 'true', 'now()'),
      ('users', 'last_login_at', 'timestamp with time zone', 'false', '')
    ), actual AS (
      SELECT c.relname::text, a.attname::text, format_type(a.atttypid,a.atttypmod), a.attnotnull::text,
        coalesce(pg_get_expr(d.adbin,d.adrelid),'')
      FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
      LEFT JOIN pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum
      WHERE n.nspname='public' AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped
    )
    (SELECT * FROM expected EXCEPT ALL SELECT * FROM actual)
    UNION ALL
    (SELECT * FROM actual EXCEPT ALL SELECT * FROM expected)
  ) THEN
    RAISE EXCEPTION 'Native columns mismatch';
  END IF;
  IF EXISTS (
    WITH expected(index_name,definition,valid,ready) AS (VALUES
      ('alembic_version_pkc', 'CREATE UNIQUE INDEX alembic_version_pkc ON public.alembic_version USING btree (version_num)', 'true', 'true'),
      ('api_keys_pkey', 'CREATE UNIQUE INDEX api_keys_pkey ON public.api_keys USING btree (id)', 'true', 'true'),
      ('ix_refresh_token_jtis_expires_at', 'CREATE INDEX ix_refresh_token_jtis_expires_at ON public.refresh_token_jtis USING btree (expires_at)', 'true', 'true'),
      ('refresh_token_jtis_pkey', 'CREATE UNIQUE INDEX refresh_token_jtis_pkey ON public.refresh_token_jtis USING btree (jti)', 'true', 'true'),
      ('ix_request_logs_created_at', 'CREATE INDEX ix_request_logs_created_at ON public.request_logs USING brin (created_at)', 'true', 'true'),
      ('request_logs_pkey', 'CREATE UNIQUE INDEX request_logs_pkey ON public.request_logs USING btree (id)', 'true', 'true'),
      ('settings_pkey', 'CREATE UNIQUE INDEX settings_pkey ON public.settings USING btree (key)', 'true', 'true'),
      ('ix_users_email', 'CREATE INDEX ix_users_email ON public.users USING btree (email)', 'true', 'true'),
      ('ix_users_only_one_admin', 'CREATE UNIQUE INDEX ix_users_only_one_admin ON public.users USING btree (role) WHERE ((role)::text = ''admin''::text)', 'true', 'true'),
      ('users_email_key', 'CREATE UNIQUE INDEX users_email_key ON public.users USING btree (email)', 'true', 'true'),
      ('users_pkey', 'CREATE UNIQUE INDEX users_pkey ON public.users USING btree (id)', 'true', 'true')
    ), actual AS (
      SELECT i.relname::text, pg_get_indexdef(i.oid), x.indisvalid::text, x.indisready::text
      FROM pg_index x JOIN pg_class i ON i.oid=x.indexrelid JOIN pg_namespace n ON n.oid=i.relnamespace
      WHERE n.nspname='public'
    )
    (SELECT * FROM expected EXCEPT ALL SELECT * FROM actual)
    UNION ALL
    (SELECT * FROM actual EXCEPT ALL SELECT * FROM expected)
  ) THEN
    RAISE EXCEPTION 'Native indexes mismatch';
  END IF;
  IF EXISTS (
    WITH expected(table_name,constraint_name,definition,validated,is_deferrable,is_deferred) AS (VALUES
      ('alembic_version', 'alembic_version_pkc', 'PRIMARY KEY (version_num)', 'true', 'false', 'false'),
      ('api_keys', 'api_keys_created_by_fkey', 'FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE CASCADE', 'true', 'false', 'false'),
      ('api_keys', 'api_keys_pkey', 'PRIMARY KEY (id)', 'true', 'false', 'false'),
      ('refresh_token_jtis', 'refresh_token_jtis_pkey', 'PRIMARY KEY (jti)', 'true', 'false', 'false'),
      ('refresh_token_jtis', 'refresh_token_jtis_user_id_fkey', 'FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE', 'true', 'false', 'false'),
      ('request_logs', 'request_logs_pkey', 'PRIMARY KEY (id)', 'true', 'false', 'false'),
      ('settings', 'settings_pkey', 'PRIMARY KEY (key)', 'true', 'false', 'false'),
      ('users', 'users_email_key', 'UNIQUE (email)', 'true', 'false', 'false'),
      ('users', 'users_pkey', 'PRIMARY KEY (id)', 'true', 'false', 'false')
    ), actual AS (
      SELECT c.relname::text, x.conname::text, pg_get_constraintdef(x.oid), x.convalidated::text,
        x.condeferrable::text, x.condeferred::text
      FROM pg_constraint x JOIN pg_class c ON c.oid=x.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace
      WHERE n.nspname='public' AND x.contype<>'n'
    )
    (SELECT * FROM expected EXCEPT ALL SELECT * FROM actual)
    UNION ALL
    (SELECT * FROM actual EXCEPT ALL SELECT * FROM expected)
  ) THEN
    RAISE EXCEPTION 'Native constraints mismatch';
  END IF;
  IF EXISTS (SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
      WHERE n.nspname='public' AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped
        AND (a.attidentity<>'' OR a.attgenerated<>'')) THEN
    RAISE EXCEPTION 'Unexpected identity/generated column';
  END IF;
  FOREACH table_name IN ARRAY expected_tables LOOP
    IF (SELECT pg_get_userbyid(relowner) FROM pg_class WHERE oid=to_regclass('public.'||table_name)) <> 'lazytt_memory_mem0_owner' THEN
      RAISE EXCEPTION 'Wrong table owner';
    END IF;
    FOREACH privilege_name IN ARRAY ARRAY['SELECT','INSERT','UPDATE','DELETE'] LOOP
      IF has_table_privilege('lazytt_memory_mem0_runtime','public.'||table_name,privilege_name)
         IS DISTINCT FROM (table_name<>'alembic_version' OR privilege_name='SELECT') THEN
        RAISE EXCEPTION 'Wrong runtime table privileges';
      END IF;
    END LOOP;
  END LOOP;
  IF EXISTS (SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
      WHERE n.nspname='public' AND c.relkind IN ('r','i') AND obj_description(c.oid,'pg_class') IS NULL)
    OR EXISTS (SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
      WHERE n.nspname='public' AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped AND col_description(c.oid,a.attnum) IS NULL)
    OR EXISTS (SELECT FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace
      WHERE n.nspname='public' AND obj_description(c.oid,'pg_constraint') IS NULL) THEN
    RAISE EXCEPTION 'Missing schema comments';
  END IF;
END $$;
SELECT 'MEMORY_SCHEMA_SQL_OK head=006' AS result;
