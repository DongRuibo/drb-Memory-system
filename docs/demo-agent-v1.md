# Demo Agent v1：真实模型使用记忆

Demo Agent v1 在 Demo v0 的记忆后端之上接入 Kimi Code `k3`，用于展示一条
可观察的真实 Agent 记忆链：

```text
明确的“请记住”请求，或自动模式下的一条自然对话
→ 可选 Kimi Extractor 只产生受控 Candidate Proposal
→ Memory Service canonical commit
→ Outbox / Mem0 projection
→ 新 Task / Attempt 编译无正文 Context Pack Manifest
→ 模型调用前原子 Consume，再次复核 Authority、Head、Version、Tombstone 和来源
→ PostgreSQL 标记 consumed 并交付未信任记忆数据
→ Agent 将消费结果作为未信任数据注入
→ Kimi k3 回答
→ Forget / Tombstone 立即阻断读取并使旧 Pack 失效
→ 另一个新 Task / Attempt 获得不含该记忆的新 Pack 并调用 Kimi
→ Mem0 projection 异步删除
```

这个 Demo 会产生两次真实模型请求，可能产生供应商费用。

## 1. 先轮换密钥

任何已经出现在聊天、日志、截图或终端历史中的 API Key 都应视为已经暴露。
运行 Demo 前，先在 Kimi 控制台撤销旧 Key 并创建新 Key。不要把 Key 写入仓库、
脚本、命令参数或会议材料。

在当前终端静默注入新 Key。macOS 默认 `zsh`：

```zsh
read -r -s 'KIMI_API_KEY?KIMI_API_KEY: '
printf '\n'
export KIMI_API_KEY
```

如果使用 `bash`：

```bash
read -r -s -p 'KIMI_API_KEY: ' KIMI_API_KEY
printf '\n'
export KIMI_API_KEY
```

完成演示后清理当前 Shell 中的变量：

```bash
unset KIMI_API_KEY
```

Demo 不打印 Key，Kimi 客户端也把 `Authorization` Header 标记为敏感数据。正式
部署仍应使用 Secret Manager 和短生命周期凭据，而不是进程环境变量。

本地 Demo 也支持仓库根目录的 `.env`。该文件已被 `.gitignore` 排除，必须是普通
文件且权限不得允许 group/others 读取：

```bash
chmod 600 .env
```

支持当前简写：

```dotenv
apikey=<rotated-local-key>
base_url=https://api.kimi.com/coding/v1
model_name=k3
```

也支持标准变量名 `KIMI_API_KEY`、`KIMI_BASE_URL`、`KIMI_MODEL`。Shell 环境变量
优先于 `.env`；解析器不会 `source` 文件或执行其中的 Shell 内容。可通过
`DEMO_AGENT_ENV_FILE` 指定其他仅本机可读的配置文件。

## 2. Kimi k3 配置

Demo 使用 Kimi Code 提供的 OpenAI 兼容 Chat Completions 协议：

```bash
export KIMI_BASE_URL='https://api.kimi.com/coding/v1'
export KIMI_MODEL='k3'
export KIMI_REASONING_EFFORT='low'
```

`KIMI_BASE_URL` 和 `KIMI_MODEL` 已有上述默认值。客户端只接受这个官方 HTTPS
地址；仅测试时允许 loopback 地址。可选配置：

| 环境变量 | 默认值 | 约束 |
| --- | --- | --- |
| `KIMI_REASONING_EFFORT` | `low` | `low`、`high` 或 `max`；演示默认低延迟 |
| `KIMI_MAX_COMPLETION_TOKENS` | `2048` | `256` 至 `8192` |
| `KIMI_TIMEOUT_SECONDS` | `90` | `5` 至 `300` |
| `DEMO_AGENT_MEMORY` | 内置讲解偏好 | 1 至 8192 字节 |
| `DEMO_AGENT_QUESTION` | 内置 Outbox 问题 | 1 至 4096 字节 |

## 3. 启动 Memory Service

先按 [Demo v0](./demo-v0.md) 启动 PostgreSQL、Mem0 和 Embedder，然后在独立终端
启动 Memory Service：

```bash
DATABASE_URL=postgres://memory:memory-local-only@127.0.0.1:55432/memory \
MEM0_BASE_URL=http://127.0.0.1:18888 \
DEMO_TENANT_ID=demo-tenant \
DEMO_USER_ID=alice \
cargo run -p memory-service
```

Mem0 默认使用宿主机 `18888`，不会再占用代理软件常用的 `8888`。确认服务和
依赖均已就绪：

```bash
curl --fail http://127.0.0.1:8080/health/ready
```

Memory Service 的 Demo API 没有认证时，`demo-agent` 只允许连接 loopback 地址。
如果设置 `MEMORY_BEARER_TOKEN`，客户端可连接远程服务，但非 loopback 地址必须
使用 HTTPS。不要把无认证的 Demo 服务直接暴露到其他网络。

## 4. 运行真实链路

在已静默注入 `KIMI_API_KEY`，或已经准备好权限为 `0600` 的 `.env` 后执行：

```bash
./scripts/demo-agent-v1.sh
```

默认 `DEMO_AGENT_MEMORY_MODE=explicit` 保留既有稳定演示。完成真实语义模型部署并以
`MEMORY_EXTRACT_MODE=kimi` 启动服务后，执行：

```bash
./scripts/demo-auto-memory.sh
```

自动模式不发送“请记住”命令，而是调用 `POST /v1/memory/extractions`。Extractor
只能提出 Candidate；首个候选集合先写入幂等快照，再经过同一 Policy、去重、冲突/
Review、Ledger 和 Outbox 链。会议主 Demo 使用前必须先完成一次本机全链彩排。

脚本会先检查 Key 是否非空，但不会打印其内容；随后检查默认的
`http://127.0.0.1:8080/health/ready`。如设置了 `MEMORY_BASE_URL`，预检和
Agent 会共同使用该地址；远程集成还必须设置短期 `MEMORY_BEARER_TOKEN`。

脚本优先执行：

```bash
cargo run -p demo-agent
```

如果 `cargo` 不在 `PATH`，则使用已有的：

```bash
./target/debug/demo-agent
```

成功输出包含六个阶段：

1. canonical memory accepted；
2. Mem0 projection ready；
3. 新 Task / Attempt 使用授权记忆调用 Kimi；
4. canonical Forget accepted；
5. 另一个新 Task / Attempt 在立即读取阻断后调用 Kimi；
6. Provider projection deleted。

重点观察两次回答前的 `context_pack_id`、`memory_context_count` 和
`used_memory_ids`：第一次 Pack 包含本次新建的 Memory ID，Forget 后的第二次
Pack 为空。模型正文用于直观展示回答风格变化，权威验收依据仍是 canonical
状态、Context Pack/Memory ID、Usage Receipt 和删除状态。

## 5. 无外部凭据的 Agent 编排验收

仓库提供本地确定性 Chat Completions 假服务。它不是模型，只用于在不外发数据、
不消耗额度的情况下验证 `demo-agent` 的完整 HTTP 编排：

```bash
python3 tools/poc_chat_completion_server.py \
  --host 127.0.0.1 \
  --port 18081
```

在另一个终端中运行：

```bash
KIMI_API_KEY=poc-local-not-secret \
KIMI_BASE_URL=http://127.0.0.1:18081/coding/v1 \
./scripts/demo-agent-v1.sh
```

这个本地服务只记录 user payload 的 SHA-256、上下文条数和合成 request ID，不记录
问题、记忆正文或 Authorization。它能证明协议和 Agent 流程，不能替代真实 Kimi
模型验收。

## 当前限制

- Demo Context Compiler 支持 `preference`、`profile_fact`、`semantic_fact`、
  `episodic_summary` 和 Tenant/User/Project/Agent typed scope，但仍只支持
  `purpose=answer_user`。它使用 `demo-conservative-char-v1`（每个 Unicode 字符加
  固定结构开销）的保守估算器；这不是 Kimi `k3` 的真实 tokenizer。
- 在线候选已由 Mem0 + PostgreSQL 本地检索经 RRF 融合，可选兼容的
  `kimi-k3-semantic-v1` 或独立 `Qwen/Qwen3-Reranker-0.6B` 语义 Rerank，失败时
  透明降级。固定 15 条正查询 smoke
  评测已通过，但尚未实现完整多特征排序解释或生产规模质量/延迟/成本评测。
- Context Pack 和 Usage Receipt 已由服务端创建并落库；Compile 不返回正文，
  Consume 使用 Attempt-bound 幂等键并在交付前原子复核。Forget 先提交时 Consume
  返回冲突；Consume 先完成时表示记忆已在该事务点正式交付给本次 Attempt。
- Demo Usage Receipt 记录 `injected/omitted`、原因、Provider score 和 rank；
  Attempt-bound Outcome Receipt 已实现，Pack 签名以及 Policy/ACL 变化驱动的批量
  失效尚未实现。
- 自动提取默认处理 User Scope 的偏好、用户事实和经历；交互式 Web Demo 还会携带
  `DEMO_PROJECT_ID` 对应的已授权 Project Scope。只有明确的高置信项目事实会自动接纳，
  冲突值先进入底层 Review 状态，再由受控 Web Runtime 自动接受、追加版本并提示；页面
  不为普通记忆或允许自动更新的项目事实弹确认。
- 旧 POC Embedder 仍可用于纯契约测试；新的 semantic profile 使用固定 revision 的
  Qwen3 Embedding 和独立 Qwen3 Reranker。最新 Migration、新 collection 和真实
  权重的端到端验收记录尚未完成。
- Memory API 已提供内部 MVP 用短期 HS256 JWT、permission 和 typed-scope
  授权，但演示脚本默认仍使用仅绑定 loopback 的 `demo` 模式。OIDC/JWKS、
  key rotation 和主动撤权事件尚未完成。
- 后端已具备 Review、Correction、Conflict、Expiry、Export 与 Erasure；Demo Agent
  现在同时支持明确写入和单消息自动提取，但尚未实现长期多轮对话编排、反思或自动
  Procedure/Skill 晋升。
- Demo 会确认上下文中是否存在目标 Memory ID，但不会把模型生成的标题或措辞
  当成确定性测试断言。
- Demo Agent 已通过可复用 `memory-client` / `MemoryPort` 访问 Memory API，但它
  仍是一条独立演示应用链，不代表 LazyTT Agent Runtime 本体完成正式集成，也不
  代表 MVP 或 Production Gate 已通过。
