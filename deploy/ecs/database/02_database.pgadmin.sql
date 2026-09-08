-- 管理员连接入口库；关闭显式事务。先单独查询同名库是否存在：
SELECT datname, pg_get_userbyid(datdba) AS owner
FROM pg_database WHERE datname='lazytt_memory_mem0';

-- A：只有确认目标是新库且上述无结果时，单独执行这一句。
CREATE DATABASE lazytt_memory_mem0 OWNER lazytt_memory_mem0_owner;

-- B：确认库归属后，在入口库执行以下段落；不更改其他数据库的 ACL。
DO $$
BEGIN
  IF (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname='lazytt_memory_mem0')
      IS DISTINCT FROM 'lazytt_memory_mem0_owner' THEN
    RAISE EXCEPTION 'Wrong database owner';
  END IF;
END $$;
REVOKE ALL ON DATABASE lazytt_memory_mem0 FROM PUBLIC;
GRANT CONNECT ON DATABASE lazytt_memory_mem0 TO lazytt_memory_mem0_runtime;
COMMENT ON DATABASE lazytt_memory_mem0 IS 'Memory dev 的 Mem0 认证与配置数据库；向量在 Elasticsearch，历史在独立持久卷。';

-- C：新开查询窗口连接 lazytt_memory_mem0 后执行，不能在入口库执行。
DO $$
BEGIN
  IF current_database() <> 'lazytt_memory_mem0' THEN
    RAISE EXCEPTION 'Wrong database';
  END IF;
END $$;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO lazytt_memory_mem0_runtime;
-- 运行账号对其他库仍可能通过 PUBLIC 获得 CONNECT。
-- 必须由共享 Store 管理员先备份并审阅共享 ACL 后处理；本脚本不自动修改。
