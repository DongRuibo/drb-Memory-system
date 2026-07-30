# 一致性、故障和恢复

## 一致性模型

| 对象 | 模型 | 对外保证 |
|---|---|---|
| Memory Ledger/Head | PostgreSQL 强一致事务 | 成功响应后当前 Version/状态立即可读 |
| Provider/FTS 投影 | Outbox 最终一致 | 通过 operation 查询 Ready；读取仍以 Head 为准 |
| Forget/Authority revoke | 权威读取强制失效 | 即使投影陈旧也不能返回 |
| Context Pack | Attempt 内不可变、短 TTL | 失效条件触发后不得复用 |
| Metrics/analytics | 最终一致 | 不参与业务决策 |

## 失败矩阵

| 失败点 | 权威状态 | 客户端结果 | 恢复 |
|---|---|---|---|
| Extractor 超时 | 抽取操作为 pending，无 Memory | retryable error | 相同 Key、请求 hash 和 Extractor 版本有界重试 |
| Extractor 首次完成后响应丢失 | 候选快照已固化，候选可能部分入库 | retryable error | 复用快照和稳定子操作，禁止重新接受另一组模型结果 |
| Candidate 被拒绝 | 无 Memory Version | rejected | 不索引，不重试 |
| DB 事务失败 | 无部分提交 | retryable error | 相同幂等键重试 |
| DB 成功、Provider 失败 | Memory 已接受，projection pending | accepted + pending | Outbox 重试/DLQ |
| Provider 超时结果未知 | Memory 已接受 | pending | 先由 Projection Operation/Mapping 对账；只有证明未创建或 Provider 支持幂等时才能重试 |
| Provider 返回孤儿命中 | 不变 | 丢弃该命中 | 指标、对账、删除 |
| Correct 并发冲突 | 单一 Head 不变或一方成功 | 409 revision conflict | 重新读取后决定 |
| Forget 时 Provider 离线 | Head Forgotten | forget 成功 | 后台删除；权威读取立即阻止 |
| Worker 崩溃 | Outbox leased/pending | 无数据丢失 | lease 到期后重领 |
| Generation 重建失败 | 旧 active 继续服务 | 普通写入暂时冻结 | 幂等 abort 把该 generation 未完成 Operation/Outbox 置为终态；修复后新建 generation |
| Embedding manifest 漂移 | canonical 不变，投影停止 | not ready / projection failed | 恢复匹配运行时或建立新 generation，禁止混写 |
| Canary 验证失败 | 新 canary 服务读取、写入仍冻结 | 管理操作可重试 | 幂等 rollback 恢复 standby |
| 重建期间 Forget/Erasure | canonical 立即 fail closed | 本次重建失效 | 自动恢复旧 active，取消失效 generation 队列，并跨 generation 删除 |
| 备份恢复 | 可能含旧内容 | 服务保持不可读 | 先回放 Ledger/Tombstone 再开放 |

## 重试和背压

- 429 遵循 `Retry-After`；timeout/5xx 指数退避并加 jitter；
- 4xx Schema/Auth 错误不重试，进入 DLQ；
- 每个操作有最大 attempts 和 deadline；
- Worker 使用租约与 fencing revision，租约过期的 Worker 不能提交新 Receipt；
- backlog 超过阈值时限制新低优先级写入，不绕过 Policy；
- Provider 熔断时普通 Context 降级为本地精确检索或空结果。

## 对账任务

周期性对账至少检查：

- Active Head 与 Projection Ready 的数量/hash；
- Provider 孤儿对象；
- Forgotten/Erased/Tombstone 是否仍有投影；
- Outbox stuck、过期 lease 和 DLQ；
- Head 是否可以从 Ledger 重建；
- Context Pack 是否引用了失效 Version。

对账只生成修复 Proposal/Operation，不直接篡改 Ledger。

## 灾难恢复顺序

1. 恢复 PostgreSQL 到选定时间点；
2. 以只读方式验证 Ledger 连续性和 Head hash；
3. 回放恢复点之后单独保存的 Tombstone/Erasure 记录；
4. 使所有 Context Pack 失效；
5. 从 Head 重建新的检索 generation；
6. 完成 Tenant 抽样与隔离测试；
7. 先开放只读，再开放写入；
8. 记录实际 RPO/RTO 和差异。

## 禁止做法

- 通过查询 Mem0 修复 Memory Ledger；
- 为追赶 backlog 跳过 Tombstone/Head 校验；
- 对 unknown provider result 创建新的 operation ID；
- 在 Provider 不支持 operation lookup/idempotency 时盲目重试 unknown mutation；
- 在备份恢复后直接开放旧索引；
- 把“Provider HTTP 200”视为投影内容已正确且已授权。
- 在 generation 仍为 building/ready/canary 时绕过写冻结；
- 在未校验 canonical 数量、revision 和 hash 时直接切换 generation。
