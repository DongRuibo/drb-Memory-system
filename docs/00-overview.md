# 目标、边界和成熟度

## 目标

LazyTT Memory System 为 Agent Runtime 提供跨会话长期记忆：从有来源的事件中提出候选，经确定性治理后保存版本化记录，按当前权限检索，并为单次 Attempt 编译可追溯的 Memory Context Pack。

第一阶段服务以下场景：

- 用户明确表达的长期偏好；
- 稳定用户资料和项目事实；
- 与当前任务相关的已验证历史经验；
- 记忆修正、争议、过期、遗忘、导出和检查；
- Tenant、User、Project 和 Agent 作用域隔离。

## 非目标

第一阶段不负责：

- Canonical Business State、Task/Attempt Runtime State；
- 身份、ACL、Consent 或工具权限的权威存储；
- 从记忆自动授予执行权；
- 自动发布 Procedure/Skill；
- 完整时间知识图谱、自动 Reflection 和复杂关系推理；
- 用聊天记录或向量数据库替代可审计的 Memory Ledger。

## 系统承诺

系统只承诺“在证据、Policy 和当前 Authority 允许范围内，提供可能有帮助的长期信息”。它不承诺记忆永远正确，因此必须支持来源、置信度、冲突、有效期、修正和不注入。

关键边界：

```text
Canonical State 决定真实业务状态
Authority 决定允许读取与执行什么
Memory Ledger 保存已接受的长期信息
Retrieval Projection 加速查找
Memory Context Pack 服务一次 Attempt
```

## 当前成熟度

当前状态为 `Demo Agent v1 complete / MVP development in progress`：

- 已完成：权威 Ledger、Transactional Outbox、Mem0 Projection、读取权威交集、
  Forget、Attempt-bound Context Pack、Usage Receipt 和原子 Consume 的真实演示链；
- 已完成：相同语义且相同内容的正式去重与来源合并、显式修正的不可变 Version、
  Head 并发控制、Context Pack 失效和新旧投影收敛；
- 已完成：相同语义但正文不同的 `NeedsReview` Candidate、`Contested` Head、
  fail-closed 读取和接受/拒绝 Review 决策；
- 已完成：记忆检查与完整版本/事件历史、快照一致的用户导出、基于
  `retain_until`/`effective_to` 的自动过期，以及可跟踪并清理 canonical
  正文和 Projection 的 Erasure；
- 已完成：从签名 Principal 派生 Tenant/User 以及可选 Tenant/Project/Agent
  授权集合；检索、Inspect、Operation、Export、Context Pack 与 Project Erasure
  都执行 typed-scope 权威交集，撤掉 Project scope 后旧 Pack fail closed；
- 已完成：Mem0 与 PostgreSQL 本地候选的 RRF 融合、canonical pre-rerank
  授权、Kimi `k3` 语义 Reranker、失败时透明降级、模型返回后的 canonical
  二次授权，以及固定合成集的可复现检索/负查询/Forget 评测；
- 已完成：可复用 `memory-client` / `MemoryPort`、远程 HTTPS/Bearer 边界、
  Context Pack Manifest/Consume 一致性复核，以及 Demo Agent 参考接入；
- 已完成本机运行验收：Qwen3 真实 Embedding 探针和 generation model
  manifest、独立 Qwen3 Reranker、Kimi `k3` 自动 Candidate Extractor、抽取主操作
  幂等快照、新 collection 重建切换，以及自动模式 Demo Agent 的完整 HTTP 链；
- 正在进行：外部 membership authority 与撤权事件、生产规模检索评测、
  高可用双 generation 切换、目标平台供应链与 Agent Runtime 本体集成；
- 不允许：宣称 MVP/Production 已完成、接入生产用户数据或把 Mem0 当权威数据库；
- ADR-0001 仍需团队根据现有 POC 证据完成正式评审。

阶段状态以 [发布准入清单](./14-release-readiness-checklist.md) 为唯一判断依据，不以“接口已经能返回结果”作为完成标准。
