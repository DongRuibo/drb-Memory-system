# Provider 与投影契约

## 能力拆分

业务层依赖三个独立端口，禁止重新合并为 `extract_and_store`：

```rust
#[async_trait]
pub trait MemoryExtractor: Send + Sync {
    async fn extract(
        &self,
        request: ExtractRequest,
    ) -> Result<Vec<CandidateProposal>, ExtractorError>;
}

#[async_trait]
pub trait RetrievalIndex: Send + Sync {
    async fn upsert(
        &self,
        request: ProjectionUpsert,
    ) -> Result<ProjectionReceipt, IndexError>;

    async fn delete(
        &self,
        request: ProjectionDelete,
    ) -> Result<ProjectionReceipt, IndexError>;

    async fn search(
        &self,
        request: ProjectionSearch,
    ) -> Result<Vec<ProjectionHit>, IndexError>;
}

#[async_trait]
pub trait MemoryReranker: Send + Sync {
    async fn rerank(
        &self,
        request: RerankRequest,
    ) -> Result<Vec<RerankedHit>, RerankerError>;
}
```

Extractor 输出是不可信 Candidate Proposal。Retrieval Index 只接受已经提交的 `memory_version_id`，Reranker 不得增加未出现在输入集合中的记录。

## 当前混合检索实现

在线服务当前从两路获取候选：

1. Mem0 当前 generation 的 Provider 命中；
2. PostgreSQL `memory_search.retrieval_documents` 的租户/作用域受限词法命中。

两路结果按 `memory_version_id` 使用 Reciprocal Rank Fusion（`k=60`）合并。随后
必须先从 PostgreSQL 加载当前授权正文，才允许调用 Reranker。Reranker 的输出只能
重排这批候选，并且必须让每个返回 ID 唯一、分数有限且位于 `[0,1]`。服务在外部
Reranker 返回后再次执行 canonical 授权，防止模型调用期间发生 Forget、Erasure
或撤权而泄漏旧正文。

`transparent-lexical-v1` 是确定性本地排序和故障降级路径；
`kimi-k3-semantic-v1` 保留为 Kimi `k3` 兼容路径；`MEMORY_RERANK_MODE=dedicated`
通过独立 bounded HTTP contract 使用 `Qwen/Qwen3-Reranker-0.6B`，避免回答模型兼任
检索排序。语义路径对已授权、限长候选做相关性判断，并将
语义分数与透明特征按固定版本融合。搜索响应和 Context Pack 都记录实际
`ranking_version`；外部 Reranker 失败时返回
`RERANKER_UNAVAILABLE_TRANSPARENT_FALLBACK`，不能伪装成正常语义排序。

这条实现提高检索质量，但不改变信任边界：任何 Reranker 都不是权威数据源，也
不能增加候选、授予 Scope 或绕过 PostgreSQL。

## 正式索引请求

每次 upsert/delete 必须携带：

- `operation_id`：稳定幂等 ID；
- `tenant_id`、`scope_kind`、`scope_id`；
- `memory_record_id`、`memory_version_id`、`head_revision`；
- `generation`；
- `content_hash`；
- upsert 时的允许索引正文和最小 metadata；
- delete 时的 Tombstone revision。

Provider ID 只能保存在 Projection Receipt/Mapping 中，不能写回 Memory Version。

## 幂等与顺序

- 相同 operation ID 重试必须返回等价结果；
- Worker 执行前重新读取 Head/Tombstone；
- operation revision 小于当前 Head revision 时标记 `stale`，不得写入；
- 同一 record 的操作以 ordering key 串行化；
- delete/erasure revision 胜过所有较旧 upsert；
- 超时结果视为 unknown，必须通过 operation ID 查询或安全重试，不能直接创建新 ID。

Mem0 POC 已证明其 `POST /memories` 不满足第一条：相同 `operation_id` 只是相同
metadata，重复请求仍创建不同对象。因此当前 Adapter 使用 PostgreSQL
`projection_operations`、稳定 Provider mapping、lease/fencing、unknown-result
reconciliation 和相同 operation ID 的安全重试提供前置/后置屏障，而不是假设
Mem0 自带幂等。该屏障已有源码和既有 Demo 验证；新 Embedding manifest 下仍需
重新执行真实 unknown-result/重建演练，ADR-0001 在团队评审前保持 `proposed`。

## 检索结果契约

Projection Hit 只返回：

```text
provider, generation, provider_object_id
tenant_id, memory_record_id, memory_version_id
content_hash, provider_score, rank_features
```

正文 MAY 返回用于性能优化，但服务在对外返回前必须从权威 Version 校验 hash、Head、Scope、Consent 和状态。无法映射的命中丢弃并产生 `orphan_projection` 指标。

## POC 必须验证的 Mem0 能力

ADR-0001 转为 accepted 前必须有可复现证据：

1. 候选提取能否与正式持久索引隔离；
2. upsert/delete 是否支持稳定幂等键或可实现安全映射；
3. metadata filter 是否支持 Tenant/Scope 的前置收窄；
4. 删除、更新和服务重启后的行为；
5. 中文偏好、项目事实、精确关键词和冲突场景效果；
6. 导出与全量重建是否只依赖 `memory_core`；
7. 固定版本、OpenAPI/HTTP 契约和升级兼容性；
8. Provider 日志、模型调用和遥测是否会外发敏感正文。

如果第 1、2、6 或 8 项不能满足，Mem0 不得作为正式 Provider 上线。

## Rebuild 与切换

每次重建创建新 generation；校验完成前线上读取继续使用旧 generation。切换必须可原子回退，旧 generation 的保留时间由运行手册定义。Tombstone 在所有 generation 上生效。

每个 generation 还必须固化 index/Embedding provider、model、revision、维度、
归一化、指令版本和 manifest hash。Readiness、在线读写与 Worker 都拒绝运行时
manifest 不匹配。维度或模型发生变化时，不得复用原 Mem0 collection；当前单实例
切换需要维护窗口，无停机双模型索引尚未实现。

## 错误分类

| 类别 | 示例 | 行为 |
|---|---|---|
| transient | timeout、429、5xx | 指数退避、有界重试 |
| permanent | Schema/认证/4xx | 不重试，进入 DLQ/告警 |
| conflict | stale revision、hash mismatch | 丢弃旧操作并触发对账 |
| unknown | 超时后结果未知 | 使用相同 operation ID 查询或重试 |
| security | tenant mismatch、unexpected data egress | 立即停止该 Provider 路径并告警 |
