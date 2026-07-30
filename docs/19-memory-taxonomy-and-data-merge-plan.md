# 记忆分类与跨项目机制吸收方案

状态：Taxonomy、Event Time 和版本化 Structured Content 已于 2026-07-30 实施并通过空库
迁移与集成测试。项目明确不迁移另一系统的历史记忆，用户与项目知识由 LazyTT 在后续真实
交互中重新认识和生成。本文只吸收“语义、情景、偏好、画像”的分类设计，以及时间、
结构化内容和多路召回机制，不建设跨项目数据导入链路。

关联决策：[ADR-0006](./adr/0006-unified-memory-taxonomy.md)、
[ADR-0007](./adr/0007-relearn-instead-of-importing-external-memory.md)、
[ADR-0008](./adr/0008-versioned-structured-content-is-auxiliary.md)。

## 1. 合并结论

两个项目共享一套正式记忆治理分类：

```text
preference
profile_fact
semantic_fact
episodic_summary
```

这四个值用于稳定 API、Policy、Retention 和 Prompt Effect，不表示四个同级的认知类别。
它们向上归入两个 `Memory Family`：

```text
semantic -> preference | profile_fact | semantic_fact
episodic -> episodic_summary
```

Working Memory 属于单轮 Runtime State；Procedure/Skill 属于具有独立评测、审批和发布门槛的
能力系统。二者都不进入正式 Memory Ledger。

分类与作用域是两个正交维度：

```text
Memory Type：这条记忆是什么、允许怎样影响回答
Memory Scope：谁在什么 Tenant/User/Project/Agent 边界内可以访问
Semantic Key：这条记忆描述的是哪个稳定命题
Memory Version：这个命题在某个时间点被接受的内容
```

因此，当前 LazyTT 的 `project_fact` 合并为 `semantic_fact`，原有 typed Scope 原位保留并重新
经过授权校验；不能从旧类型名反向推导它一定属于 Project Scope。向量、FTS/BM25、图和
时间是 Retrieval Projection 或召回通道，不是 Memory Type。

## 2. 目标与非目标

### 2.1 目标

- 为 LazyTT 建立一套稳定、可版本化的正式 Memory Type 协议；
- 保留 LazyTT 的 Authority、Ledger、Version、Head、Outbox、Context Pack 和 Forget/Erase 不变量；
- 吸收另一方案的时间、结构化内容、中文语义、实体关系和多路召回能力，但只把它们作为
  LazyTT 原生 Source、Candidate 特征或可重建投影；
- 让 LazyTT 只通过后续真实交互产生新记忆，不继承另一系统的用户画像、偏好和历史结论；
- 为 LazyTT 旧客户端提供有限期分类兼容路径，并能够安全停止旧类型兼容。

### 2.2 非目标

- 不合并两个代码仓库的目录、Git 历史或部署脚本；
- 不读取、暂存或导入另一系统的历史 Memory Record、History、Embedding 或用户数据；
- 不把聊天记录、Provider/向量索引或管理后台数据直接提升为正式记忆；
- 不在本方案中引入完整 GraphRAG、Reflection 或自动 Skill 发布；
- 不用记忆替代业务 Canonical State、身份、权限、Approval 或工具执行权。

## 3. 统一领域模型

### 3.1 两个认知族、四个治理类型

| Memory Family | Canonical Type | 定义 | 允许的主要影响 | 典型例子 | 不能包含 |
|---|---|---|---|---|---|
| `semantic` | `preference` | 主体明确表达、可长期复用的回答或工作方式偏好 | 回答风格、格式、非权限型协作方式 | “技术问题先讲结论” | 权限、安全规则、项目强制约束 |
| `semantic` | `profile_fact` | 关于用户身份、背景或稳定特征的来源支持事实 | 背景个性化 | “用户是 Agent 研发工程师” | 登录身份、成员关系、授权结论 |
| `semantic` | `semantic_fact` | 关于项目、领域或环境的来源支持稳定命题 | 当前事实个性化或背景补充 | “该项目以 PostgreSQL 为权威数据库” | 未验证猜测、向量相似结果、业务权威状态副本 |
| `episodic` | `episodic_summary` | 从明确发生的经历中提炼出的简洁、可复用经验 | 相关历史参考 | “曾用 event_id 解决 Outbox 重复消费” | 原始聊天、流水日志、未经验证的反思 |

`profile_fact` 在认知分类上也是事实，但单独保留是有意的：个人信息需要比普通 `semantic_fact` 更严格的 Consent、Retention、显示和 Prompt Effect 策略。

`preference` 同样属于 Semantic Family，但它影响表达方式，而普通事实只允许做回答依据；
因此不应为了追求纯认知分类而把三者压成一个无法实施差异化治理的 `semantic` 枚举。

### 3.2 Scope 约束

`scope` 是访问边界，不是记忆类别，也不天然等于事实主体。

| Memory Type | 默认允许 Scope | 受控扩展 |
|---|---|---|
| `preference` | `user`、`agent` | 项目规范应优先归为 `semantic_fact`；跨 Scope 复制必须有独立来源和 Policy Decision |
| `profile_fact` | `user` | 向 Project/Tenant 暴露必须基于明确 Consent 创建独立记录，不移动原记录 |
| `semantic_fact` | `tenant`、`user`、`project`、`agent` | 影响真实操作前仍须回查 Canonical State |
| `episodic_summary` | `user`、`project`、`agent` | `tenant` 共享必须人工审核并去除个人/项目敏感信息 |

服务端根据 Principal、Membership、Task Context 和 Policy 决定最终 Scope；Extractor 和
客户端只能请求收窄，不能自行扩大 Scope。

### 3.3 正式身份

合并后的 live semantic identity 保持为：

```text
(tenant_id, scope_kind, scope_id, memory_type, semantic_key)
```

- 类型映射后必须重新执行碰撞检查；
- 相同 identity + 相同规范化内容只合并来源；
- 相同 identity + 不同内容进入 `NeedsReview/Contested`；
- 不得仅凭更新时间、Embedding 分数或 Provider ID 自动覆盖当前 Head。

## 4. 两个项目的类型语义对照

| 来源 | 来源类型 | Canonical Type | Scope 规则 | 说明 |
|---|---|---|---|---|
| LazyTT | `preference` | `preference` | 保持原 Scope并复核矩阵 | 直接映射 |
| LazyTT | `profile_fact` | `profile_fact` | 默认 `user` | 直接映射 |
| LazyTT | `project_fact` | `semantic_fact` | 原 Scope 原位保留并复核授权 | 类型改名，Record ID 和版本链保持；历史数据并非全部是 Project Scope |
| LazyTT | `episodic_summary` | `episodic_summary` | 保持原 Scope并复核矩阵 | 直接映射 |
| 另一项目 | 偏好 | `preference` | 由已验证主体派生 | 不接受自由文本 Scope |
| 另一项目 | 画像 | `profile_fact` | 默认 `user` | 第三方画像默认拒绝或审核 |
| 另一项目 | 语义 | `semantic_fact` | 按来源所属 Tenant/User/Project/Agent 派生 | 不能把“向量命中”当成事实来源 |
| 另一项目 | 情景 | `episodic_summary` | 按经历所属主体/项目派生 | 原始事件先形成 Source Event，再提炼摘要 |

这张表只用于学习和校准 LazyTT 的分类语义，不代表建立跨项目 Adapter，也不授权读取或
迁移另一项目的真实数据。LazyTT 的新记忆仍必须来自自身运行期间记录的 Source Event。

### 4.1 `procedural_candidate` 的处理

`procedural_candidate` 不映射到四类正式记忆：

- `pending/needs_review`：以 `TYPE_MOVED_TO_PROCEDURE_PIPELINE` 拒绝或导出到独立人工审核队列；
- LazyTT 历史上已经形成正式 Record：禁止自动重分类，逐条决定转为
  `episodic_summary`、`semantic_fact`，或迁移到未来 Skill/Procedure 子系统；
- 仍有未处理的正式 `procedural_candidate` Record 时，不允许执行最终约束收紧；
- 无论迁移结果如何，都不得自动注册 Tool、Skill 或获得执行权限。

### 4.2 为什么不增加 Working、Graph 或 Temporal 类型

- Working/Conversation/Session Memory 的生命周期属于 Runtime/Conversation Context，不是跨
  Attempt 的正式长期记忆；
- Graph、Vector、FTS/BM25 和 Temporal 表达的是同一条记忆的结构或检索通道，不改变其
  内容语义；
- 用户反馈、模型反思和工具结果首先是 Source Event，只有经过提取和治理后才成为上述某个
  Canonical Type；
- 项目规则或组织 Policy 必须由 Canonical State/只读配置提供，记忆只能引用，不能获得
  Policy 或执行权。

## 5. LazyTT 原生写入链路

```text
LazyTT Conversation / Tool Result / Task Result / User Correction
  -> Source Event（保留 occurred_at、来源引用和内容 hash）
  -> 时间与结构化信息提取
  -> Taxonomy v2 Normalizer
  -> Memory Candidate
  -> Secret / Scope / Consent / Policy / Confidence Guard
  -> Exact Dedup or Conflict Decision
  -> PostgreSQL Record + Immutable Version + Ledger Event + Head
  -> Projection Operation + Outbox
  -> Mem0 / pgvector / FTS-BM25 / Graph / Time Projection
```

必须坚持单一权威入口：DeepSeek/Kimi、Mem0、Embedding、Reranker 和图/时间检索都只能
提出 Candidate 或维护投影，不能绕过 Policy 直接写 Head。

### 5.1 时间机制

LazyTT 已具备 `SourceEvent.occurred_at`、`MemoryVersion.effective_from/effective_to`、
中文时间查询解析和 Temporal Retrieval，并已把另一方案中有价值的 Event Time 契约接入
原生提取、Candidate、Version、Inspect/Export 和时间候选通道：

- 区分“事件什么时候发生”“记忆什么时候被记录”“命题什么时候有效”；
- 仅对 `episodic_summary` 支持事件开始、结束、IANA 时区和受控时间精度；
- 使用 UTC 半开区间存储，保留解释原始表达所用时区和精度，不伪造到秒级；
- 没有来源证据时 Event Time 为空，不拿 `SourceEvent.occurred_at` 冒充历史事件时间；
- 检索时使用结构化时间窗作为候选通道，最终仍回表校验 Head、Scope 和生命周期；
- 时间变化不能静默覆盖旧 Version；当前 Correction 保留原 Event Time，后续增加显式
  Event Time 修正契约后才能变更或清空。

### 5.2 结构化内容机制

正文仍是可读、可擦除、可版本化的 canonical content；结构化内容是经过 Schema 校验的
辅助表示，而不是第二份权威记忆。当前 v1 已实现：

- `schema_name`、`schema_version` 和受限 JSON payload；
- preference 的 subject/value/qualifier；
- profile fact 的 subject/attribute/value；
- semantic fact 的 subject/predicate/object；
- episode 的 event kind、participants 和 outcome；Event Time 继续使用独立字段；
- 所有字段经过大小、深度、键白名单、PII/Secret 和 Prompt Injection 检查；
- Correction 不复制可能过期的旧 payload，Forget 阻断读取，Erase 同步清空；
- Embedding、图边和时间索引只能从已接受 Version 重建。

## 6. API 兼容策略（已实施）

分类枚举变化采用“Canonical 单写、旧别名只在请求边界适配”，禁止为同一记忆双写两条
权威 Record。当前 API 尚未 GA，因此沿用 `/v1` 路径并直接返回统一分类；OpenAPI 是当前
契约来源。

### 6.1 Canonical 契约

- 请求和响应只使用四个 Canonical Type；
- `semantic_fact` 必须与服务端授权的 typed Scope 一起出现；
- Context Pack 继续使用 `user_preference/current_fact/relevant_experience` section：
  - `preference -> user_preference`
  - `profile_fact | semantic_fact -> current_fact`
  - `episodic_summary -> relevant_experience`

### 6.2 `project_fact` 兼容层

- Search/Context 请求仍可提交 `project_fact`，但必须同时给出显式且全部为 Project 的 Scope
  Selector；边界将其规范化为 `semantic_fact`，缺失、混合 Scope 或同时提交 pinned
  Memory Version 时 fail closed；
- 该别名是安全的迁移垫片，不是对旧查询语义的完整仿真：历史 Tenant Scope 的
  `project_fact` 已规范化为 Tenant Scope `semantic_fact`，调用方必须改用 Canonical Type；
- 旧 Extraction Snapshot 中的 `project_fact` 在回放时规范化为 `semantic_fact`，但候选
  Idempotency-Key 继续绑定原始类型，避免重放创建第二条正式记忆；
- Extractor、Inspect、Export、Search、Context Pack 和 Web UI 响应只返回 Canonical Type；
- `semantic`、`episodic`、`profile` 只用于跨项目设计语义对照，不作为 HTTP API 输入；
  `procedural` 进入独立候选处置，不规范化为正式 Memory Type。

兼容层只存在于 HTTP/Extraction 回放边界，不进入 PostgreSQL Canonical Type、Ledger 新事件
或 Provider metadata。

## 7. PostgreSQL 迁移方案

### 7.1 迁移前审计

先按 Tenant 导出只读报告：

```text
count by memory_type / scope_kind / head_state
project_fact -> semantic_fact 后的 identity collision
procedural_candidate Record/Candidate 数量和状态
缺失或非法 semantic_key / scope / source / consent
未完成 Projection Operation / Outbox / Erasure Request
```

任何 identity collision、活跃 Erasure 或无法解释的 Procedure Record 都是迁移阻断项。

### 7.2 Expand（Taxonomy 已完成）

使用追加式 `0022_unified_memory_taxonomy.sql`，没有修改历史 migration：

1. 迁移前阻断正式/已接受 `procedural_candidate` 和 live identity 碰撞；
2. 使用 `memory_type_reclassifications` 保存对象 ID、原类型、目标类型、taxonomy version 和
   稳定 reason code，不复制正文；
3. 将 Candidate/Record 的 `project_fact` 原位规范化为 `semantic_fact`，保持 Record、Version、
   Head revision 和 Source 不变；
4. 正式 Record 约束收紧为四个 Canonical Type；Candidate 仅允许未决/已拒绝的
   `procedural_candidate`，禁止它成为正式 Record；
5. Repository、Extractor、Client、API、Reranker、Context Pack 和 Web UI 只写/返回
   Canonical Type。

### 7.3 Backfill（本地权威数据已完成）

当前仓库的追加 migration 在一个事务中锁定类型表、执行全量碰撞预检查、写入
`memory_type_reclassifications`，再原位更新类型。因为现有 Provider metadata 不保存
`memory_type`，Mem0 对象无需因改名重写；BM25/授权读取实时回表读取 Canonical Record。

本地迁移实测将 64 个 Candidate 和 49 个 Record 从 `project_fact` 改为
`semantic_fact`，碰撞为 0；其中 Candidate 为 Project/Tenant Scope 47/17，Record 为
Project/Tenant Scope 32/17。Migration、审计行数和迁移后类型分布均已核对。迁移保留
Scope，不更新 Memory Version、Head revision、Source、正文或内容 hash。

发生碰撞时不得直接 `UPDATE`：

- 内容 hash 相同：选择一个 live Record，合并来源，另一个 Record 走可审计的身份退休流程；
- 内容不同：两个命题进入 Contested/Review，由授权 Reviewer 决定 Correct、Reject 或保持分离并修正 Semantic Key。

### 7.4 Contract（部分完成）

满足全部验收门槛并经过观察期后：

- [x] 禁止新写 `project_fact`；
- [x] 正式 `memory_records.memory_type` 约束只保留四个 Canonical Type；
- 历史原始类型保留在重分类审计中，不篡改来源证据；
- 下线旧 `project_fact` 别名与兼容指标；
- [x] ADR-0006 经实现和迁移验证后从 `proposed` 改为 `accepted`。

## 8. 检索与 Context Pack 合并

另一项目的四路召回与 LazyTT 当前检索链可以并存：

| 召回通道 | 合并后角色 | 是否权威 |
|---|---|---:|
| 向量 / Mem0 / pgvector | 语义相似候选 | 否 |
| FTS / 中文 BM25 | 名称、关键词、版本号候选 | 否 |
| 实体关系 / 两跳图 | 与人物、项目、实体相关的候选 | 否 |
| 时间检索 | 与查询时间区间匹配的候选 | 否 |

四路结果可使用版本化 RRF 和 Reranker 融合，但融合前后都必须回 PostgreSQL 校验当前 Head、Version、Scope、Purpose、Consent、Retention、Tombstone、revision 和 content hash。最终只有 Context Compiler 能按预算生成 Attempt-bound Context Pack。

## 9. 分阶段交付

### Phase 0：统一分类（已完成）

- 固化四个 Canonical Type、两个 Memory Family 和 Scope Matrix；
- 完成 `project_fact -> semantic_fact` 迁移、碰撞预检和重分类审计；
- Extractor、API、Repository、Context Pack 和 UI 统一使用 Canonical Type。

退出条件：新写入不再产生 `project_fact`，正式 Record 只保留四类。

### Phase 1：时间语义基线（已完成）

- 分离 Source Occurrence Time、Event Time、Recorded Time 和 Effective Time；
- 保持中文显式日期和相对时间窗解析；
- 增加事件结束时间、原始时区、时间精度和不确定性契约；
- Extractor 仅在来源明确支持时为 `episodic_summary` 产生 Event Time；
- Candidate/Version 持久化，Inspect/Export 返回；Erase 同步清空；
- 时间检索优先使用 Event Time 区间，无 Event Time 时兼容旧数据并回退 Source
  Occurrence Time；
- 已覆盖 IANA 时区、半开区间、非法区间、消息时间与事件时间分离、Event Time 检索与
  Forget 后授权阻断。

2026-07-30 已在隔离空库执行 `0001` 至 `0024` 全量 migration，16 个 Memory Repository
PostgreSQL 集成测试全部通过。现有开发 `memory` 库没有被修改。Context Pack 目前只携带
canonical content，不单独暴露 Event Time；显式时间元数据进入 Context 属于 Phase 4。

退出条件：写入、查询、Inspect 和 Export 对同一时间语义解释一致，且干净 PostgreSQL
通过 migration 与集成测试。

### Phase 2：结构化内容契约（已完成）

- 已为四类记忆分别定义 `lazytt.*` v1 JSON Schema；
- v1 只允许白名单字段、最多 4096 bytes、最多 8 个 participants，文本字段不超过
  256 bytes；
- 模型返回先作为非可信 JSON，再由确定性边界解析；结构失败时保留普通正文；
- Repository 再次执行 Schema 与安全校验，禁止结构携带 Secret、Prompt Injection、
  Policy、Permission 或执行权；
- 当前只在 Inspect/Export 展示，尚不单独进入 Prompt 或作为检索授权条件。

退出条件：未知 Schema、未知字段、超限和不安全内容全部 fail closed。

### Phase 3：原生持久化与生命周期（已完成）

- 结构化 payload 绑定到 Candidate，Accept 后复制到不可变 Memory Version，而不是
  Record 或 Provider；
- Correction 追加新 Version，并把结构清空，避免复制与新正文不一致的旧 payload；
- Forget 继续由 Head 阻断读取；Erase、Inspect 和 Export 已覆盖结构化字段；
- 只对 LazyTT 新产生的记忆启用，不回填另一项目数据。

退出条件：无双重权威、无越权字段泄漏、擦除后无法从投影恢复 payload。

### Phase 4：投影、Context 与评测（计划）

- 从已接受 Version 生成时间、实体关系和检索字段；
- RRF/Reranker 融合后继续执行 canonical authorization；
- Context Compiler 按类型和预算选择正文或安全结构化摘要；
- 评测 Recall、Precision、时间命中、跨语言、Prompt Injection 和 Forget/Erase 零复活。

退出条件：质量指标达到门槛，且关闭任一投影都不影响 PostgreSQL 权威状态。

## 10. 回滚策略

迁移采用预检查、审计、Backfill、Contract；不以无审计的反向 `UPDATE` 作为回滚：

- 本地 migration 前保存受限权限的 PostgreSQL 压缩备份；
- 应用可临时继续使用要求显式 Project Scope 的 `project_fact` 请求别名；
- 已追加的 Version、Ledger Event 和来源关系不删除、不覆盖；
- 投影异常时停止 Worker generation、切回上一 active generation，并从 PostgreSQL 重建；
- 发现越权、Forget 复活或 hash 不一致时立即停止批次，Authority 读路径 fail closed；
- 只有完成备份恢复演练和业务签字后才能执行不可逆的 Contract/Erase 操作。

## 11. 可观测性

至少增加以下指标和审计字段：

- `memory_type_alias_total{source_type,client}`；
- `memory_taxonomy_collision_total{tenant,outcome}`；
- `memory_taxonomy_unresolved_procedure_total`；
- `memory_structured_extraction_total{schema_name,outcome}`；
- `memory_temporal_precision_total{precision,outcome}`；
- `memory_projection_type_mismatch_total{provider,generation}`；
- Source/Candidate/Record/Version/Head/Projection 对账结果；
- 每次自动或人工重分类的 actor、reason、旧类型、新类型和关联 ID。

指标标签不得包含原始正文、用户 ID、Semantic Key 或其他高基数字段。

## 12. 验收清单

### 12.1 分类与数据

- [x] 正式 Record 只包含四个 Canonical Type；
- [x] `project_fact` 全部转换为保留原 typed Scope 的 `semantic_fact`；
- [ ] 所有 Procedure 有拒绝、人工重分类或进入未来 Procedure/Skill Pipeline 的结果；
- [x] 明确不导入另一项目的历史 Memory、History 或用户画像；
- [x] LazyTT 原生重复 Source 不新增 Record/Version/Event；
- [x] 精确重复只增加来源，原生冲突内容不会静默覆盖；
- [x] 类型迁移不改写 Version、Head、Source、Ledger 或内容 hash，Candidate/Record 与审计
  行数对账通过。

### 12.2 安全与生命周期

- [x] Tenant/User/Project/Agent Scope 交叉访问测试全部 fail closed；
- [ ] Profile 跨 Scope 必须有 Consent 和独立 Decision；
- [x] Forget 在投影离线时仍立即阻断读取；
- [x] Erased 正文、结构化字段和来源摘录不因 Outbox 回放或投影重建复活；
- [x] 记忆内容不能改变 System Prompt、ACL、工具参数或执行授权。

### 12.3 API、检索与 Context

- [x] Canonical Type 与 `project_fact` 受限别名通过契约测试；
- [x] Extractor、Search、Context Pack、Inspect、Export 和 Web UI 使用同一分类；
- [x] `semantic_fact` 与语义检索在代码和当前规范文档中明确区分；指标命名待统一
  Metrics 接入；
- [x] 每个 Retrieval Hit 在 Rerank 前后均完成 canonical authorization；
- [x] Context Pack section 映射、预算和 allowed effect 保持稳定；
- [ ] 黄金查询的 Recall、Precision、跨语言和时间检索达到评测门槛。

## 13. 实施改动面

分类合并已经完成。后续不建设 Importer，实施顺序调整为：

1. 时间：补齐事件结束、时区、精度和不确定性，保持现有时间检索链；
2. 结构：定义四类记忆的版本化 JSON Schema 和安全边界；
3. 数据库：将结构化 payload 绑定到不可变 Version，补齐 Forget/Erase/Export；
4. Extractor：同时生成可读正文和受限结构化字段，失败时安全降级为正文；
5. Retrieval：从正式 Version 派生时间和实体投影，继续执行授权回表；
6. Context：按类型、Purpose 和预算选择安全字段，不让结构化内容获得工具权限；
7. Runtime/UI：展示时间精度和结构化字段来源，支持 Inspect；
8. 验证：SQL 集成、HTTP E2E、安全回归、时间与结构化黄金查询评测。

建议先完成时间契约，再实现结构化 Schema，最后接入投影与 Context。任何阶段都只处理
LazyTT 原生产生的新 Source/Candidate，不能绕过 Authority-first 原则。
