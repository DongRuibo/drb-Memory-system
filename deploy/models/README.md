# Semantic Retrieval Models

本目录提供本机真实语义模型的参考部署。模型网关同时暴露：

- `POST /v1/embeddings`：OpenAI-compatible Embedding；
- `POST /v1/rerank`：LazyTT bounded Rerank contract；
- `GET /health`：实际模型、revision、维度与设备。

默认模型是 `Qwen/Qwen3-Embedding-0.6B`（revision
`97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`、1024 维）和
`Qwen/Qwen3-Reranker-0.6B`（revision `e61197e`）。服务遵循 Qwen 官方实现：Embedding 使用 last-token
pooling、MRL 维度裁剪和 L2 归一化；Reranker 使用 yes/no logits 得到 `[0,1]`
相关性分数。请求日志只保留 SHA-256 和长度，不记录正文。

## 1. 安装与启动

### Docker profile（推荐给下载测试者）

根目录 Compose 已把本地模型接入完整后端：

```bash
# 任意 Docker 主机，模型在 CPU 容器中
./scripts/backend-stack.sh cpu up

# Linux x86_64 + NVIDIA Container Toolkit，模型在 CUDA 容器中
./scripts/backend-stack.sh cuda up
```

两个模型共享持久化的 Hugging Face 缓存卷，首次运行下载权重，后续重建容器不会重复
下载。Apple Silicon 的 MPS 不能直接透传给 Docker Linux 容器；Mac 上需要 GPU 加速时，
先按下一节在宿主机启动模型，再执行 `./scripts/backend-stack.sh host-mps up`。完整说明见
[后端 Docker 本地部署](../../docs/22-backend-docker-deployment.md)。

### 宿主机 Python / Apple MPS

建议使用独立虚拟环境；不要把模型缓存或凭据提交到仓库。

```bash
python3 -m venv .venv-retrieval-models
. .venv-retrieval-models/bin/activate
python -m pip install -r deploy/models/requirements.lock.txt
./scripts/retrieval-models.sh
```

`requirements.lock.txt` 是已在 macOS arm64/Python 3.14 实际 smoke 的精确环境；其他
平台可先用 `requirements.txt` 的兼容范围解析，再生成并评审该平台自己的 lock。
脚本默认设置 `HF_HUB_DISABLE_XET=1`，避免常见本机代理下的大文件 Xet 连接卡住；
已验证 Xet 的环境可显式设为 `0`。

首次启动需要从模型仓库下载权重。若生产环境禁止运行时下载，应预先把模型权重放入
受控 artifact store。参考脚本已经固定当前 commit；升级时必须显式修改
`MEMORY_EMBEDDING_REVISION` 与 `MEMORY_DEDICATED_RERANK_REVISION`，重新评测并建立
新 generation。

## 2. 配置 Mem0

Mem0 的正式写入仍使用 `infer=false`，但会调用 Embedder。把官方 Mem0 server 的
OpenAI-compatible Embedder 指向宿主机网关：

```dotenv
OPENAI_API_KEY=local-semantic-model-no-secret
OPENAI_BASE_URL=http://host.docker.internal:18082/v1
MEM0_TELEMETRY=false
```

网关默认把 Mem0 当前常用的 `text-embedding-3-small` 请求名显式路由到实际 Qwen
模型，但 generation manifest 永远记录真实模型名，不把 alias 当作模型身份。
由于该通用接口不区分查询和文档，Mem0 profile 当前对两者都使用无指令的对称
Embedding，manifest 记为 `mem0-symmetric-document-v1`。这是真实语义向量，但不是
Qwen 推荐的非对称 query-instruction 最优检索配置。

从 1536 维 POC Embedder 切换到 1024 维 Qwen 前，必须使用新的 Mem0 collection/
数据库或清空仅含合成 POC 数据的 collection，然后从 PostgreSQL 权威记录重建。
不同维度不能写入同一个 pgvector collection。当前单 Mem0 实例的切换需要维护窗口；
无停机双读需要为旧、新 manifest 各部署一个 Retrieval Index profile，尚未实现。

## 3. 启动 Memory Service

```bash
./scripts/run-memory-service-semantic.sh
```

服务启动配置会生成不可伪造的 `model_manifest_hash`。Readiness 会调用一次真实
Embedding，并验证模型名、1024 维、有限非零向量与 L2 norm。读写和 Worker 还会
校验 generation 的 manifest；不一致时返回 `PROJECTION_MODEL_MISMATCH` 或把错误
投影标为 `EMBEDDING_MANIFEST_MISMATCH`。

初次切换时创建并激活新 generation：

```bash
SOURCE_GENERATION=demo-v0-generation-1 \
  ./scripts/rebuild-projection.sh qwen3-embedding-0.6b-v1
```

若旧 collection 已被替换，这次迁移不具备旧向量 canary rollback 能力；必须先备份
并把它当作维护窗口重建。后续在同一模型/维度 manifest 内的 generation 重建仍可
使用现有 canary/finalize/rollback 流程。

## 4. 自动记忆演示

Memory Service 使用 `MEMORY_EXTRACT_MODE=kimi` 后：

```bash
./scripts/demo-auto-memory.sh
```

自动提取包含一次外部模型调用，脚本会把 Memory API timeout 默认提高到 30 秒；可按
已测 P95 用 `MEMORY_TIMEOUT_SECONDS` 覆盖，但客户端上限仍为 30 秒。
语义服务和自动演示脚本默认共同使用隔离的 `DEMO_USER_ID=semantic-demo`。Web 演示
还默认使用 `DEMO_PROJECT_ID=lazytt-memory-system-demo`；Memory Service 与 Web API
必须显式使用相同的 User/Project ID，否则项目事实会按授权边界拒绝。CLI 自动演示只写
User Scope，不受 Project Scope 影响。

演示链为“自然对话 → Kimi 候选提取 → Policy/去重/冲突 → PostgreSQL 权威写入 →
真实 Embedding → 专用 Rerank → Context Pack → Kimi 回答 → Forget”。

自动提取主请求和候选子操作都使用数据库幂等：首个完整候选集合（包括零候选）会
固化，重试不会再次接受漂移的模型结果。详细契约见
[真实语义模型与自动记忆](../../docs/16-semantic-models-and-automatic-memory.md)。

## 5. 当前验收状态

网关实现、固定 revision、服务探针、脚本和测试已完成；固定模型已在 macOS MPS
完成健康、1024 维 L2 中文向量和中文 Rerank smoke。Migration `0013/0014`、新建
`memories_qwen3_1024` collection、从 PostgreSQL 重建并切换 generation，以及自动
记忆 Demo 全链均已实际通过。证据见
[真实语义模型与自动记忆验收](../../poc/evidence/semantic-models-automatic-memory-validation.md)。

这份验收只证明当前本机会议 Demo profile 可用。正式发布仍需要目标平台 lock/SBOM、
固定质量集和阈值、P95/容量测试、模型 artifact 供应链以及高可用切换演练。
