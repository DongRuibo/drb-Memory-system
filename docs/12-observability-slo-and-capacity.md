# 可观测性、SLO 与容量

## 追踪关联

所有请求和后台操作携带：

```text
request_id, trace_id, operation_id
tenant_id_hash, task_id, attempt_id
memory_record_id, memory_version_id
policy_version, provider, generation
```

Telemetry 默认不记录 Memory 正文、Source excerpt、query 原文或身份令牌。

## 核心 Span

```text
memory.event.admit
memory.extract
memory.policy.decide
memory.ledger.commit
memory.projection.enqueue
memory.projection.apply
memory.authority.check
memory.retrieval.search
memory.context.compile
memory.forget
memory.erasure.process
memory.reconcile
```

## 指标

### 正确性与安全

```text
memory_authority_denied_total{reason}
memory_scope_violation_total
memory_stale_hit_total
memory_orphan_projection_total
memory_tombstone_block_total
memory_secret_rejected_total
memory_context_invalidated_total{reason}
```

### 可靠性

```text
memory_api_requests_total{operation,status}
memory_api_latency_seconds{operation}
memory_ledger_commit_failure_total
memory_projection_backlog
memory_projection_lag_seconds
memory_projection_failure_total{provider,category}
memory_outbox_oldest_age_seconds
memory_dlq_total
memory_erasure_age_seconds
```

### 质量与成本

```text
memory_candidate_decisions_total{outcome,type}
memory_duplicate_total
memory_retrieval_hits_total
memory_context_items_total{result,reason}
memory_context_tokens_total
memory_provider_tokens_total{operation,model}
memory_provider_cost_total{provider,model}
```

Tenant label 必须使用受控 hash/bucket，避免高基数和身份泄漏。

## 初始 SLO 草案

以下目标必须通过 POC/压测校准后才能转为发布 SLO：

| SLI | POC 目标 |
|---|---|
| 权威 search/context API availability | 99.9%（不含 Provider 质量） |
| Context compile P95 | 目标值待 POC，分别报告本地与 Provider 路径 |
| Ledger write P95 | 目标值待 POC，不把异步 projection 时间算入 |
| 95% projection lag | 目标值待 POC |
| Forget 权威失效 | DB commit 后立即生效 |
| Erasure completion | 按数据分类设 deadline，超时告警 |
| Scope/Secret/Stale 注入 | 0 |

禁止在没有负载模型和原始数据时填写拍脑袋延迟数字。

## 告警

立即告警：

- 任意 scope violation、Secret escape 或 Forgotten/Erased 注入；
- Tombstone 后出现较旧 upsert；
- RLS/Authority Gateway 故障；
- Erasure deadline 违约。

工单/趋势告警：

- projection backlog/lag 持续超阈值；
- DLQ、orphan projection、reconcile mismatch 增长；
- Provider 成本或 false memory 回归；
- Context 空结果率异常变化。

## 容量模型

上线前基于以下变量估算，而不是直接采用固定机器规格：

```text
active tenants/users
source events per second
candidates and accepted versions per event
average retained content bytes
embedding dimension and index overhead
context compiles per second and Top-K
retention period and version growth
projection rebuild duration
erasure and outbox backlog peak
```

压测同时覆盖正常负载、Provider 降级、批量重建和删除高峰，记录数据库连接、锁竞争、Worker backlog、CPU、内存、IO 和成本。
