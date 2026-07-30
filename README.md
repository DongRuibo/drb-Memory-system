# LazyTT Memory System

LazyTT Agent Runtime 的长期记忆子系统。当前仓库已经完成
**Architecture Baseline + Mem0 POC + Memory Service Demo v0**：
PostgreSQL canonical Ledger、Repository、Transactional Outbox、Mem0 Retrieval
Projection、Search authority recheck 和 Forget 链路均已落地，Demo v0 已完成本地
端到端验收。仓库同时提供 `demo-agent`，用于接入 Kimi Code `k3` 展示真实模型在
Forget 前后使用不同记忆上下文；Demo v1 服务端 Context Pack、确定性预算、
Usage Receipt、Forget invalidation 和消费前原子校验已落地。精确语义去重、
来源合并、显式 Correction 的不可变版本链，以及同一语义不同正文的
NeedsReview/Contested/Review 链也已完成。Inspect/History、快照一致的用户导出、
自动 Expiry 和可跟踪 Erasure 已完成真实 PostgreSQL 与 HTTP/投影验证。正式认证
已具备短期 JWT、独立 API permission、Tenant allowlist、Tenant Worker，以及
Tenant/User/Project/Agent typed-scope 读取权限交集。Project Erasure 和权限移除
拒绝已通过 PostgreSQL 集成测试。检索已升级为 Mem0 + PostgreSQL 本地候选的
RRF 融合。最新源码已经加入固定 manifest 的真实 Qwen3 Embedding、独立 Qwen3
Reranker，以及 Kimi `k3` 自动候选提取；抽取结果先固化为 Tenant 隔离的幂等快照，
再进入现有 Policy、去重、冲突/Review、Ledger 和 Outbox 链。固定合成评测集和
Forget 零注入断言已经落地。真实模型权重、最新 Migration、新 collection 重建和
自动记忆完整 HTTP 演练已经在本机通过。外部 membership authority 与撤权事件、正式
tokenizer、生产规模检索评测、Agent Runtime 本体集成、MVP 和 Production Gate
仍未完成。可复用的
`memory-client` / `MemoryPort` 及 Demo Agent 参考接入已经实现。Projection generation
的 rebuild/校验/canary/finalize/rollback 状态机和 API 已实现，但最新 migration
与真实 Mem0 的端到端重建演练已经完成；高可用双 generation 切换和生产回退演练
仍按“未完成”管理。

核心原则：

- 模型和 Provider 只能提出候选或维护检索投影，不能修改正式状态；
- PostgreSQL `memory_core` 中的版本、账本事件和当前头是权威数据；
- 权限先于检索，索引命中不等于允许读取；
- Memory、Runtime State、Canonical State 和单次 Attempt 的 Context 必须分开；
- 修正、遗忘、撤权和来源变化必须传播到所有检索投影与缓存。

## 文档入口

- [领域语言](./CONTEXT.md)
- [文档地图](./docs/README.md)
- [项目与代码架构](./docs/18-project-and-code-architecture.md)
- [系统架构与信任边界](./docs/02-architecture-and-trust-boundaries.md)
- [开发准入和发布检查](./docs/14-release-readiness-checklist.md)
- [架构决策记录](./docs/adr/README.md)
- [Mem0 POC 证据](./poc/evidence/mem0-manifest.md)
- [POC 验证报告](./poc/evidence/validation-report.md)
- [Demo v0 真实业务链](./docs/demo-v0.md)
- [Demo Agent v1：Kimi k3 真实模型链](./docs/demo-agent-v1.md)
- [Web 演示计划：让外行看懂 Agent 记忆](./docs/demo-web-presentation-plan.md)
- [Agent Runtime 集成契约](./docs/15-agent-runtime-integration.md)
- [真实语义模型、独立 Reranker 与自动记忆](./docs/16-semantic-models-and-automatic-memory.md)
- [本地语义模型部署](./deploy/models/README.md)
- [Demo Agent v1 本地编排证据](./poc/evidence/demo-agent-v1-validation.md)
- [Demo Agent v1 真实 Kimi 验收证据](./poc/evidence/demo-agent-v1-kimi-validation.md)
- [Demo Context Pack 本地端到端证据](./poc/evidence/demo-context-pack-validation.md)
- [Demo Context Pack 原子消费证据](./poc/evidence/demo-context-pack-consume-validation.md)
- [Demo 去重与 Correction 验证证据](./poc/evidence/demo-dedupe-correction-validation.md)
- [Demo 冲突与 Review 验证证据](./poc/evidence/demo-conflict-review-validation.md)
- [Demo Inspect、Export、Expiry 与 Erasure 验证证据](./poc/evidence/demo-inspect-export-expiry-erasure-validation.md)
- [Demo JWT Principal 与跨用户隔离验证证据](./poc/evidence/demo-jwt-authorization-validation.md)
- [Typed Scope 与 Project Erasure 验证证据](./poc/evidence/demo-typed-scope-validation.md)
- [真实语义检索固定评测证据](./poc/evidence/demo-retrieval-evaluation-validation.md)
- [真实语义模型与自动记忆验收证据](./poc/evidence/semantic-models-automatic-memory-validation.md)
- [Demo Web 本地验收证据](./poc/evidence/demo-web-validation.md)
- [Web 四类记忆统一演示验收证据](./poc/evidence/web-four-memory-e2e-validation.md)

## 当前可运行验证

```bash
cargo test --workspace
docker compose -f deploy/local/postgres.compose.yaml up -d
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /tests/sql/ledger_invariants.sql
```

Mem0 的本地无外发 POC 启动与 `health/cycle/persist/duplicate` 命令见 [deploy/mem0/README.md](./deploy/mem0/README.md)。POC Embedder 只验证接口与过滤行为，不代表语义检索质量。真实语义链使用本地 Qwen3 模型网关，并必须创建新的 1024 维 generation/collection。

隔离启动评测专用 Memory Service 后，可运行 `./scripts/eval-retrieval-v1.sh`。
它会写入合成记忆、比较 no-memory/本地词法/Mem0/混合服务/oracle，执行负查询与
Forget 后零注入检查，并把版本、原始结果、失败样本和指标写入
`target/evals/retrieval-v1/<evaluation-id>/`。

Projection 重建的受控操作入口是 `./scripts/rebuild-projection.sh <new-generation>`；
默认完成 canary 后 finalize，设置 `REBUILD_SWITCH_ACTION=rollback` 可演练回退。
在真实 PostgreSQL/Mem0 验收证据完成前，不应把该脚本用于生产变更。

Memory Service 就绪后，可先执行 `./scripts/demo-v0.sh` 验证后端生命周期。静默
注入已轮换的 `KIMI_API_KEY` 后，再执行 `./scripts/demo-agent-v1.sh` 演示
`remember → fresh Agent attempt → Kimi → forget → fresh Agent attempt`。脚本和
文档不会保存或输出真实 Key。

完成 [真实语义模型部署](./deploy/models/README.md) 后，执行
`./scripts/demo-auto-memory.sh` 可演示
`自然对话 → 自动候选 → 受控入库 → 真实语义检索/重排 → 回答 → Forget`。这条
链路已经完成本机全量运行验收，可作为会议主 Demo；首次模型下载、MPS 预热和外部
Kimi 网络延迟仍应在会前完成。

面向非技术观众时，可执行 `./scripts/demo-web.sh`，然后打开
`http://localhost:3000`。Web 界面采用常见大模型对话布局，通过“自然认识用户与
项目、跨对话按需使用、项目事实自动更新并提示、指定经历立即忘记”展示一条统一业务链。
偏好、用户事实和经历进入 User Scope；已授权的高置信项目事实进入 Project Scope。
对话、流式事件和 Outcome Receipt 均持久化，刷新或 SSE 重连不会把已生成内容丢掉。

`doc/` 保存早期调研和设计输入；`docs/` 是当前规范性文档。两者冲突时，以 `docs/`、OpenAPI/事件契约和已接受 ADR 为准。
