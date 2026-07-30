---
status: accepted
---

# Use an append-only Memory Ledger and a rebuildable Memory Head

Memory Version 和 Memory Event 采用追加写，修正、争议、过期和遗忘不覆盖历史内容；每个 Memory Record 只有一个 Memory Head 作为当前读取投影。相比直接更新一行记录，这增加了表和事务复杂度，但能够提供可追溯版本、并发单赢家、确定性恢复和投影重建能力。

## Consequences

- 当前状态由 Ledger 归约得到，Head 可以重建。
- 一次状态变化使用 `Lock → Guard → Decide → Append → Reduce → Outbox → Commit` 短事务。
- 模型调用、Provider 调用、对象存储和消息发送不得位于该数据库事务内。
