# Database Migrations

`0015_demo_chat_runtime.sql` 增加交互式演示的持久化运行时。`demo_chat` 与权威记忆账本
`memory_core` 明确分离。

`0016_demo_chat_client_message_id.sql` 只对非空的浏览器消息 ID 建唯一索引，允许每个
会话拥有多条没有客户端 ID 的 assistant 消息。

Migration 按文件名顺序执行，目标数据库必须是 PostgreSQL。应用生成 UUID；Migration 不依赖 `pgcrypto`。

本地验证：

```bash
docker compose -f deploy/local/postgres.compose.yaml up -d
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0001_memory_ledger.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0002_head_transition_guard.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0003_projection_unknown_and_fencing.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0004_source_client_event_id.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0005_context_pack_idempotency.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0006_context_pack_consumption.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0007_memory_semantic_identity.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0008_memory_conflict_review.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0009_memory_erasure_workflow.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0010_context_pack_ranking_version.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0011_projection_generation_rebuild.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0012_outcome_receipts.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0013_projection_model_manifest.sql
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /migrations/0014_extraction_operation_snapshots.sql
```

Migration 之后运行 SQL 不变量测试：

```bash
docker compose -f deploy/local/postgres.compose.yaml exec -T postgres \
  psql -U memory -d memory -v ON_ERROR_STOP=1 \
  -f /tests/sql/ledger_invariants.sql
```

`0001_memory_ledger.sql` 建立权威 Ledger、Outbox、Context、Erasure 和本地 FTS 投影的第一版约束；`0002_head_transition_guard.sql` 增加 Head revision 单步递增、终态不可复活及可读状态必须引用当前 Version 的状态机守卫；`0003_projection_unknown_and_fencing.sql` 增加 Provider unknown-result 状态和 Worker fencing version；`0004_source_client_event_id.sql` 保存并唯一约束 API `event_id`；`0005_context_pack_idempotency.sql` 唯一约束一次 Context Pack 编译的稳定身份；`0006_context_pack_consumption.sql` 增加原子消费时间、请求指纹和 Tenant 级消费幂等唯一约束；`0007_memory_semantic_identity.sql` 增加 live semantic identity 唯一约束、identity retirement、Candidate target 和 Source 到 canonical operation 的稳定结果映射；`0008_memory_conflict_review.sql` 增加 open Review 唯一约束、Review 幂等结果和每 Record Outbox 顺序索引；`0009_memory_erasure_workflow.sql` 增加 Erasure 幂等请求指纹、逐 Record 投影进度、Candidate/Version 正文擦除约束，并允许 Expired Head 继续进入 Erased 终态；`0010_context_pack_ranking_version.sql` 把实际 Reranker 版本固化到 Context Pack，避免切换排序策略后错误复用旧 Pack；`0011_projection_generation_rebuild.sql` 增加 generation 状态机、幂等切换记录、RLS、初始 generation 回填，并把实际 projection generation 固化到 Context Pack；`0012_outcome_receipts.sql` 增加 Attempt-bound、Tenant 隔离且幂等的 Outcome Receipt，只保存模型与用量元数据和供应商请求 ID hash，不保存回答正文；`0013_projection_model_manifest.sql` 把索引、Embedding 提供方、模型、revision、维度、归一化、指令版本和清单 hash 固化到每个 generation，阻止错误模型混写；`0014_extraction_operation_snapshots.sql` 固化自动提取请求的首个完整候选集合和零候选结果，阻止模型重试漂移产生第二组正式记忆；`0015` 至 `0021` 依次增加 Demo Chat、边界安全、中文 BM25、时间检索和 Recall Explain 回执；`0022_unified_memory_taxonomy.sql` 将正式类型统一为 `preference`、`profile_fact`、`semantic_fact`、`episodic_summary`，阻断 identity 碰撞与正式 Procedure，并保存不含正文的重分类审计；`0023_native_memory_event_time.sql` 为原生 `episodic_summary` Candidate/Version 增加半开 Event Time、IANA 时区、时间精度与检索索引；`0024_versioned_structured_content.sql` 为 Candidate/Version 增加版本化、限长且可擦除的类型专属 JSON envelope；`0025_remove_abandoned_external_import_staging.sql` 幂等删除已经放弃的外部记忆导入暂存表，不改动正式 Ledger 行。生产数据库角色与 Grant 在部署环境中管理，不能使用表 Owner 作为应用身份。

`0026_extraction_diagnostics.sql` 将不含正文的候选数量、过滤原因和受限覆盖统计与提取快照一起固化，使幂等重放和 Demo Web 都能解释同一轮提取结果。

`0031_episode_consolidation.sql` 增加 Tenant 隔离的 Episode、Completed Turn 与异步 Consolidation Job；稳定轮次序号、覆盖区间、幂等键和 Worker 租约保证每段已完成对话最多排队一次，并保存可重放的 0～4 个 Candidate Proposal 快照。

`0027_context_pack_runtime_and_event_snapshot.sql` 将服务器生成的参考时间与时区固化到
Context Pack，并把每条记忆的 Event Time 一起快照，保证时间检索、最终提示词和证据
面板使用同一份不可变时间证据。

`0028_prompt_context_contract_gate.sql` 只允许由 `demo-context-compiler-v2` 生成的 Pack
作为“模型实际收到”的证据；旧 Pack 即使因迁移补齐了时间列，也不会被伪装成历史 Prompt。

`0029_demo_turn_fallback_runtime_context.sql` 仅为 Context Pack 不可用的降级轮次保存非敏感
运行时上下文和空记忆数组，使证据面板仍能精确展示模型输入而不复制记忆正文。

`0030_demo_chat_standalone_snapshot.sql` 是拆仓兼容 Migration：它把模型实际使用的完整
Prompt Context 保存到 `demo_chat.turns.prompt_context_snapshot`，并可在没有
`memory_core` 的干净 Demo 数据库中建立最终 Demo Schema。相同文件同时嵌入
`apps/demo-web-api/migrations/`；两份文件必须保持相同 checksum。
