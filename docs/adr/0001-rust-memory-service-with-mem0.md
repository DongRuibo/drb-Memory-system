---
status: proposed
---

# Use a Rust Memory Service in front of Mem0

LazyTT 自研 Agent Runtime 已经拥有 Task、Attempt、Policy、工具和 Context 控制，因此第一阶段由 Rust Memory Service 持有领域模型和治理能力，Mem0 OSS 只通过 Adapter 提供候选提取、向量检索或重排能力。相比整体采用 Letta 或在第一阶段采用更重的长期记忆引擎，这一选择减少 Runtime 重叠和迁移锁定；只有在 POC 证明 Mem0 能满足隔离、删除、中文效果和投影协议后，本 ADR 才能转为 `accepted`。

## Consequences

- 业务层不得依赖 Mem0 ID、请求结构或数据库表。
- Mem0 不能成为正式状态或权限来源。
- 如果 Mem0 不能把候选提取与正式索引隔离，Adapter 必须使用可清理的 staging 区，或者更换 Extractor。
