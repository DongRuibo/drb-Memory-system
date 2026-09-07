-- 管理员在 lazytt_memory_mem0 执行；只读验收，不创建或修改对象。
-- 原生迁移入口还会核对候选镜像内由上游 models 生成的字段/类型/主外键清单。
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
      AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls) THEN
      RAISE EXCEPTION 'Invalid role privileges';
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
