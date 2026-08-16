# Agent Runtime 集成契约

## 目标

Agent Runtime 不直接访问 PostgreSQL、Mem0 或 Provider。Runtime 只依赖
`crates/memory-client` 暴露的 `MemoryPort`，把长期记忆作为一次 Attempt 的受控输入：

```text
Task / Attempt / Purpose / 当前身份
→ MemoryPort::acquire_context_pack
→ 服务端 Compile（只返回 Manifest，不返回正文）
→ 服务端 Consume（原子重新授权并交付正文）
→ Runtime 把 item 作为 untrusted_memory_data 注入模型
→ Runtime 保存 context_pack_id、Memory ID、模型调用和结果关联
```

`apps/demo-agent` 已迁移到这个 Port，是当前可运行的参考接入；它不是 LazyTT
Agent Runtime 本体。

## 客户端已经提供的控制

`MemoryHttpClient` 提供：

- `submit_event`、`append_completed_turn`、`request_episode_consolidation`、
  `episode_status`、`operation`、`compile_context_pack`、`consume_context_pack` 和 `forget`
  的类型化 API；
- `acquire_context_pack` 组合操作，校验 Task/Attempt/Purpose、Authority snapshot、
  Policy/Compiler/Ranking version、projection generation、预算、有效期和 item 数量；
- Consume 后再次核对 Manifest，并拒绝没有 provenance 或未标记为
  `untrusted_memory_data` 的 item；
- 对带 Idempotency-Key 的写入、Consume，以及由 Attempt 请求指纹稳定标识的
  Compile 只做有界传输重试，不把未知结果重新解释为新操作；
- 禁止 HTTP 重定向；非 loopback 地址必须使用 HTTPS 和 Bearer Token；
- Bearer Header 标记为敏感值，错误只保留状态码、短错误码和 request ID。

Runtime 可以使用 loopback Demo 模式：

```rust
let client = MemoryHttpClient::new(MemoryClientConfig::loopback(
    "http://127.0.0.1:8080",
    Duration::from_secs(5),
))?;
```

正式集成应由 Runtime 的身份组件注入短期令牌：

```rust
let client = MemoryHttpClient::new(MemoryClientConfig::authenticated(
    "https://memory.internal.example",
    Duration::from_secs(5),
    short_lived_bearer,
))?;
```

令牌、Tenant、User、Project、Agent 和 permission 必须由受信任身份系统派生，不能从
模型输出或用户正文生成。

## Runtime 的责任

客户端不会替 Runtime 做以下决定：

1. 为每次执行生成稳定的 `task_id` 和新的 `attempt_id`；
2. 从当前执行上下文确定 `purpose`、允许收窄的 Scope 和 token budget；
3. 在调用模型之前完成 Consume，禁止直接使用旧 Pack 或缓存正文；
4. 把记忆放在明确的未信任数据字段，禁止拼接进 system/developer policy；
5. 保存 `context_pack_id`、使用的 Memory/Version ID、模型 request ID 和 Outcome；
6. Attempt 重试、身份或权限变化时重新获取 Pack；
7. Forget、Correction 或撤权冲突时 fail closed，不回退到客户端缓存。

## Completed Turn 与 Episode Consolidation V1

Runtime 在可见最终回答产生后，可把本轮受控结果追加到稳定的 `episode_id`：

```text
用户消息 + 可见最终回答 + 已验证成功的工具结果 + Outcome
→ POST /v1/memory/episodes/{episode_id}/turns
→ 立即从该完整轮次提取 Candidate，供后续轮次及时召回
→ 每 10 个尚未排队的 Completed Turn 自动建立一个不重叠巩固区间
→ 后台 Consolidation Worker 固化 0～4 个 Proposal 快照
→ 复用 Candidate → Policy/Scope → Version/Head/Ledger → Outbox
```

隐藏推理、失败工具日志和未经验证的 Observation 不属于 Completed Turn。Worker 失败不会
阻塞回答；租约超时可重领，最多尝试 8 次。相同巩固区间首次产生的 Proposal 会持久化，
重试不会接受第二组模型结果。Runtime 也可用 `manual` 或 `session_end` 触发未排队轮次，
并通过 `/status` 查看覆盖位置和最近 Job。

即时提取与十轮巩固是两条独立链路：前者替代旧的单条用户消息提取，保证新偏好和事实能
尽快进入长期记忆；后者总结跨轮经历和重复模式。两者都复用相同的 Candidate 治理链。

V1 只巩固一个 Episode 内的轮次，继续复用现有四类记忆；跨 Episode 经验学习不在本阶段。

## 最小正式验收

在宣告 Agent Runtime 集成完成前，至少需要独立于 Demo Agent 的契约测试：

- 无记忆、正常记忆、`memory_required=true` 无命中；
- Compile 后 Forget，Consume 必须失败；
- 错误 Attempt、Purpose、Authority 或过期 Pack 必须失败；
- 同一 Consume Idempotency-Key 重放返回同一结果，不同请求复用 Key 必须冲突；
- Project membership 移除后不能继续消费旧 Pack；
- projection generation 切换前后，新 Pack 绑定实际 generation；
- 模型 Prompt 中记忆只能进入未信任数据区；
- Memory Service 超时后的重试不产生第二个 canonical 操作；
- Outcome/Usage 关联可从一次 Agent Attempt 追溯到具体 Pack 和 Memory Version。
- Completed Turn 幂等重放、10 轮阈值、手动触发、Worker 崩溃重领、零 Candidate 和
  Candidate 治理链均有离线 Fake 覆盖。

## 尚未完成

- 尚未在 LazyTT Agent Runtime 仓库中接入 `MemoryPort`；
- 尚未接入 Runtime 的 OIDC/JWKS 身份和短期令牌刷新；
- Outcome Receipt 已实现；尚未实现正式 tokenizer 预算和 Pack 签名；
- 尚未完成上述独立契约测试及真实 Runtime 端到端验收。
