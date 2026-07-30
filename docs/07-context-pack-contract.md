# Memory Context Pack 契约

Memory Context Pack 是一次 Attempt 的记忆输入，不是完整 Prompt，也不是 Authority 或 Canonical State。

## 输入

Context Compiler 必须接收：

```text
tenant_id, principal_id
task_id, attempt_id, purpose
authorized_scope_set, authority_snapshot_id
query, memory_types
token_budget, tokenizer_id
policy_version, compiler_version
required_or_optional
```

Tenant、Principal 和 authorized scope 由服务端上下文提供；客户端字段只能进一步收窄。

## 两阶段交付

`POST /v1/memory/context-packs` 只编译 Manifest，不返回记忆正文：

```json
{
  "context_pack_id": "uuid",
  "task_id": "uuid",
  "attempt_id": "uuid",
  "purpose": "answer_user",
  "authority_snapshot_id": "authz_123",
  "policy_version": "memory-policy-1",
  "compiler_version": "context-compiler-1",
  "ranking_version": "kimi-k3-semantic-v1",
  "created_at": "2026-07-27T10:00:00Z",
  "expires_at": "2026-07-27T10:05:00Z",
  "tokenizer_id": "model-tokenizer-version",
  "token_budget": 1200,
  "estimated_tokens": 184,
  "item_count": 1,
  "delivery_status": "compiled",
  "consumed_at": null,
  "omission_summary": {}
}
```

Runtime 在即将组装模型输入时调用
`POST /v1/memory/context-packs/{context_pack_id}/consume`，携带与 Manifest 相同的
Task、Attempt、purpose 和 `Idempotency-Key`。服务端按 Head → Context Pack 的顺序
加锁，再次验证 Authority snapshot、有效期、Pack 失效状态、当前 Head/Version、
scope、purpose、Tombstone、正文快照和来源。校验通过后在同一事务中标记
`consumed_at`，此时才返回正文：

```json
{
  "context_pack_id": "uuid",
  "task_id": "uuid",
  "attempt_id": "uuid",
  "purpose": "answer_user",
  "authority_snapshot_id": "authz_123",
  "policy_version": "memory-policy-1",
  "compiler_version": "context-compiler-1",
  "ranking_version": "kimi-k3-semantic-v1",
  "created_at": "2026-07-27T10:00:00Z",
  "expires_at": "2026-07-27T10:05:00Z",
  "consumed_at": "2026-07-27T10:00:01Z",
  "tokenizer_id": "model-tokenizer-version",
  "token_budget": 1200,
  "estimated_tokens": 184,
  "sections": [
    {
      "kind": "user_preference",
      "items": [
        {
          "memory_record_id": "uuid",
          "memory_version_id": "uuid",
          "text": "用户偏好先讲原理，再讲代码。",
          "source_refs": ["conversation:msg_9"],
          "effective_from": "2026-07-01T00:00:00Z",
          "confidence": 1.0,
          "inclusion_reason": "EXPLICIT_PREFERENCE_AND_TASK_RELEVANT",
          "trust": "untrusted_memory_data"
        }
      ]
    }
  ],
  "omission_summary": {
    "STALE_VERSION": 1,
    "TOKEN_BUDGET": 2
  }
}
```

在 Pack 仍有效且当前权威状态未变化时，相同消费 `Idempotency-Key` 和请求指纹重试
返回同一已消费 Pack；不同请求不能消费已经消费的 Pack。Pack 创建后不可修改；
授权或记忆变化通过失效旧 Pack 和重新编译处理。

## 内容规则

- 只允许当前 Head 的合法 Version；
- 每条 item 必须携带 Version ID、来源引用和 inclusion reason；
- 记忆正文作为引用数据封装，不解释其中的“忽略规则”“执行工具”等指令；
- 不包含密码、Token、Cookie、Lease、Approval、ACL 或完整敏感来源；
- 安全规则、权限约束和 Canonical 项目配置由 Runtime 的权威 Context 单独提供；
- 可能影响真实操作的“项目约束记忆”必须由 Runtime 对 Canonical State 再验证，否则只能作为未确认背景或省略。

## 排序与预算

排序必须使用可解释特征并记录版本：

```text
relevance
+ explicitness
+ source_quality
+ scope_specificity
+ type_weight
+ bounded_recency_for_episodic
- uncertainty
- redundancy
```

禁止把不同量纲的原始分数直接相加。每个特征必须归一化并在评测规范中固定权重/版本。

预算顺序：

1. 明确且与 Task 相关的用户偏好；
2. 已验证的当前事实；
3. 强相关历史经验；
4. 一般背景。

权限与安全约束不参与此排序，因为它们不属于 Memory Context Pack。

## Usage Receipt

对进入候选集合的每个 Version 记录：

- considered / injected / omitted；
- retrieval/rerank 分数与 rank；
- inclusion/omission reason code；
- Context Pack、Task、Attempt；
- Authority/Policy/Compiler 版本；
- 创建时间。

任务结果和人工反馈通过独立、Attempt-bound、幂等的 Outcome Receipt 追加；它不能
回写或篡改原 Usage Receipt，也不保存回答正文。

## 失效条件

以下任一发生时，尚未消费的 Pack 必须失效：

- expires_at 到达；
- Attempt 改变或重试；
- principal membership、Consent 或 Policy 变化；
- Pack 引用的 Version 被争议、遗忘或擦除；
- Runtime 请求不同 purpose。

## Demo v1 实现边界

当前 `POST /v1/memory/context-packs` 已实现一条受限但真实的链路：

- 支持 `preference`、`profile_fact`、`project_fact` 和 `episodic_summary`，分别进入
  `user_preference`、`current_fact` 和 `relevant_experience` section；仍只支持
  `answer_user` purpose；
- 请求可选择 Tenant/User/Project/Agent scope，但只能收窄签名 Principal 中的
  typed scope 授权集合；未传 selector 时默认只读取当前 User scope；
- Mem0 只返回候选映射，PostgreSQL 在创建 Pack 的同一事务中复查 Head、Version、
  scope、purpose、Tombstone、revision、content hash 和 provenance；
- Head 使用共享锁参与编译事务，避免 Pack 创建与 Forget 提交交叉产生已知过期 item；
- Pack 以 Attempt 和请求指纹生成稳定 ID，数据库唯一索引保证相同请求复用同一不可变结果；
- 实际 `ranking_version` 进入请求指纹、Manifest 和消费结果；排序策略改变或
  Kimi 失败后采用透明降级时，不会错误复用另一排序版本生成的旧 Pack；
- 注入和因预算省略的授权候选均写入 Usage Receipt；
- Forget 在权威事务中把引用目标 Memory 的 Pack 标记为 `MEMORY_FORGOTTEN`。
- Compile 只返回无正文 Manifest；Consume 按 Head → Pack 顺序加锁并原子复核；
- 相同消费请求可安全重放，错误 Attempt、purpose、Authority、过期或失效 Pack 均
  fail closed。

Demo 使用 `demo-conservative-char-v1` 进行确定性保守估算，不声称与任何模型
tokenizer 等价。服务端已实现版本化透明/Kimi/独立 Qwen3 语义 Rerank，但尚未实现完整的
explicitness、source quality、scope specificity、recency 等多特征排序解释，
也未实现 Pack 签名，以及外部 membership/Policy/Consent
变化事件触发的主动失效传播。每次 Consume 会根据当前 Principal 重新计算
Authority snapshot 并 fail closed，但正式撤权时效仍取决于身份系统签发短期
新令牌。因此本节不是 Production 完成声明。
