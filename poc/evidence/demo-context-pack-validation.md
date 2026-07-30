# Demo Context Pack 本地端到端验证

验证日期：2026-07-28

## 验证范围

本次验证使用真实 PostgreSQL、Memory Service、Outbox Worker、固定 Mem0 runtime
和本地确定性 Chat Completions 假服务。它验证服务端 Context Pack、Usage Receipt
与 Forget invalidation；不调用外部模型，也不证明语义检索质量。

## 真实运行结果

`scripts/demo-agent-v1.sh` 完整通过：

1. 明确 Preference 写入 canonical Ledger，Head revision 为 `0`；
2. Outbox Worker 将 Mem0 Projection 推进到 `ready`；
3. 第一个新 Task/Attempt 获得 Context Pack，包含目标 Memory；
4. 本地模型收到一条 `untrusted_memory_data` 并生成带【原理】和【示例】的回答；
5. Forget 在权威事务中将 Head revision 推进到 `1`；
6. 第二个新 Task/Attempt 获得新的空 Context Pack；
7. Mem0 Projection 最终进入 `deleted`。
8. 相同的第二个 Context Pack 请求重放后仍返回
   `159acb9b-abad-59ed-b01d-f4050844d93c`，HTTP `201`，没有创建第二个 Pack；
9. 重放已被 Forget 失效的第一个 Pack 请求返回 HTTP `409 CONFLICT`，没有重新暴露
   旧记忆。

本次运行对象：

```text
memory_record_id:       2736a0a4-c867-5d44-a570-8ad6c5b73da7
with-memory pack:       83c2f897-e750-50bb-b63a-a31e30f2e40d
without-memory pack:    159acb9b-abad-59ed-b01d-f4050844d93c
```

数据库只读核验：

```text
with-memory pack:
  estimated_tokens=67
  item_count=1
  injected_receipts=1
  omitted_receipts=0
  invalidated=true
  invalidation_reason=MEMORY_FORGOTTEN

without-memory pack:
  estimated_tokens=0
  item_count=0
  injected_receipts=0
  omitted_receipts=0
  invalidated=false
```

这证明旧 Pack 不只是从第二次 API 响应中消失，而是被 Forget 事务显式失效；第一条
授权候选也产生了可审计的 `injected` Usage Receipt。

## 工程回归

```text
cargo fmt --all -- --check                 PASS
cargo clippy --workspace --all-targets
  -- -D warnings                           PASS
cargo test --workspace                     PASS (30 tests)
ledger_invariants.sql                      PASS
OpenAPI YAML parse                         PASS
bash -n demo scripts                       PASS
Python POC server compile                  PASS
git diff --check                           PASS
repository long-format API-key scan        NO MATCH
```

本地 PostgreSQL 的 `_sqlx_migrations` 显示 1 至 5 全部 `success=true`，其中 version 5
为 `context pack idempotency`。

## 实现边界

- 当前 token estimator 为 `demo-conservative-char-v1`，不是 Kimi tokenizer；
- 当前 POC Embedder 返回固定向量，不能证明多记忆相关性；
- 当前只实现 Preference/`answer_user`；
- 尚无 Context Pack 消费前的原子有效性校验或租约，因此仍存在 Pack 返回后、
  模型调用前并发 Forget 的窗口；
- 尚未实现 Pack 签名、Outcome Receipt 和 Policy/ACL/Consent 全量失效传播。
