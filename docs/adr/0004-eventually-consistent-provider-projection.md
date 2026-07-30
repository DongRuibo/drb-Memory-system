---
status: accepted
---

# Synchronize provider projections with an outbox

PostgreSQL 提交不与外部 Provider 建立分布式事务；Memory Version 被接受后，在同一事务中写入 Outbox，由 Worker 使用稳定的 operation ID 幂等地更新或删除 Retrieval Projection。系统接受短暂的索引滞后，换取权威提交不依赖 Provider 可用性以及可回放恢复能力。

## Consequences

- API 必须区分 `accepted` 与 `projection_ready`。
- Worker 需要租约、退避、死信、重放和 Projection Receipt。
- 强依赖记忆的 Task 可以等待给定版本 Ready；普通 Task 在索引不可用时降级为空或本地精确检索。
