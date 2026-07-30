# Demo 冲突与 Review 验证

验证日期：2026-07-28

## 结论

user/preference 的同一 semantic identity 出现不同正文时，不再覆盖旧值或只返回
瞬时错误，而是提交可治理的冲突：

```text
新 Source + 冲突 Candidate
→ Candidate = NeedsReview
→ Head = Contested, revision + 1
→ 旧 Context Pack 失效
→ PostgreSQL 立即拒绝读取
→ Worker 删除旧投影
→ Review accept/reject
→ Head = Active, revision + 1
→ 当前 Version 重新投影
```

## PostgreSQL 验证

Migration `0008_memory_conflict_review.sql` 已应用。集成测试
`semantic_conflict_is_fail_closed_until_review_accept_or_reject` 通过，覆盖：

1. 冲突 Source/Candidate/Decision/Event/Head/Outbox 原子提交；
2. 相同冲突请求幂等重放到同一 operation；
3. Contested Head 即使收到旧 Provider hit 也不返回正文；
4. 冲突使旧 Context Pack 以 `MEMORY_CONTESTED` 失效；
5. Review Accept 追加新 Version 并恢复 Active；
6. Review Reject 保留原 Version 并恢复 Active；
7. Candidate revision 的接受/拒绝并发只有一个赢家；
8. 同一 Record 的 original upsert、contest delete、review upsert 按 Outbox 顺序领取；
9. 接受产生两个 Version，拒绝仍只有一个 Version。

## HTTP 与 Mem0 验证

使用最新 Memory Service、真实 PostgreSQL、固定 Mem0 runtime 和本地 POC
Embedder：

```text
record:                273eaa08-707c-558b-96ba-9fede45a00ef
original version:      f44d2721-9fea-5d37-a2fb-d6d2f18af602
conflict candidate:    8cabf3b2-dcdd-531d-bb94-1cee4745680b
conflict operation:    6424d294-9c9d-59b9-9257-4e3ff3a7b8fd
reviewed version:      4c92f3a2-8420-5743-9d4e-edf9e427d067
review operation:      d507f9d6-731a-510f-80d6-4ffe4d3c26a4
```

观察结果：

- 冲突响应为 `needs_review / revision=1`，包含稳定 Candidate ID 和 reason code；
- 冲突提交后的 Search 不包含该 Record；
- contest delete 最终为 `deleted`，原 Mem0 object 读取返回 `null`；
- Review Accept 响应为 `memory_accepted / revision=2`；
- Review upsert 最终 Ready，Search 返回 reviewed Version 和冲突新正文；
- 相同 Review 请求重放返回同一 operation/version/provider；
- 再次决策已终态 Candidate 返回 HTTP 409；
- reviewed Mem0 object metadata 携带 revision 2 和正确 Record/Version 映射。

本验证不证明真实 reviewer 角色授权、跨 Tenant/Project Scope、正式语义检索、
Erasure、全量重建或 Production Gate。
