# LazyTT Memory System

## ECS dev 发布

面向 `dev` 分支的 ECS 标准部署使用 [deploy/ecs/README.md](deploy/ecs/README.md)，
通过 PR、GitHub 镜像交付和显式管理员迁移执行；不部署 Agent。
下文本地及通用服务器示例不代替该 ECS 流程。

当前推荐运行模式是 `mem0_primary`：Agent 只调用 Memory Service，Mem0 负责
长期记忆的提取、更新和语义检索，Elasticsearch 保存向量，Mem0 PostgreSQL
保存 Mem0 的认证与配置状态。

```text
Agent -> Memory Service :18081 -> Mem0 -> Elasticsearch
                                      \-> Mem0 PostgreSQL
```

Agent 不应直接调用 Mem0 或 Elasticsearch。

## 首次本地部署

要求：

- Docker Desktop 与 Docker Compose v2；
- 已获得该 GitHub INTERNAL 仓库的组织访问权限，并能访问 Docker Hub、PyPI 和配置的模型 Provider；
- 一组 OpenAI-compatible 的聊天模型凭据和 embedding 凭据。

在当前功能尚未合并到 `main` 前，需要明确克隆此分支：

```bash
git clone --branch feature/ruibo-dong_lazyx/2/memory-system-implementation \
  --single-branch https://github.com/lazyaix/lazytt-memory-system.git
cd lazytt-memory-system
cp .env.example .env
./scripts/bootstrap-backend-env.sh
```

编辑 `.env`，至少填写以下六项；不要提交 `.env`：

```dotenv
MEMORY_EXTRACT_API_KEY=<REDACTED>
MEMORY_EXTRACT_BASE_URL=https://your-provider.example/v1
MEMORY_EXTRACT_MODEL=your-chat-model
MEMORY_EMBEDDING_API_KEY=<REDACTED>
MEMORY_EMBEDDING_BASE_URL=https://your-provider.example/v1
MEMORY_EMBEDDING_MODEL=your-embedding-model
```

`MEMORY_EMBEDDING_DIMENSIONS` 必须与 embedding 模型实际输出维度一致；改变模型、
维度或索引名称后，不要复用已有 Elasticsearch 索引。

构建固定版本的 Mem0 镜像，然后启动本地 B 模式：

```bash
./scripts/build-mem0-elasticsearch.sh

docker compose \
  -f compose.yaml \
  -f compose.elasticsearch.yaml \
  -f compose.elasticsearch-runtime.yaml \
  up --build -d mem0-config-elasticsearch memory-service-elasticsearch
```

首次构建需要下载 Mem0、Rust、Python 与容器依赖，耗时取决于网络环境。

## 服务器复用已有 PostgreSQL 和 Elasticsearch

`compose.server-runtime.yaml` 不部署 PostgreSQL 或 Elasticsearch，也不执行 Alembic。
Mem0 使用 Elasticsearch 保存向量。服务器管理员应创建独立认证配置数据库和低权限运行账号，
审核并手动执行仓库提供的固定 SQL；运行账号不能建表或修改表结构。

先以 PostgreSQL 管理员身份连接入口数据库。数据库必须由管理员（或专用 schema
owner）持有，不能把所有者设成 `mem0_runtime`。密码使用 `psql` 的 `\password`
交互设置，不能写进命令历史或提交到 Git：

```sql
CREATE ROLE mem0_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;
\password mem0_runtime
CREATE DATABASE mem0_app;
```

在仓库根目录由管理员把锁定版本的 schema 应用到空数据库：

```bash
docker exec -i <POSTGRES_CONTAINER> \
  psql -U <POSTGRES_ADMIN_USER> -d mem0_app -v ON_ERROR_STOP=1 \
  < deploy/mem0/sql/mem0-schema-v006.sql
```

随后仍由管理员授予运行账号最小权限。这里会同时撤销 `public` 的建表和临时表
权限，因此 `mem0_app` 应当是 Mem0 专用数据库：

```sql
REVOKE CONNECT, TEMPORARY ON DATABASE mem0_app FROM PUBLIC;
GRANT CONNECT ON DATABASE mem0_app TO mem0_runtime;
\connect mem0_app
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO mem0_runtime;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO mem0_runtime;
```

检查固定 schema 版本，结果必须是 `006`：

```sql
SELECT version_num FROM public.alembic_version;
```

`mem0-schema-v006.sql` 只适用于新的空数据库，已有数据库不能重复执行。未来升级
Mem0 时，必须先由项目提供并审核新的管理员迁移 SQL，再升级服务镜像。

把已有 PostgreSQL、Elasticsearch 容器与 Memory System 接入同一个 Docker 网络。
已连接的容器不需要重复执行 `docker network connect`：

```bash
docker network create lazytt-memory-data
docker network connect lazytt-memory-data <POSTGRES_CONTAINER>
docker network connect lazytt-memory-data <ELASTICSEARCH_CONTAINER>
```

服务器 `.env` 至少需要配置以下连接信息。容器之间使用容器名或网络别名和容器
端口，不能使用 `127.0.0.1` 或宿主机映射端口：

```dotenv
MEM0_POSTGRES_HOST=<POSTGRES_CONTAINER_OR_ALIAS>
MEM0_POSTGRES_PORT=5432
MEM0_POSTGRES_DB=mem0_app
MEM0_APP_DB_NAME=mem0_app
MEM0_POSTGRES_RUNTIME_USER=mem0_runtime
MEM0_POSTGRES_RUNTIME_PASSWORD=<REDACTED>

LAZYTT_MEMORY_DATA_NETWORK=lazytt-memory-data
LAZYTT_MEMORY_ELASTICSEARCH_ENDPOINT=http://<ELASTICSEARCH_CONTAINER_OR_ALIAS>
ELASTICSEARCH_PORT=9200
ELASTICSEARCH_USERNAME=<ELASTICSEARCH_USER>
STORE_ELASTICSEARCH_PASSWORD=<REDACTED>
```

先构建镜像、检查 Compose，再启动服务：

```bash
./scripts/build-mem0-elasticsearch.sh

docker compose --env-file .env -f compose.server-runtime.yaml config --quiet
docker compose --env-file .env -f compose.server-runtime.yaml up --build -d
```

现有 PostgreSQL 必须允许 `mem0_runtime` 从该 Docker 网络连接。服务器部署前
应先备份 PostgreSQL 和 Elasticsearch，并验证管理员 SQL、最小权限、健康检查
和一条合成记忆 E2E。Mem0 运行期间若 schema 不匹配会启动或请求失败，但不会
自行创建、修改或删除 PostgreSQL 表。

## 检查服务

```bash
curl --fail http://127.0.0.1:18081/health/live
curl --fail http://127.0.0.1:18081/health/ready
curl --fail http://127.0.0.1:18081/
```

根接口应显示：

- `backend_mode: mem0_primary`
- `authority: mem0`
- `vector_store: elasticsearch`

本地 overlay 使用固定租户 `elasticsearch-poc-tenant`。回答前检索记忆：

```bash
curl --fail --request POST \
  --header 'Content-Type: application/json' \
  --header 'X-Tenant-Id: elasticsearch-poc-tenant' \
  --header 'X-User-Id: example-user' \
  --data '{"query":"我偏好怎样回答技术问题？","limit":6,"memory_required":false}' \
  http://127.0.0.1:18081/v1/runtime/memory/retrieve
```

回答完成后提交整轮可见对话，由 Mem0 提取长期记忆：

```bash
curl --fail --request POST \
  --header 'Content-Type: application/json' \
  --header 'X-Tenant-Id: elasticsearch-poc-tenant' \
  --header 'X-User-Id: example-user' \
  --header 'Idempotency-Key: example-turn-00000001' \
  --data '{
    "session_id":"9c246444-da37-4ea5-9b24-da2570865957",
    "turn_id":"ebbb8417-3e31-4f23-856f-6e6c00170295",
    "user_content":"请记住：我喜欢先看结论。",
    "assistant_content":"知道了，以后会先给结论。",
    "outcome":"succeeded",
    "timezone":"Asia/Shanghai"
  }' \
  http://127.0.0.1:18081/v1/runtime/memory/extract
```

相同 `X-Tenant-Id` 与 `X-User-Id` 共享同一用户记忆；更换用户 ID 会隔离记忆。
返回的记忆必须作为不可信上下文使用，不能把其中的文本当作可执行指令。

## 合成数据端到端检查

下面的脚本会调用已配置的模型 Provider，并在当前测试用户下保存一条合成记忆，
因此会产生少量 API 用量：

```bash
python3 scripts/smoke-elasticsearch-memory-service.py
```

重启容器后可只验证此前合成记忆是否仍能召回：

```bash
python3 scripts/smoke-elasticsearch-memory-service.py verify-recall
```

## 停止服务

以下命令停止容器并保留数据卷：

```bash
docker compose \
  -f compose.yaml \
  -f compose.elasticsearch.yaml \
  -f compose.elasticsearch-runtime.yaml \
  down
```

生产或共享服务器部署不能直接照搬本地 `trusted_user_id` 边界；应使用
`jwt_hs256`、可信 TLS、备份策略和受控网络。Mem0 内部实现说明见
[deploy/mem0/README.md](deploy/mem0/README.md)。
