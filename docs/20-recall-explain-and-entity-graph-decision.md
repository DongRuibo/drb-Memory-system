# Recall Explain 合并结果与实体图决策

状态：2026-07-30 实现决策。本文只描述本阶段已经实现的 Explain 能力，以及明确暂缓的
实体图范围。

## 1. 从 mem0-memLayer 吸收了什么

钟佳栋项目中最有复用价值的不是管理台页面本身，而是 Recall Evidence 的数据思想：一次
检索应能回答“候选来自哪一路、RRF 加了多少、在哪个授权或排序阶段被排除、最终为何
入选”。LazyTT 将这一思想适配到自身的 canonical Version 与双重授权链路：

```text
Provider / Chinese BM25 / Lexical / Temporal
  -> channel rank + raw score + weight + RRF contribution
  -> fused score
  -> pre-rerank canonical policy
  -> reranker score
  -> post-rerank canonical authorization
  -> relevance threshold / final limit
  -> included 或稳定 exclusion_reason
```

每次 `POST /v1/memory/search` 都在 `memory_core.retrieval_runs` 和
`memory_core.retrieval_candidate_receipts` 中写入同事务回执，并返回
`retrieval_run_id`。管理员可使用：

- `POST /v1/memory/retrieval:explain`：执行一次真实检索并同时返回结果与完整 Run；
- `GET /v1/memory/retrieval-runs/{retrieval_run_id}`：读取已持久化 Run。

两个管理接口都要求 `memory:admin`。普通 `memory:read` 调用无法读取候选证据。

## 2. 数据最小化差异

这里没有原样复制源项目的 `sanitized_query_preview`。当前实现只保存：

- 边界清洗后 Query 的 `sha256:` hash；
- Task/Attempt/Purpose、固定 reference time、时区与解析后的时间窗口；
- Projection generation、Ranking version 与降级理由；
- canonical Record/Version ID、通道分数和阶段决策。

它不保存原始 Query、候选正文、Prompt、Provider 原始对象或认证信息。因此 Explain 是可
审计的决策元数据，不是第二份聊天记录或记忆内容副本。

## 3. 为什么暂缓实体图

钟佳栋项目的图检索建立在模型同时输出 `entities`、`relations` 的分析契约上，并用
PostgreSQL 保存 Entity、Alias、Mention 和 Relation，再做最多两跳递归召回。这对
“别名指向同一实体”和“跨记忆关系查询”有价值，但当前 LazyTT 不能安全地直接搬运：

1. 当前 `MemoryExtractor` 只输出受控 Memory Candidate，没有实体证据跨度、关系来源和
   独立审核结果；直接入图会把模型猜测提升成貌似权威的边。
2. Correction、Supersede、Forget、Expiry 与 Erasure 尚无统一的 Mention/Relation
   失效和重建协议，旧边可能绕过当前 Head 语义。
3. 当前固定 Retrieval v2 没有“只有图遍历才能回答”的样本，无法证明图通道相对中文
   BM25/向量召回的增益，也无法量化错误关系注入率。
4. 实体别名是跨记录共享数据，必须先明确 Tenant/Scope 交集、PII 分类和 Erasure
   传播语义。

因此本阶段不创建半成品图表或启用 Graph Recall。重新启动图实现前必须同时满足：

- 版本化 Entity/Relation Candidate 契约，携带 source span、confidence 与提取器版本；
- Rust Policy 独立接受/拒绝，模型输出不能直接成为 active edge；
- 每条边绑定来源 Memory Version，生命周期变化可确定性失效并重建；
- 加入 graph-only 正例、错误关系、别名冲突、跨 Scope、Forget/Erasure 旧边等固定评测；
- 证明四路加入 Graph 后 Recall@K 提升，同时 Stale/Unauthorized/False Relation 注入仍为 0。

这个决策保留了实体图的正确落点：它是可重建 Retrieval Projection，不是新的记忆类型，
也不是绕过 PostgreSQL Head 与授权检查的第二权威库。
