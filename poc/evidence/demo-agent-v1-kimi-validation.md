# Demo Agent v1 真实 Kimi 验收

验证日期：2026-07-28

## 范围与安全边界

本次运行使用：

```text
Memory Service: http://127.0.0.1:8080
PostgreSQL:     127.0.0.1:55432
Mem0:          fixed local runtime
Agent model:   k3
Kimi endpoint: https://api.kimi.com/coding/v1/chat/completions
```

Kimi 凭据由仓库根目录、Git 已忽略且权限为 `0600` 的 `.env` 提供。验证记录、
程序输出和日志均未保存 Key、Authorization Header 或 Key 的任何片段。

`.env` 使用安全的 dotenv 数据解析器读取，没有被 Shell `source` 或执行。运行时
只确认 `apikey` 非空，并验证 `base_url` 和 `model_name` 分别为批准的官方 endpoint
与 `k3`。

## 真实端到端结果

`scripts/demo-agent-v1.sh` 成功完成：

1. canonical Memory accepted；
2. Outbox Worker 将 Mem0 Projection 推进到 `ready`；
3. 全新 Task/Attempt 检索目标 Memory；
4. 将一条授权 Preference 作为 `untrusted_memory_data` 提供给 Kimi `k3`；
5. Kimi 回答采用偏好要求的生活类比、【原理】和【示例】结构；
6. canonical Forget 提交后 Head revision 从 `0` 变为 `1`；
7. 在等待 Provider 删除之前，第二个全新 Task/Attempt 已无法读取目标 Memory；
8. 第二次 Kimi 调用的 `memory_context_count=0`；
9. Mem0 Projection 最终进入 `deleted`。

本次合成业务对象：

```text
memory_record_id:  df6b95b2-8246-5742-9041-1d2d22fef723
remember operation: 0fa7d6a3-592f-50bc-9dda-f9ccb1e9e2fc
forget operation:   5e336e48-e28a-597b-b1a9-a8f9a052c75c
```

第一次模型调用：

```text
model:                k3
memory_context_count: 1
prompt_tokens:        404
completion_tokens:    1061
total_tokens:         1465
```

第二次模型调用：

```text
model:                k3
memory_context_count: 0
prompt_tokens:        240
completion_tokens:    658
total_tokens:         898
```

本次 Provider 响应没有提供可用的 `x-request-id`，运行输出显示
`request_id=unavailable`。客户端随后已补充标准 Chat Completion `id` 作为 fallback，
后续运行将保留该非凭据 ID。

## 结论

Demo Agent v1 的真实业务链已经完成验收：

```text
Remember
→ canonical commit
→ Mem0 projection
→ fresh Agent attempt
→ authorized memory context
→ real Kimi k3 answer
→ Forget
→ immediate read denial
→ fresh Agent attempt without that memory
→ real Kimi k3 answer
→ projection deletion
```

该结果证明真实 Kimi 调用和单条显式 Preference 生命周期可工作，但不代表：

- POC 固定向量 Embedder 已具备语义检索质量；
- 多租户认证、RLS、真实 Agent Runtime 或 Production Gate 已完成。

## 服务端 Context Pack 改造后的再次验收

同日完成服务端 Context Pack、确定性预算、Usage Receipt 和 Forget invalidation
后，使用相同安全配置再次执行 `scripts/demo-agent-v1.sh`，真实 Kimi 链路通过。

本次业务对象：

```text
memory_record_id:    1b091478-82e1-50ac-9023-d03e24c84cc6
with-memory pack:    23e0775d-81c7-5dc5-a536-bd9391676cbc
without-memory pack: 8e443104-7d2f-59c8-ba5b-9bd310f2c859
```

第一次模型调用：

```text
model:                k3
request_id:           chatcmpl-g1GQ85i7K5p8F1fVqjpuyrW4
memory_context_count: 1
prompt_tokens:        415
completion_tokens:    1204
total_tokens:         1619
```

第二次模型调用：

```text
model:                k3
request_id:           chatcmpl-ppDdlzA9FafahGITftYEVjbB
memory_context_count: 0
prompt_tokens:        240
completion_tokens:    805
total_tokens:         1045
```

数据库只读核验显示，第一个 Pack 有一个 item 和一个 `injected` Usage Receipt，
Forget 后 `invalidated=true`、`invalidation_reason=MEMORY_FORGOTTEN`；第二个 Pack
为零 item 且仍有效。两个真实模型调用合计 `2664` tokens。

因此当前已验证链路更新为：

```text
Remember
→ canonical commit
→ Mem0 projection
→ fresh Agent attempt
→ server-side Context Pack + Usage Receipt
→ real Kimi k3 answer
→ Forget + old Pack invalidation
→ fresh Agent attempt with an empty Pack
→ real Kimi k3 answer
→ projection deletion
```

仍未完成的是消费前原子校验、真实 tokenizer/reranker、Pack 签名、完整
Policy/ACL/Consent 失效传播、真实语义检索评测和 Agent Runtime 正式集成。
