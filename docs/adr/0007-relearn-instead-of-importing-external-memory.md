---
status: accepted
---

# Relearn through LazyTT instead of importing external memory

LazyTT 只学习另一记忆项目的分类、时间、结构化内容和检索机制，不读取或迁移其历史 Memory、History、Embedding、偏好或用户画像。新的长期记忆必须来自 LazyTT 后续真实交互记录的 Source Event，并继续经过 Candidate、Policy、Ledger 和 Projection 链路。相比导入旧数据，这会失去冷启动连续性，但避免继承来源、Consent、Scope 和正确性无法由 LazyTT 独立证明的历史结论，也保持 PostgreSQL Ledger 是唯一权威入口。

## Consequences

- 不建设跨项目 Importer、Import Staging 或历史数据映射工作台。
- 钟佳栋项目中的类型只作为统一分类的语义对照。
- 时间、结构化字段和多路召回能力只作用于 LazyTT 原生新记忆。
- 用户和项目信息通过未来真实交互重新认识，不以历史数据回填缩短冷启动。
