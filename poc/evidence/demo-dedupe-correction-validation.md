# Demo 去重与 Correction 验证

验证日期：2026-07-28

## 结论

user/preference 的精确 semantic identity 去重和显式 Correction 已形成可运行闭环：

```text
相同 identity + 相同正文
→ 复用 Record/Version/Projection Operation
→ 新 Source 附加到不可变 Version

指定 Record + expected_revision + 新正文
→ 追加 Source/Decision/Version/MemoryCorrected
→ Head revision + 1
→ 旧 Context Pack 立即失效
→ 新投影 Ready
→ 删除同 Record/generation 的旧 Provider/FTS 投影
```

这不包含不同正文冲突的 `NeedsReview`、`Contested` 和人工 Review API。

## 数据库与并发验证

真实本地 PostgreSQL 已应用 Migration 1 至 7。显式运行三个外部依赖测试：

```text
cargo test -p memory-postgres -- --ignored --nocapture

context_pack_consume_is_atomic_bound_and_idempotent                    PASS
correction_appends_version_invalidates_pack_and_has_single_revision_winner PASS
exact_semantic_dedupe_reuses_record_and_attaches_sources_under_concurrency PASS
```

Correction 测试覆盖：

1. 新 Version 与原 Version 属于同一 Record，Version ID 不同；
2. Head revision 从 0 推进为 1，状态返回 `memory_corrected`；
3. 相同幂等键和请求指纹返回相同 operation；
4. 相同幂等键但请求变化返回 Conflict；
5. 修正后旧 Context Pack 无法消费，失效原因是 `MEMORY_CORRECTED`；
6. 两个请求竞争同一个 expected revision 时只有一个成功；
7. 每次成功修正只追加一个 Version 和一个 `memory_corrected` Event。

精确去重测试覆盖顺序和并发重复写入，证明只有一个 live semantic Record/Version，
且不同 Source 都保留在 `memory_version_sources`。

## HTTP 与 Mem0 验证

使用最新 `memory-service`、真实 PostgreSQL、固定 Mem0 runtime 和本地 POC
Embedder 执行：

```text
original record:       0c8c5ad5-f871-5da4-8164-c70c64e01743
original version:      e9256fcd-ad6f-5643-9a5d-369ac1ad386c
original provider:     30bbf77d-2eea-448f-a1b7-6efb174a8135

corrected version:     b42e029a-e93b-5a67-b613-8c34eee51154
correction operation:  b8b3811f-a57a-505e-bc35-29262040b92b
corrected provider:    c227186a-e8de-400f-9a3f-c3f290f24cf2
```

结果：

- Correction 首次响应为 `memory_corrected / pending / revision=1`；
- Operation 最终为 `memory_corrected / ready`；
- 授权 Search 只返回 corrected Version 和新正文；
- Mem0 读取 original provider object 返回 `null`；
- Mem0 读取 corrected provider object 返回新正文和 revision 1 metadata；
- 相同 Correction 重放返回同一 operation/version/provider；
- 使用陈旧 revision 的新请求返回 HTTP 409。

## 工程回归

```text
cargo check --workspace                         PASS
cargo test --workspace                          PASS
cargo test -p memory-postgres -- --ignored       PASS
cargo clippy --workspace --all-targets -D warnings PASS
OpenAPI YAML parse                              PASS
Correction HTTP + Mem0 projection E2E           PASS
```

本验证不证明冲突审核、真实认证多租户、正式语义检索、Erasure、全量重建或
Production Gate。
