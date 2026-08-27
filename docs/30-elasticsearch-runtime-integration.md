# Elasticsearch Memory Runtime 接入与本地验收

## 1. 当前结论

Memory Service 已支持在启动时选择一个检索投影 Provider：

```dotenv
MEMORY_INDEX_PROVIDER=mem0
```

或：

```dotenv
MEMORY_INDEX_PROVIDER=elasticsearch
```

PostgreSQL 始终是权威库。Elasticsearch 只保存可重建的检索投影，搜索结果仍需经过 PostgreSQL 权威版本校验后才能进入 Context Pack。

当前不做 Mem0 与 Elasticsearch 双写。一个 Projection Generation 只能绑定一个 Provider、一个 Embedding 模型清单和一个维度。

## 2. 运行链路

```text
Agent Runtime
  │
  ├─ POST /v1/runtime/memory/extract
  │      │
  │      ├─ PostgreSQL：保存 Completed Turn、候选、版本和 Outbox
  │      └─ Projection Worker：领取 provider=elasticsearch 的任务
  │                            │
  │                            ├─ Embedding API：生成文档向量
  │                            └─ Elasticsearch：写入 Generation 索引
  │
  └─ POST /v1/runtime/memory/retrieve
         │
         ├─ Embedding API：生成查询向量
         ├─ Elasticsearch：按 namespace + generation 做 kNN
         ├─ PostgreSQL：BM25 通道与权威版本复核
         └─ Context Pack：形成并消费不可变快照
```

## 3. 本地隔离环境

本地验收使用三份 Compose 文件，只启动指定的三个服务：

```bash
cd /Users/drb/code/lazytt-memory-system

docker compose \
  -f compose.yaml \
  -f compose.elasticsearch.yaml \
  -f compose.elasticsearch-runtime.yaml \
  up --build -d \
  postgres elasticsearch memory-service-elasticsearch
```

访问地址：

| 服务 | 本机地址 | 说明 |
|---|---|---|
| Memory Service | `http://127.0.0.1:18081` | trusted user ID 测试实例 |
| Elasticsearch | `http://127.0.0.1:19200` | 仅本机 POC，认证已关闭 |
| PostgreSQL | `127.0.0.1:55432` | 独立 POC Volume |

该组合不会启动 Mem0。它使用独立 Compose Project `lazytt-memory-elasticsearch-poc`，不会读取正式 Memory Service 的 PostgreSQL Volume。

## 4. 自动验收

自然对话写入并召回：

```bash
python3 scripts/smoke-elasticsearch-memory-service.py
```

脚本会验证：

1. Runtime Extract 从自然对话产生记忆；
2. PostgreSQL 保存权威版本；
3. Projection Worker 把任务写入 Elasticsearch；
4. Runtime Retrieve 能召回该记忆；
5. 返回的 Context Pack 已被消费。

重启后的只读召回：

```bash
python3 scripts/smoke-elasticsearch-memory-service.py verify-recall
```

测试请求显式提供固定 `completed_at`。幂等重放时，请求体、Turn ID 和 `Idempotency-Key` 必须全部保持一致。

## 4.1 本地隔离模拟栈（非当前正式数据源）

该 Compose 只用于本机 PostgreSQL 18.4 + Elasticsearch 8.11.3 隔离模拟，不代表当前
服务器数据，也不应作为服务器迁移源：

```bash
docker compose -f compose.pg18-elasticsearch.yaml up --build -d
docker compose -f compose.pg18-elasticsearch.yaml ps
curl http://127.0.0.1:8080/health/ready
```

这条命令按以下顺序启动并等待健康检查：

```text
PostgreSQL 18.4 -> Elasticsearch 8.11.3 -> Memory Service
```

访问地址：

| 服务 | 本机地址 |
|---|---|
| Memory Service | `http://127.0.0.1:8080` |
| Elasticsearch | `http://127.0.0.1:9200` |
| PostgreSQL | `127.0.0.1:55433` |

Compose 继续使用迁移阶段已经验证的数据卷：

- `lazytt-memory-pg18-simulation-data`；
- `lazytt-memory-es-migration-data`。

停止容器但保留数据：

```bash
docker compose -f compose.pg18-elasticsearch.yaml down
```

不要执行 `down -v`，也不要对这些卷执行 `docker volume rm` 或
`docker volume prune`。

## 5. 停止与再次启动

停止容器，保留 PostgreSQL 和 Elasticsearch POC 数据：

```bash
docker compose \
  -f compose.yaml \
  -f compose.elasticsearch.yaml \
  -f compose.elasticsearch-runtime.yaml \
  down
```

再次执行第 3 节的 `up --build -d` 可以继续使用原来的 POC 数据。

不要在未确认 Volume 名称和数据用途前执行 `down -v`。

### 5.1 本机服务连接服务器 PostgreSQL 与 Elasticsearch

当前部署只在本机运行 Memory Service，权威 PostgreSQL 和 Elasticsearch 位于同一局域网
服务器。启动和停止命令为：

```bash
docker compose -f compose.server-runtime.yaml up --build -d
curl http://127.0.0.1:8080/health/ready

docker compose -f compose.server-runtime.yaml down
```

`.env` 需要提供服务器地址、PG 应用角色、ES Basic Auth 和模型配置。真实密码不能写进
Compose、文档或 Git。该 Compose 不会启动本地 PostgreSQL、Mem0 或 Elasticsearch。

当前 Memory Service 使用专用 PostgreSQL 资源：

```dotenv
MEMORY_APP_POSTGRES_USER=lazytt_memory_app
MEMORY_APP_POSTGRES_PASSWORD=<REDACTED>
MEMORY_APP_POSTGRES_DB=lazytt_memory
```

通用的 `lazytt` 超级用户不进入 Memory Service 容器，只用于管理员迁移和回滚操作。

## 6. Provider 与 Generation 隔离

- 投影操作的 `provider` 来自其 Generation 的 `index_provider`；
- Elasticsearch Worker 只领取 `provider=elasticsearch` 的任务；
- Mem0 Worker 只领取 `provider=mem0` 的任务；
- Elasticsearch 索引名由索引前缀和 Generation 哈希组成；
- 文档 ID 使用投影 Operation ID，因此相同操作重试只保留一个最终文档；
- 所有搜索和精确查询必须同时匹配 namespace 与 generation。

## 7. 安全配置

本地 POC 使用：

```dotenv
MEMORY_ALLOW_INSECURE_INDEX_HTTP=true
```

它只适用于 Docker 内部的 `http://elasticsearch:9200`，并且宿主机端口只绑定 `127.0.0.1`。

当前同一可信 Wi-Fi 下的临时服务器链路使用 HTTP + Basic Auth：

```dotenv
ELASTICSEARCH_HOST=<SERVER_IP>
ELASTICSEARCH_PORT=9200
ELASTICSEARCH_USERNAME=elastic
ELASTIC_PASSWORD=<REDACTED>
MEMORY_ALLOW_INSECURE_INDEX_HTTP=true
```

该方式只适合当前局域网联调。正式环境应改为 HTTPS，并优先使用最小权限 API Key：

```dotenv
MEMORY_INDEX_PROVIDER=elasticsearch
ELASTICSEARCH_BASE_URL=https://<ELASTICSEARCH_HOST>:9200
ELASTICSEARCH_API_KEY=<REDACTED>
ELASTICSEARCH_INDEX_PREFIX=lazytt-memory
ELASTICSEARCH_REQUIRED_VERSION=8.11.3
MEMORY_ALLOW_INSECURE_INDEX_HTTP=false
```

Basic Auth 密码或 API Key 只能放在密钥管理或权限为 `600` 的 `.env` 中，不能写入文档
和 Git。若 API Key 与 Basic Auth 同时配置，Memory Service 优先使用 API Key。

## 8. 当前已经验证

- Elasticsearch 8.11.3 版本校验；
- 1024 维 Qwen3 Embedding；
- 幂等写入、查询、精确查找和删除；
- namespace 与 generation 隔离；
- PostgreSQL 权威写入与 Elasticsearch 异步投影；
- Runtime Extract、Runtime Retrieve 和 Context Pack；
- Elasticsearch 与 Memory Service 重启后继续召回。
- PostgreSQL 18.4 权威库包含原 PostgreSQL 17 的全部对象；
- 三个非空租户共 23 条有效记忆从 Mem0 Generation 重建到 Elasticsearch；
- Canary 召回、Finalize、Elasticsearch 重启和 trusted user ID Runtime 召回。
- 服务器 PostgreSQL 18.4 恢复后核心表为 31 Records、32 Versions、31 Heads、41 Events，
  且不存在断裂 Head；
- 服务器 Elasticsearch 8.11.3 使用 Basic Auth，三个绿色索引共 23 个有效投影文档；
- 本机 Memory Service 连接服务器 PG/ES，重启后以 `trusted_user_id` 模式完成真实召回。

## 9. 当前没有验证

- 服务器 Elasticsearch 的 TLS 和最小权限 API Key（当前仅验证局域网 HTTP + Basic Auth）；
- 生产规模容量、延迟和故障恢复；
- Elasticsearch 集群多节点、高可用和快照备份；
- 多机故障切换与服务器级备份恢复演练。

这些项目必须在正式服务器迁移阶段单独验收，不能用本地 POC 结果替代。
