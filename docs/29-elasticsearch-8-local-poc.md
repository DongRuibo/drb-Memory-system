# Elasticsearch 8.11.3 本地投影验证

## 目的

本验证只回答一个问题：Elasticsearch 8.11.3 是否能承载 LazyTT 的可重建检索投影。

它不会替换当前 Mem0，也不会修改 PostgreSQL 权威记忆。验证通过后，才进入 Elasticsearch Adapter 和 Memory Service 接线阶段。

## 隔离边界

- Elasticsearch 只监听 `127.0.0.1:19200`；
- 本地 POC 关闭 Elasticsearch 认证，禁止把该 Compose 配置用于局域网或服务器；
- 索引固定为 `lazytt-memory-poc-v1`；
- 数据保存在独立 Volume `lazytt-memory-elasticsearch-poc_elasticsearch-poc-data`；
- Embedding 使用 `.env` 中已有的 `MEMORY_EMBEDDING_*` 配置，脚本不输出 API Key；
- 不启动 Memory Service、PostgreSQL 或 Mem0。

## 启动与验证

```bash
cd /Users/drb/code/lazytt-memory-system
chmod 600 .env
docker compose -f compose.elasticsearch.yaml up -d
docker compose -f compose.elasticsearch.yaml ps
python3 scripts/smoke-elasticsearch-projection.py run
```

首次启动需要下载 Elasticsearch 镜像。脚本验证：

1. 1024 维 `dense_vector` Mapping；
2. 确定性文档 ID 和重复写入幂等；
3. 向量检索；
4. BM25 文本检索；
5. namespace 与 generation 过滤；
6. 精确查询；
7. 删除；
8. 写入持久化标记。

如果使用本地 LazyTT Embedding 服务，把下列变量放入 `.env`：

```dotenv
MEMORY_EMBEDDING_BASE_URL=http://127.0.0.1:18082
MEMORY_EMBEDDING_REQUEST_DIALECT=lazytt
```

远程 OpenAI 兼容服务默认使用 `openai` 请求方言。

## 验证重启持久化

```bash
docker compose -f compose.elasticsearch.yaml restart elasticsearch
docker compose -f compose.elasticsearch.yaml ps
python3 scripts/smoke-elasticsearch-projection.py verify-persistence
```

只有在 Elasticsearch 恢复为 `healthy` 后，再运行持久化检查。

## 停止与回滚

停止容器但保留 POC 数据：

```bash
docker compose -f compose.elasticsearch.yaml down
```

再次执行 `up -d` 可以继续使用原来的测试索引。

本阶段不提供自动删除 Volume 的命令。需要清除 POC 数据时，应先确认 Volume 的准确名称再单独操作。

## 通过标准

- 两次验证命令都返回 `"status":"ok"`；
- 容器重启后仍能读取持久化标记；
- 现有 Memory Service、PostgreSQL 和 Mem0 的容器与 Volume 均未被启动、修改或删除。

Memory Service 接线和端到端验收见 [Elasticsearch Memory Runtime 接入与本地验收](./30-elasticsearch-runtime-integration.md)。
