# Inspect、Export、Expiry 与 Erasure 验证证据

验证日期：2026-07-28  
环境：本机 PostgreSQL、固定 Mem0 OSS runtime、hash-only POC Embedder、Rust
`memory-service`。

## 结论

- `GET /v1/memories/{id}` 返回按版本号和事件序号排序的 canonical 历史，并对其他
  用户隐藏记录；
- `GET /v1/memory/export` 在一个 `REPEATABLE READ READ ONLY` 事务中导出当前用户
  所有非 Erased 记录，包含 Version、Source reference、retention/Policy 元数据和
  Ledger Event；
- Expiry Worker 同时支持 `retain_until` 和 `effective_to`，先将 Head 置为
  `expired` 并使 Context Pack fail closed，再异步删除 Mem0/FTS；
- 重复 Expiry sweep 不会生成第二个事件或 operation；
- 记录级和用户级 Erasure 会擦除 Version/Candidate/Source/Context 正文，并在投影
  Receipt 后将 Request 标记为 `complete`；
- 后续 typed-scope 回归已补充已授权 Project Erasure；详见
  `demo-typed-scope-validation.md`。

## 自动化回归

```text
cargo clippy --workspace --all-targets -- -D warnings
→ passed

cargo test --workspace
→ 41 passed, 0 failed, 7 PostgreSQL tests intentionally ignored

TEST_DATABASE_URL=postgres://... cargo test -p memory-postgres \
  -- --ignored --test-threads=1
→ 7 passed, 0 failed
```

七条 PostgreSQL 集成测试包括：

```text
context_pack_consume_is_atomic_bound_and_idempotent
correction_appends_version_invalidates_pack_and_has_single_revision_winner
erasure_scrubs_canonical_content_and_completes_after_projection_receipt
exact_semantic_dedupe_reuses_record_and_attaches_sources_under_concurrency
export_is_authorized_and_expiry_is_fail_closed_and_projection_safe
semantic_conflict_is_fail_closed_until_review_accept_or_reject
typed_scopes_intersect_authority_and_project_erasure_is_tracked
```

SQL Ledger 不变量脚本通过，`_sqlx_migrations` 中 1～9 均为 `success=true`。
OpenAPI YAML 可解析，并确认 Export path 与 `expired` canonical status 已进入契约。

## 真实 HTTP + Mem0 Expiry 链

合成记录：

```text
memory_record_id  = a919ae13-3e11-553c-a766-d0bea8f12f0a
memory_version_id = 807864f9-3bf1-5c52-89ce-b480272c5e2e
upsert_operation  = a80ba1c1-3230-56ae-b855-132b74e88f46
provider_object   = af20adff-8db1-46f4-a20c-3d5f7797bc99
expiry_operation  = c9216bb3-4c4f-5828-be01-854c2b47cc0a
```

观察顺序：

1. 显式写入返回 `memory_accepted / pending`；
2. Worker 完成 Mem0 upsert，Operation 变为 `ready`；
3. Export 返回 `lazytt-memory-export-v1`，记录为 `active`，包含正文、Source 和
   retention 字段；
4. 将该合成 Version 的 `retain_until` 设置为过去时间；
5. 周期 Worker 自动追加 `memory_expired / RETENTION_EXPIRED`，Head 从 revision 0
   变为 revision 1；
6. Expiry projection operation 达到 `deleted`；
7. Search 对该 Record 返回空数组；
8. 直接查询旧 Mem0 provider object 返回 HTTP 404；
9. Export 仍允许用户获得该历史记录，但状态为 `expired`，不会进入 Search 或
   Context Pack。

## Erasure 验证

PostgreSQL 集成测试验证：

- Head 立即进入 `erased`，旧 Hit 与旧 Pack 立即失效；
- Version、Candidate、Source excerpt 和 Context snapshot 中不再存在原正文；
- 本地 FTS 立即删除；
- Provider delete operation 完成后，Erasure Request 从 `processing` 进入
  `complete`；
- 其他用户无法检查 Erasure Request；
- 用户级请求可在一个事务中处理多个 Record，并且持久化 target reference 不保存
  raw user ID。

此前 HTTP + Mem0 链的合成记录为：

```text
memory_record_id = 3ac4726a-2151-55ec-9f98-9f5d76969434
erasure_request  = 96f76e3c-bfac-5b9e-8518-2e681ca89f57
erasure_operation= 040cbb3c-d2b2-583d-a7f0-effa32c2cf70
```

最终 Request 为 `complete`、Inspect 返回 404、原 Mem0 object 不存在，数据库
只保留不含正文的 Head/Tombstone/Request 审计屏障。

## 尚不代表

本证据不代表 OIDC/JWKS、外部 membership authority、备份介质擦除、Projection
generation 全量重建或 Production Gate 已完成。
