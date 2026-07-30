# Mem0 POC Runtime

本仓库不复制或修改 Mem0 官方 Compose。先按官方 self-hosted server 文档启动固定 commit/tag 的 `server/` stack，再让 Rust POC 通过 REST 连接。当前可复现证据使用 commit `b357a5a1b03c299ec8229c268e63cfac0f7c6566`。

要求：

- Docker Desktop 已启动；
- Mem0 server 仅监听本机/私网；
- 生产式验证必须保持 Auth 开启并使用专用 API Key；只有隔离、本机、全合成数据的契约 POC 才允许临时 `AUTH_DISABLED=true`；
- 在 `poc/evidence/mem0-manifest.md` 记录 Mem0 commit、容器 digest、LLM、Embedder 和配置；
- 不把真实用户数据用于 POC。

## 本机端口约定

Mem0 容器内仍监听 `8000`，宿主机统一使用 `18888`，避免占用代理软件常用的
`8888`。在官方 `server/docker-compose.yaml` 中配置：

```yaml
services:
  mem0:
    ports:
      - "${MEM0_HOST_PORT:-18888}:8000"
```

如需临时使用其他端口，可在启动 Compose 时设置 `MEM0_HOST_PORT`，并让
`MEM0_BASE_URL` 使用同一个端口。

运行 Adapter 健康检查：

```bash
MEM0_BASE_URL=http://localhost:18888 \
MEM0_API_KEY=replace-with-local-poc-key \
cargo run -p poc-cli -- health
```

运行合成数据的 upsert/search/delete：

```bash
MEM0_BASE_URL=http://localhost:18888 \
MEM0_API_KEY=replace-with-local-poc-key \
cargo run -p poc-cli -- cycle
```

`cycle` 的 upsert 固定发送 `infer=false`，只验证“已接受 Version → 可重建检索投影”。Mem0 的 `infer=true` 自动提取会同时落库，只能在隔离 namespace 中做实验，不得接入正式写入链路。

## 无外部凭据的契约 POC

本仓库提供 [tools/poc_embedding_server.py](../../tools/poc_embedding_server.py)。它实现最小 OpenAI-compatible `/v1/embeddings`，日志只保留输入 SHA-256，不记录正文；所有文本返回相同的非零向量，所以只能验证 REST、metadata filter、PostgreSQL 持久化和删除，不能用于语义质量评测。

```bash
python3 tools/poc_embedding_server.py --host 0.0.0.0 --port 18080
```

在 Mem0 `server/.env` 中配置：

```dotenv
OPENAI_API_KEY=poc-local-not-secret
OPENAI_BASE_URL=http://host.docker.internal:18080/v1
MEM0_TELEMETRY=false
```

然后运行：

```bash
MEM0_BASE_URL=http://localhost:18888 cargo run -p poc-cli -- cycle
MEM0_BASE_URL=http://localhost:18888 cargo run -p poc-cli -- persist-write
# 重启 Mem0 服务，但保留它的 PostgreSQL
MEM0_BASE_URL=http://localhost:18888 cargo run -p poc-cli -- persist-verify
MEM0_BASE_URL=http://localhost:18888 cargo run -p poc-cli -- duplicate-probe
```

`duplicate-probe` 会自动清理合成投影。当前固定版本会为相同 `operation_id` 创建两个 Provider object，因此生产 Worker 不能依赖 Mem0 实现幂等；必须由本系统的 Projection Operation/Mapping 层在调用前后约束。
