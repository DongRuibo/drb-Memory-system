# Architecture Decision Records

ADR 记录难以逆转、存在真实取舍且仅看代码不容易理解的决策。状态为 `proposed` 的 ADR 必须经过 POC 或评审后才能改为 `accepted`。

- [0001：以 Rust Memory Service 包装 Mem0](./0001-rust-memory-service-with-mem0.md)
- [0002：权威先于索引](./0002-authority-before-index.md)
- [0003：追加式 Ledger 与可重建 Head](./0003-append-only-ledger-and-head.md)
- [0004：Provider 投影采用最终一致性](./0004-eventually-consistent-provider-projection.md)
- [0005：读取采用权限交集](./0005-authority-intersection-for-read.md)
- [0006：统一正式记忆分类](./0006-unified-memory-taxonomy.md)
- [0007：通过 LazyTT 重新认识而非导入外部记忆](./0007-relearn-instead-of-importing-external-memory.md)
- [0008：版本化结构化内容只是权威正文的辅助表示](./0008-versioned-structured-content-is-auxiliary.md)
