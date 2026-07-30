# 数据模型与事务约束

本文件是 Migration 的设计基线。实际 SQL 必须在集成测试中由 PostgreSQL 验证。

## Schema 所有权

```text
memory_core    权威 Ledger、Policy 引用、Outbox 和审计
memory_search  自有可重建 FTS/精确检索投影
mem0           Mem0 自管数据，业务代码禁止直接读取
```

生产环境 SHOULD 为 `memory_core` 与 `mem0` 使用不同数据库角色；是否使用同一 PostgreSQL 实例由容量和故障域评审决定。

## 核心表

### `source_events`

| 字段 | 约束 |
|---|---|
| `id`, `tenant_id` | 主标识；所有关联携带 tenant |
| `source_type`, `source_ref` | 受控枚举与外部稳定引用 |
| `client_event_id` | API 事件标识；`UNIQUE(tenant_id, client_event_id)` |
| `content_hash` | 对规范化输入计算 |
| `redacted_excerpt` | 可空；不得保存 Secret |
| `occurred_at`, `recorded_at` | 事实发生时间与系统记录时间 |
| `classification`, `consent_ref` | Policy 输入 |
| `idempotency_key` | `UNIQUE(tenant_id, idempotency_key)` |

### `extraction_operations`

保存自动提取主请求的 Tenant 级 Idempotency-Key、完整 request hash、Extractor 版本、
`pending/completed` 状态和首个完整 Proposal JSON 快照。`completed` 必须同时具有数组
快照和完成时间，数组可以为空；`pending` 不得含部分快照。它解决模型重试非确定性，
不替代每个 Candidate 的正式 Policy/Decision/Ledger 事务。

### `memory_candidates` 与 `memory_decisions`

Candidate 保存 typed scope、memory type、候选正文、normalized key、置信度、推断类型、Extractor 版本和创建时 Policy 版本。Decision 保存 outcome、reason code、actor、决策时 Policy 版本和时间；候选准入 Decision 与 Candidate 一对一，用户修正等直接治理操作也必须创建独立 Decision，因此 `candidate_id` 可以为空。

Candidate 与 Source 通过 `candidate_sources` 多对多关联，所有复合外键包含 `tenant_id`。

### `memory_records`

只保存逻辑身份和稳定归属：

```text
id, tenant_id, scope_kind, scope_id, memory_type
semantic_key, identity_retired_at, created_at
```

约束：

- `scope_kind = tenant` 时 `scope_id IS NULL`；
- 其他 scope kind 时 `scope_id IS NOT NULL`；
- live identity 在 `(tenant, scope, memory_type, semantic_key)` 上唯一；
- Forget/Erasure 退休 identity 后，未来显式写入可以建立新的 Record；
- 不保存 Provider ID、当前正文或可变状态。

### `memory_ingest_results`

保存一次 Source/Candidate 最终映射到的 Record、Version、Projection Operation 和结果
类型。它让重复来源附加在现有 Version 后，原请求和重放请求仍能解析为同一个
canonical operation，而不依赖 Provider 的幂等能力。

### `memory_review_results`

保存 Review API 的 Tenant 级幂等键、完整请求指纹、Candidate、最终 outcome、
Record/Version 和 Projection Operation。Candidate 使用自身 revision 保证最终
Decision 单赢家；`memory_decisions` 保留初始 NeedsReview 和最终 Accept/Reject
两个追加事实。

### `memory_versions`

每个 Version 不可变，至少包含：

```text
id, tenant_id, memory_record_id, version_no
content, content_hash, normalized_key
confidence, importance, inference_kind
effective_from, effective_to
classification, allowed_purposes
consent_ref, retention_policy_id, retain_until
decision_id, created_at
```

约束：

- `UNIQUE(tenant_id, memory_record_id, version_no)`；
- `effective_to IS NULL OR effective_to > effective_from`；
- accepted Version 必须关联最终 Accept Decision 和至少一个 Source；
- 数据库角色禁止 UPDATE/DELETE；Erasure 由专用受审计过程清除受保护正文。

### `memory_events`

追加式领域事件：

```text
id, tenant_id, memory_record_id, sequence_no
event_type, version_id, actor_type, actor_id
reason_code, payload, occurred_at
```

`UNIQUE(tenant_id, memory_record_id, sequence_no)` 保证同一聚合内顺序。`payload` 不得复制完整敏感正文。

### `memory_heads`

当前读取投影：

```text
tenant_id, memory_record_id, current_version_id
state, revision, updated_at
```

约束：

- `PRIMARY KEY(tenant_id, memory_record_id)`；
- `revision` 单调递增；
- 更新语句必须带 `WHERE revision = expected_revision`；
- `Forgotten/Erased` 不允许 current Version 被读取或索引；
- Head 必须能够从 Version/Event 重建并进行 hash 对账。

### `memory_version_sources`

复合主键为 `(tenant_id, memory_version_id, source_event_id)`，两侧都使用含 tenant 的复合外键，防止跨 Tenant 关联。

### `tombstones` 与 `erasure_requests`

Tombstone 保存 record ID、tenant、最低禁止 generation、reason、创建时间，不保存记忆正文。Erasure Request 保存状态、目标范围、各处理器进度、deadline 和完成时间。

### `projection_operations` 与 `projection_receipts`

Provider 映射从 Memory Version 分离：

```text
operation_id, tenant_id, version_id, generation
provider, operation_type, idempotency_key
status, lease_owner, lease_until, lease_version, attempts, available_at
provider_object_id, provider_revision, content_hash
last_error_code, created_at, completed_at
```

同一 Version 可以有多个 Provider/generation 映射。`idempotency_key` 必须稳定编码 provider、generation、operation type、record/version 和 Head revision，并由 `UNIQUE(tenant_id, provider, idempotency_key)` 约束。Provider timeout 使用 `unknown` 状态；带 `lease_version` 的完成写入用于 fencing 过期 Worker。

### `projection_generations` 与 `projection_generation_transitions`

每个 Tenant 最多有一个 `active`、一个处于 `building/ready/canary` 的 generation。
Generation 保存来源 generation、预期/ready/failed 数量、完成/激活/退役时间和
稳定 rebuild idempotency key。状态机为：

```text
building → ready → canary → active
                  ↘ rollback → retired
building/ready → failed
active → standby → retired
```

`projection_generation_transitions` 以 Tenant + `Idempotency-Key` 保存
activate/finalize/rollback/abort 的请求 hash 和响应快照，使客户端在未知结果后可以
安全重试。Context Pack 同时保存 `ranking_version` 与 `projection_generation`，
generation 切换后不会错误复用旧 Pack。

### `context_packs`、`context_pack_items`、`usage_receipts`

Context Pack 保存 Attempt、Authority/Policy/Compiler/Ranking/Projection generation、
query hash、tokenizer、预算、创建和过期时间。Item 引用不可变 Version，保存顺序、
分区、引用、内容快照或加密引用。Usage Receipt 保存
considered/injected/omitted 与稳定 reason code。

### `outbox_events`

至少包含：

```text
id, tenant_id, aggregate_type, aggregate_id, aggregate_revision
event_type, payload, ordering_key
status, attempts, available_at, lease_owner, lease_until
created_at, processed_at
```

Worker 使用 `FOR UPDATE SKIP LOCKED` 或等价租约；同一个 `ordering_key` 的后继
Outbox 不得越过仍处于 Pending/Leased 的前驱；达到上限后进入 DLQ，不得无限热重试。

### `audit_events`

保存 actor、action、resource、request/trace ID、reason code、Policy 版本和时间。审计正文默认只记录 ID/hash，不复制敏感内容。

## 数据库级安全要求

- 所有受保护表启用 RLS，Tenant 条件来自经过验证的 session context；
- 应用数据库角色不能绕过 RLS；Migration/Erasure 使用分离角色；
- 所有跨表外键包含 `tenant_id`；
- 状态、类型和 reason code 使用受控值或 `CHECK`；
- idempotency 唯一约束以 Tenant 为边界；
- append-only 表通过数据库权限阻止普通角色 UPDATE/DELETE；
- Migration 使用向前兼容的 expand/migrate/contract，不承诺任意 DDL 原地回滚。

## 权威写事务

一次 Accept/Correct/Forget 必须在同一事务中完成：

1. 锁定 Head 或创建新 Record；
2. 校验 expected revision、Scope、Consent、状态和 Tombstone；
3. 追加 Decision/Version/Event；
4. 更新 Head；
5. 写 Outbox 和 Audit；
6. 提交。

事务失败不得留下已推进 Head 或缺失 Outbox 的 Version。Provider 调用不参与事务。

## 本地精确检索投影

`memory_search` SHOULD 保存当前合法 Version 的规范化正文、`tsvector`/trigram、scope 和 generation，用于精确关键词、Provider 降级和重建校验。它不是 Source of Truth，写入同样由 Outbox 驱动。
