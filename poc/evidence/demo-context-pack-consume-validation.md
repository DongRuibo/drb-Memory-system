# Demo Context Pack 原子消费验证

验证日期：2026-07-28

## 结论

Context Pack 已改为两阶段交付：

```text
Compile 无正文 Manifest
→ Consume 锁定 Head
→ 锁定 Pack
→ 复核 Attempt / Authority / 有效期 / Head / Version / Scope / Purpose
→ 复核 Tombstone / 正文快照 / 来源
→ 同一事务写入 consumed_at 和消费幂等指纹
→ 返回未信任记忆正文
```

Memory Service、Demo Agent 和 OpenAPI 均已切换到该契约。Compile Response 不再包含
`sections` 或记忆正文；只有 Consume 成功后才返回 Context Pack items。

## 数据库集成验证

Migration `0006_context_pack_consumption.sql` 已应用到真实本地 PostgreSQL。
`_sqlx_migrations` 显示 version 1 至 6 均为 `success=true`。

被忽略的外部依赖测试已显式运行并通过：

```text
cargo test -p memory-postgres \
  context_pack_consume_is_atomic_bound_and_idempotent \
  -- --ignored

PASS
```

测试覆盖：

1. 首次 Consume 返回一个当前、授权、有来源的 Preference；
2. 相同 `Idempotency-Key` 和请求指纹重放返回完全相同结果；
3. 错误 Attempt 被拒绝；
4. 过期 Pack 被拒绝；
5. 已消费 Pack 在后续 Forget 失效后不再允许重放正文；
6. Forget 先提交后，Consume 被拒绝；
7. Consume 与 Forget 并发不会死锁；
8. 并发结果只能是 Consume 先完成交付，或 Forget 先完成并使 Consume fail closed。

## 完整 HTTP 链路

使用真实 PostgreSQL、固定 Mem0 runtime、Memory Service 和本地确定性 Chat
Completions 假服务运行 `scripts/demo-agent-v1.sh`，结果通过：

```text
memory_record_id: 51b95b6e-e246-5c2e-afcd-c61ff5a2fe5f
with-memory pack: 736236cd-22f6-5d47-9838-887d4e5bfe89
without-memory pack: 1bbb88b6-e32f-5150-aec0-4fb640ae295e
```

第一次全新 Attempt 经 Compile + Consume 得到一条记忆；Forget 后第二次全新
Attempt 经 Compile + Consume 得到零条记忆；Provider Projection 最终进入
`deleted`。

数据库只读核验：

```text
with-memory pack:    consumed=true invalidated=true
                     reason=MEMORY_FORGOTTEN item_count=1
without-memory pack: consumed=true invalidated=false item_count=0
```

## 工程回归

```text
cargo fmt --all -- --check                 PASS
cargo clippy --workspace --all-targets
  -- -D warnings                           PASS
cargo test --workspace                     PASS
database consume integration               PASS
Migration 1..6                             PASS
local Demo Agent HTTP E2E                  PASS
```

本验证不证明正式 tokenizer/reranker、真实中文语义检索、Pack 签名、
Outcome Receipt、完整 Policy/ACL/Consent 失效传播或 Production Gate。
