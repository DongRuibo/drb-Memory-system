# 外部 PostgreSQL 18.4 迁移手册

本文用于把 Memory Service 的**权威数据库**从本机 Docker PostgreSQL 17 迁移到同一
局域网 Mac mini 上的 PostgreSQL 18.4。当前阶段不迁移 Mem0，不接入 Elasticsearch。

```text
Memory Service（用户 Mac）
  |-- PostgreSQL 18.4（Mac mini）  权威记忆、版本、账本
  `-- Mem0 + pgvector（用户 Mac） 可重建检索投影，暂时保留
```

权威 PostgreSQL 本身不需要安装 pgvector。Elasticsearch 8.11.3 替换检索投影属于下一
阶段，应在本次权威库迁移稳定后单独实施。

## 1. 迁移前必须准备

Mac mini 管理员需要提供：

- 稳定地址，例如 DHCP 保留后的 IP 或可解析的 `.local` 主机名；
- PostgreSQL 容器已发布 `5432` 端口；
- 只允许 Memory Service 所在 Mac 的局域网 IP 访问该端口；
- 一个专用数据库和非超级用户角色；
- 数据库备份目录、磁盘空间和恢复责任人。

第一阶段可使用一个非超级用户角色同时执行 Migration 和运行服务。正式生产前应拆成
Owner/Migrator 与 Runtime 两个角色，并把 `AUTO_MIGRATE` 从应用启动流程中移出。

不要使用 `postgres` 超级用户运行 Memory Service，不要把真实密码写进文档、命令历史
或 Git。若密码含有 `@`、`:`、`/`、`?`、`#`、`%` 等 URI 保留字符，写入连接 URL 前
必须进行百分号编码；更简单的做法是为应用生成 URL-safe 随机密码。

## 2. 在 Mac mini 创建空目标库

在服务器进入 PostgreSQL 容器，容器名由服务器管理员替换：

```bash
docker exec -it <POSTGRES_CONTAINER> psql -U postgres -d postgres
```

在 `psql` 中执行：

```sql
CREATE ROLE lazytt_memory_app
  LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
\password lazytt_memory_app

CREATE DATABASE lazytt_memory
  OWNER lazytt_memory_app
  ENCODING 'UTF8'
  TEMPLATE template0;
```

密码由 `\password` 交互输入，不会出现在 SQL 历史中。随后让服务器管理员确认
`listen_addresses`、`pg_hba.conf`、Docker 端口映射和主机防火墙只开放所需来源。

在运行 Memory Service 的 Mac 上测试连通性：

```bash
docker run --rm -it postgres:18.4-alpine \
  psql -W \
  --host=192.168.31.2 \
  --port=5432 \
  --username=lazytt_memory_app \
  --dbname=lazytt_memory \
  --command='select version(), current_database(), current_user;'
```

该命令只验证网络、账号和数据库，不创建 Memory 表。失败时先修服务器监听、访问控制或
防火墙，不要继续迁移。

## 3. 冻结写入并备份本地权威库

先确认当前使用的模型 profile，以下以 `cloud` 为例。只停止 Memory Service，保留本地
PostgreSQL 容器用于导出：

```bash
docker compose --profile cloud stop memory-service-cloud
```

记录迁移版本和核心表数量：

```bash
docker compose exec -T postgres psql -U memory -d memory -c \
  "SELECT version, success FROM _sqlx_migrations ORDER BY version DESC LIMIT 1;"

docker compose exec -T postgres psql -U memory -d memory -c \
  "SELECT 'records' AS item, count(*) FROM memory_core.memory_records
   UNION ALL SELECT 'versions', count(*) FROM memory_core.memory_versions
   UNION ALL SELECT 'heads', count(*) FROM memory_core.memory_heads
   UNION ALL SELECT 'events', count(*) FROM memory_core.memory_events;"
```

导出 PostgreSQL custom-format 备份。把宿主机目标路径替换成真实的安全路径：

```bash
docker compose exec -T postgres \
  pg_dump -U memory -d memory -Fc --no-owner --no-acl \
  > /absolute/safe/path/memory-authority-before-pg18.dump

shasum -a 256 /absolute/safe/path/memory-authority-before-pg18.dump
```

保留备份文件和校验值。此时不要执行 `down --volumes`，本地 PostgreSQL Volume 是回滚
保障。

## 4. 恢复到 PostgreSQL 18.4

目标数据库必须是专用于本次迁移的空库。使用 18.4 客户端恢复，过程中交互输入应用角色
密码：

```bash
docker run --rm -it \
  -v /absolute/safe/path:/backup:ro \
  postgres:18.4-alpine \
  pg_restore -W \
  --host=192.168.31.2 \
  --port=5432 \
  --username=lazytt_memory_app \
  --dbname=lazytt_memory \
  --no-owner --no-acl --exit-on-error \
  /backup/memory-authority-before-pg18.dump
```

如果恢复失败，不要在半恢复数据库上反复覆盖。由服务器管理员删除并重建这个**明确命名的
目标数据库**，再从原始备份重试。

恢复后由数据库管理员比较源库与目标库：

- `_sqlx_migrations` 的最高成功版本及全部成功记录；
- `memory_records`、`memory_versions`、`memory_heads`、`memory_events` 行数；
- 每个 Head 的 `current_version_id` 都能找到对应 Version；
- Tombstone、Erasure 和 Outbox 状态没有丢失。

Head/Version 完整性检查：

```sql
SELECT count(*) AS broken_heads
FROM memory_core.memory_heads h
LEFT JOIN memory_core.memory_versions v
  ON v.tenant_id = h.tenant_id
 AND v.memory_record_id = h.memory_record_id
 AND v.id = h.current_version_id
WHERE h.current_version_id IS NOT NULL
  AND v.id IS NULL;
```

结果必须是 `0`。不要仅凭服务能启动就判定迁移成功。

## 5. 切换 Memory Service

先停止当前 Compose 栈，但保留所有 Volume：

```bash
./scripts/backend-stack.sh cloud down
```

在本机 `.env` 中设置，以下全部是占位符：

```dotenv
MEMORY_POSTGRES_MODE=external
MEMORY_DATABASE_URL=postgresql://<APP_USER>:<URL_ENCODED_PASSWORD>@192.168.31.2:5432/lazytt_memory
```

确保 `.env` 权限正确，然后启动：

```bash
chmod 600 .env
./scripts/backend-stack.sh cloud up
curl --fail http://127.0.0.1:8080/health/ready
./scripts/backend-stack.sh cloud ps
```

启动输出应显示 `Authoritative PostgreSQL mode: external.`。此模式不会把本地 `postgres`
作为 Memory Service 依赖启动；本地 `mem0-postgres` 仍会运行，因为 Mem0 尚未迁移。

完成健康检查后，再用一条测试用户数据执行“提取 → operation 完成 → 搜索 → Context Pack”
验收。不要直接用真实用户数据做第一次验证。

## 6. 回滚

如果切换后尚未接收新写入，可以直接回滚连接：

```bash
./scripts/backend-stack.sh cloud down
```

把 `.env` 改回：

```dotenv
MEMORY_POSTGRES_MODE=local
MEMORY_DATABASE_URL=
```

再启动：

```bash
./scripts/backend-stack.sh cloud up
curl --fail http://127.0.0.1:8080/health/ready
```

如果外部库已经接收新写入，直接切回旧本地库会丢失这些新数据。此时必须先重新冻结写入并
制定反向迁移方案，不能把“切回连接”当成完整的数据回滚。

## 7. 本阶段验收标准与边界

本阶段完成的标准：

- 本地模式仍能使用 Compose 内置 PostgreSQL；
- 外部模式必须显式提供合法 PostgreSQL URL；
- 外部模式不依赖本地权威 PostgreSQL；
- PostgreSQL 17 备份可在 18.4 恢复，核心数量和引用完整性一致；
- 本地数据库 Volume 未删除，可用于受控回滚。

本阶段没有完成：

- Elasticsearch 8.11.3 Provider、索引模板、Alias 和 Projection Worker；
- Mem0/pgvector 到 Elasticsearch 的重建切换；
- TLS、Secret Manager、自动备份、监控、HA；
- 生产级 Migrator/Runtime 数据库角色拆分。
