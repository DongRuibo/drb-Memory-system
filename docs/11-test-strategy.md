# 测试策略

## 测试层级

### 单元测试

- typed Scope 和 Authority intersection；
- Candidate/Head 状态机；
- Policy reason code；
- normalized key 只能作为信号；
- token budget 与安全引用格式；
- Provider 错误分类；
- Embedding model/dimension/norm manifest；
- Extractor Proposal schema、主操作快照和候选子操作幂等；
- Reranker index 全覆盖、唯一性、分数边界和阈值；
- Tombstone 和 revision 比较。

### 属性与状态机测试

- 任意操作序列后最多一个 Head；
- revision 单调递增；
- Forgotten/Erased 永远不可再次注入；
- 只重放 Ledger 可得到相同 Head；
- 任意租户输入组合都不能产生跨租户关联。

### 数据库集成测试

使用真实 PostgreSQL：

- RLS、复合外键和唯一约束；
- Accept/Correct/Forget 原子事务；
- 两个并发 revision 更新单赢家；
- Outbox lease、崩溃重领、DLQ；
- Migration 从上一发布版本升级；
- Head rebuild/hash 对账。
- Generation 重建写冻结、ready 计数、切换、幂等重放、rollback 和 finalize；
- Extraction snapshot 的零候选、并发首完成者、请求指纹冲突和 partial-ingest 重放；

### Provider 契约测试

同一套测试运行于 Mem0 Adapter 和 fake Adapter：

- extract 不产生正式索引；
- operation ID 幂等；
- delete 胜过旧 upsert；
- metadata filter 与 canonical second check；
- timeout unknown result；
- generation rebuild/switch；
- 固定 OpenAPI/版本兼容性。

### 端到端测试

```text
Source Event / Automatic Extraction → Candidate → Decision → Ledger
→ Projection → Search → Context Pack → Receipt
→ Correct/Forget/Erasure
```

必须覆盖 Provider、Worker、PostgreSQL 重启以及恢复过程。

真实语义 profile 还必须固定模型 revision，比较无向量/POC/真实 Embedding/独立
Reranker 的 Recall、MRR、负查询命中、Forget 零注入、P95 和内存；只跑 HTTP 200
不算模型验收。

### 安全测试

- Tenant/User/Project/Agent 访问矩阵；
- 伪造 Scope、过期身份、成员移除；
- Secret、PII、Prompt Injection corpus；
- 管理/审核/导出/擦除权限；
- 日志和 Trace 正文泄漏扫描。

## 测试可追溯性

每个 [领域不变量](./01-domain-model-and-invariants.md) 必须至少对应：

- 一个正向测试；
- 一个失败/对抗测试；
- 对并发或恢复相关不变量的集成测试。

CI 输出 invariant ID 到 test name 的映射。缺少映射视为未完成，而不是“后续补测试”。

## 测试环境

- 单元测试可使用 fake ports；
- 数据库语义不得用 SQLite 替代 PostgreSQL；
- Provider 契约测试固定容器镜像 digest；
- 时间、UUID 和随机性可注入；
- 测试数据使用合成内容，不放真实用户数据。

## CI Gate

- 格式、lint、单元、属性测试；
- Migration apply + schema contract；
- PostgreSQL 集成测试；
- OpenAPI/AsyncAPI/YAML 校验；
- Adapter 契约测试；
- 安全负例和固定回归评测；
- 依赖许可、漏洞和 secret scan。
