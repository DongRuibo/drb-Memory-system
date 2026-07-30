# Demo v0：真实记忆业务链

Demo v0 的目标不是模拟一个“看起来像记忆”的接口，而是跑通以下真实边界：

```text
Explicit Source Event
→ Candidate + deterministic Policy Decision
→ PostgreSQL Version/Event/Head/Outbox commit
→ asynchronous Worker
→ Mem0 infer=false Projection
→ Mem0 candidate IDs
→ PostgreSQL canonical authorization
→ Forget/Tombstone
→ immediate read denial + asynchronous Provider deletion
```

## 当前 Demo 范围

- Demo auth 模式使用服务端配置的 `demo-tenant / alice` Authority，并被强制限制在
  loopback；正式集成可切换短期 JWT；
- 只接受以 `请记住`、`记住` 或 `remember:` 开头的明确记忆请求；
- 只自动接受低风险 `user/preference`；
- 检索只允许 `purpose=answer_user`；
- 支持逻辑 Forget，不在 Demo v0 中执行物理 Erasure；
- Mem0 是可重建 Projection，返回的正文不会直接进入 API Response；
- 相同 `Idempotency-Key` 返回同一个 canonical operation；
- Provider unknown result 进入 reconciliation，重复 Provider object 会被收敛清理。

## 启动

### 1. PostgreSQL

```bash
docker compose -p lazytt-demo-v0 \
  -f deploy/local/postgres.compose.yaml up -d
```

Memory Service 默认执行 SQLx Migration。新环境不需要手工运行 SQL。

### 2. Mem0 与 Embedder

按 [Mem0 POC Runtime](../deploy/mem0/README.md) 启动固定 commit 的官方 Mem0 server。为了无外部凭据演示接口和生命周期，可以先启动本地契约 Embedder：

```bash
python3 tools/poc_embedding_server.py --host 0.0.0.0 --port 18080
```

默认输出 1536 维确定性向量。若复用一个已经由 1024 维语义模型创建的本地 Mem0 collection，
可显式传 `--dimensions 1024`；Memory Service 的 `MEMORY_EMBEDDING_DIMENSIONS` 必须使用
相同数值。

Mem0 `server/.env`：

```dotenv
OPENAI_API_KEY=poc-local-not-secret
OPENAI_BASE_URL=http://host.docker.internal:18080/v1
AUTH_DISABLED=true
MEM0_TELEMETRY=false
```

本地 Embedder 对所有输入返回相同非零向量，只用于验证 Filter、生命周期和权限交集。会议需要展示真实语义相关性时，应替换成已批准的真实 Embedder。

### 3. Memory Service

```bash
DATABASE_URL=postgres://memory:memory-local-only@127.0.0.1:55432/memory \
MEM0_BASE_URL=http://127.0.0.1:18888 \
MEMORY_AUTH_MODE=demo \
DEMO_TENANT_ID=demo-tenant \
DEMO_USER_ID=alice \
cargo run -p memory-service
```

JWT 集成模式使用：

```bash
MEMORY_AUTH_MODE=jwt_hs256 \
MEMORY_JWT_HS256_SECRET='<from-secret-manager-at-least-32-bytes>' \
MEMORY_JWT_ISSUER='https://identity.internal' \
MEMORY_JWT_AUDIENCE='lazytt-memory' \
MEMORY_TENANT_IDS='tenant-a,tenant-b' \
cargo run -p memory-service
```

JWT 必须包含 `sub`、`tenant_id`、`user_id`、`permissions`、`iat`、`nbf` 和 `exp`；
可选 `tenant_scope`、`project_ids`、`agent_ids` 表示签发时刻的 typed-scope
authority snapshot。`sub` 必须等于 `user_id`，最长寿命一小时。演示脚本默认继续
使用 loopback Demo 模式，不生成或保存认证凭据。

本项目约定将 Mem0 的容器内 `8000` 映射到宿主机 `18888`，避免与常见代理软件
使用的 `8888` 冲突。官方 Mem0 `server/docker-compose.yaml` 中应配置：

```yaml
ports:
  - "${MEM0_HOST_PORT:-18888}:8000"
```

可使用以下命令确认端口和服务：

```bash
lsof -nP -iTCP:18888 -sTCP:LISTEN
curl --noproxy '*' --fail http://127.0.0.1:18888/auth/setup-status
```

若 Rust 工具链暂时不在 `PATH`，但仓库已有构建产物，可以将
`cargo run -p memory-service` 替换为：

```bash
./target/debug/memory-service
```

确认依赖：

```bash
curl --fail http://127.0.0.1:8080/health/ready
```

### 4. 一键演示

```bash
./scripts/demo-v0.sh
```

脚本首先检查 `/health/ready`。如果服务或依赖未启动，会在写入任何 Demo
数据前停止并给出启动提示。

脚本会断言：

1. Source 被转换为正式 Memory；
2. 重复 Idempotency-Key 返回相同 Operation；
3. Worker 将 Accepted Version 投影到 Mem0；
4. 检索只返回 PostgreSQL 重新授权后的 canonical 正文；
5. Forget 提交后，即使 Provider 删除尚未完成也立即返回空结果；
6. 后台删除最终进入 `deleted`。

## 会议演示要点

不要只展示“向量库找到了内容”。建议同时展示 Operation Response 中的两个状态：

```json
{
  "canonical_status": "memory_accepted",
  "projection_status": "pending"
}
```

随后变为：

```json
{
  "canonical_status": "memory_accepted",
  "projection_status": "ready"
}
```

这能直观说明 PostgreSQL 已经接受正式记忆，而 Mem0 只是异步检索投影。Forget 演示则证明读取权限不依赖 Provider 删除速度。
