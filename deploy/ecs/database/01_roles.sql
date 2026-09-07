-- 执行位置：管理员连接共享 PostgreSQL 的入口库。只创建本系统角色。
-- 遇到没有本包归属注释的同名角色即停止，不接管、不改密码。
DO $$
DECLARE name text;
BEGIN
  FOREACH name IN ARRAY ARRAY['lazytt_memory_mem0_owner', 'lazytt_memory_mem0_runtime'] LOOP
    IF EXISTS (SELECT FROM pg_roles WHERE rolname=name) THEN
      IF (SELECT shobj_description(oid, 'pg_authid') FROM pg_roles WHERE rolname=name)
          IS DISTINCT FROM 'LazyTT Memory dev 专用角色；由 deploy/ecs 管理。' THEN
        RAISE EXCEPTION 'Existing role ownership is not established';
      END IF;
    ELSE
      EXECUTE format('CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 8', name);
      EXECUTE format('COMMENT ON ROLE %I IS %L', name, 'LazyTT Memory dev 专用角色；由 deploy/ecs 管理。');
    END IF;
  END LOOP;
END $$;
-- 在 pgAdmin 角色属性中分别设置密码，或 psql 使用 \password 交互输入。
-- owner 只用于显式迁移；runtime 只有本库数据权限，没有建表和跨库权限。
