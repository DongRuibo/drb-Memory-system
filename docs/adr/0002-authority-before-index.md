---
status: accepted
---

# Apply authority before indexing and context assembly

所有 Source Event 必须先经过身份、Scope、敏感数据和写入 Policy 检查；只有 Rust 接受并提交到 Memory Ledger 的 Memory Version 才能进入正式 Retrieval Projection。索引只负责找到候选，读取时仍须对当前权威状态再次授权，因此 Provider 命中永远不等于可读或可注入。

## Consequences

- 禁止使用 `extract_and_store` 作为正式写入原语。
- 被拒绝、待审核、过期、争议、遗忘或已擦除内容不得出现在正式索引。
- 检索后二次授权失败必须 fail closed，并记录不含敏感正文的审计事件。
