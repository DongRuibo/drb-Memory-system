# LazyTT Memory System

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
