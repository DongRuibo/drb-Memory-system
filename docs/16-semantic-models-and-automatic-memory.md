# 真实语义模型与自动记忆

本节描述三个已经进入源码的能力：真实 Embedding、独立 Reranker，以及受控的自动
Memory Extractor。它们让演示从“用户必须说请记住”扩展为“自然对话也能提出记忆
候选”，但仍不允许任何模型直接修改权威记忆。

## 一句话理解

```text
Kimi Extractor 负责从对话中划重点
PostgreSQL + Policy 负责决定能不能记、如何去重和冲突
Qwen Embedding 负责从大量记忆中找候选
Qwen Reranker 负责把最相关的候选排到前面
Kimi k3 负责基于受控 Context Pack 生成最终回答
```

PostgreSQL `memory_core` 始终是权威数据；Mem0 和模型都只是可替换、可重建的
辅助组件。

## 固化的模型清单

| 职责 | 默认实现 | 固化身份 | 是否修改权威状态 |
|---|---|---|---|
| Memory Extractor | Kimi Code `k3` | `kimi-k3-memory-extractor-v2` | 否，只产生 Candidate Proposal |
| Embedding | `Qwen/Qwen3-Embedding-0.6B` | revision `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`、1024 维、L2 | 否，只维护检索投影 |
| Reranker | `Qwen/Qwen3-Reranker-0.6B` | revision `e61197e` | 否，只重排已授权候选 |
| Answer Model | Kimi Code `k3` | Demo Agent 配置 | 否，只生成回答 |

Embedding 与 Reranker 默认在本机 `127.0.0.1:18082` 的模型网关中运行。固定 revision
只代表当前可复现基线；升级必须经过评测并建立新的 projection generation。

## Embedding 与 generation 不变量

Migration `0013_projection_model_manifest.sql` 把 index provider、Embedding provider、
模型、revision、维度、归一化、指令版本、是否真实语义模型和 manifest hash 固化到
每个 generation。Readiness 会发送真实探针，并验证：

- 返回模型身份与声明一致；
- 只有一个向量，维度正确；
- 所有数值有限且向量非零；
- 声明为 L2 时 norm 在允许误差内；
- 当前 active/canary generation 的 manifest hash 与运行时完全一致。

Search、写入和 Worker 都再次检查 manifest。错误配置不能把 1536 维 POC 向量与
1024 维 Qwen 向量混入同一个 generation。

Mem0 当前的通用 OpenAI Embedding 请求不区分 query/document，因此当前 profile
使用 `mem0-symmetric-document-v1`：查询和文档都采用无指令的对称向量。网关保留
显式 `input_type=query` 能力，但要真正使用 Qwen query instruction，需要后续让
Retrieval Index Adapter 分开调用查询与文档接口。

## 独立 Reranker 契约

`MEMORY_RERANK_MODE=dedicated` 使用 `/v1/rerank`，不再让回答模型兼任排序。服务只
发送已经通过第一次 canonical 授权的有限候选；Adapter 要求响应：

- 模型身份匹配；
- 每个输入 index 恰好出现一次，不得增加候选；
- 分数有限且处于 `[0,1]`；
- 超大、缺失、重复或未知 index 响应全部拒绝；
- 排序返回后再次执行 canonical 授权，以防调用期间发生 Forget 或撤权。

Rerank 同时区分“主题相关性”和“记忆类型/查询意图适用性”。`preference` 属于跨问题
行为约束，不能仅因当前问题没有复述“回答风格”而被否决。`profile_fact`、
`project_fact`、`episodic_summary` 通常仍须达到语义阈值；但用户明确询问自己的工作
角色/编程语言、当前项目数据库或过去处理经验时，对应已授权且已召回的类别按确定性
查询意图保留，避免小模型对指代式中文的低分误删。系统保留模型原始低分，不伪造
相关性，并通过 `DEDICATED_RERANK_POLICY_APPLICABILITY` reason code 标明策略保留。
Search、透明降级和 Context Compiler 使用同一规则，避免前一层保留、后一层误删。

专用 Reranker 故障时，服务仍按现有透明排序路径可观测降级，不能把降级结果标成
模型正常排序。

## 自动提取与幂等

`POST /v1/memory/extractions` 接受一条自然对话消息。处理顺序是：

1. 校验 `memory:extract` permission、Tenant/User scope、大小和 Secret；
2. 用请求正文、来源、时间、作用域和 Principal 生成 request hash；
3. 在 `memory_core.extraction_operations` 建立 Tenant 隔离的幂等操作；
4. 服务端按当前已授权 Scope 决定允许提取的类型，Kimi 只返回最多 8 个严格 JSON
   Candidate Proposal；个人类型固定写入 User Scope，只有携带已认证 Project Scope 时才
   允许提出 `project_fact`；
5. 对类型、规范化 semantic key、正文、置信度、重要性、重复 identity 和 Secret
   再次校验；单条不合格候选被丢弃，不会让同批其他合格候选一起失败；
6. PostgreSQL 原子固化首个完成的候选快照，包括“零候选”；
7. 每个候选使用由主 Idempotency-Key + type + semantic key 派生的稳定子操作，进入
   现有 Policy、精确去重、来源合并、冲突/Review、Ledger 和 Outbox 链。

因此同一个 Idempotency-Key 重试不会再次接受一组漂移的模型结果；不同请求复用
同一 Key 会冲突。进程若在模型调用期间退出，操作保持 `pending`，相同模型版本可以
重试；未完成操作不能跨 Extractor 版本继续。

当前自动提取允许 `preference`、`profile_fact`、`episodic_summary`，derived 候选
置信度必须至少为 `0.80`。当请求同时携带服务端验证过的 Project Scope 和
`memory:write` 权限时，还允许显式来源支持的 `project_fact`，门槛为 `0.90`；模型不能
自行选择 Scope。新项目事实可自动接纳，同一 Project + semantic key 的不同值进入
`needs_review`，不能由模型静默覆盖。交互式 Web Demo 的受控 Runtime 会对已经通过
权限、类型、来源和置信度检查的低风险冲突自动提交 accept，追加新版本并向用户提示；
Procedure/Skill 仍不允许自动晋升。

## 启动和演示

模型安装、Mem0 配置和维护窗口重建见
[Semantic Retrieval Models](../deploy/models/README.md)。服务与 Demo 启动顺序：

```bash
./scripts/retrieval-models.sh
./scripts/run-memory-service-semantic.sh
./scripts/demo-auto-memory.sh
```

本机首次启动会下载两个约 0.6B 参数模型；应先用 `/health` 验证实际 model/revision，
再创建新 generation。不要在已有 1536 维 POC collection 上原地切换。

## 当前验收边界

源码、契约、迁移、脚本和纯单元测试已经落地；Migration `0013/0014` 与 10 个
PostgreSQL 集成测试已经通过，两个固定 Qwen 权重也已在 Apple MPS 完成真实 smoke：
1024 维向量 norm 为 1.0，相关/无关中文相似度为 0.737/0.300，Reranker 正确把无关
候选排在最后。全新 Mem0 1024 维 collection、generation 重建和切换已完成；真实
Kimi Extractor、专用 Qwen Rerank、Context Pack、Kimi 回答、Outcome Receipt、Forget
与投影删除的完整 HTTP 演练已经通过。无凭据证据见
[真实语义模型与自动记忆验收](../poc/evidence/semantic-models-automatic-memory-validation.md)。
这意味着会议 Demo 链已经可运行，不等于 Production Ready。

当前 macOS arm64/Python 3.14 环境已有精确依赖 lock。后续生产化还需要：各目标平台
独立 lock 与 SBOM、模型 artifact 内网分发、GPU/CPU
容量和 P95、质量回归阈值、Extractor 供应商数据条款、外部 OIDC/membership/Consent、
正式 tokenizer、Pack 签名与高可用双 generation 切换。
