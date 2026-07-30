# Memory Policy

Memory Policy 是版本化、确定性、可审计的决策集合。模型可以提供特征或建议，但不能作为最终 Policy。

## Policy 输入

- Auth Context：tenant、principal、roles、membership snapshot；
- Task Context：task、attempt、purpose、project/agent；
- Source Event：类型、来源、时间、正文分类、source hash；
- Candidate：类型、内容、置信度、normalized key、推断标记；
- Existing Head：当前版本、有效期、冲突状态；
- Consent 与 Retention 引用；
- 当前 `policy_version`。

## 写入准入

### 自动接受范围

第一版只允许自动接受：

- 用户对自己的明确长期表达偏好；
- 有授权、可验证来源的低风险项目事实；
- 已验证 Task Result 形成的低风险 Episodic Summary。

### 必须拒绝

- 密码、API Key、Token、Cookie、私钥、完整支付信息；
- 未授权第三方敏感个人信息；
- 当前 Task 中间状态、心跳、重复系统日志；
- 可以直接从 Canonical State 查询且不应复制的权限或业务状态；
- 要求绕过安全策略、审批或工具权限的文本；
- 缺少 Tenant、来源或允许用途的数据。

### 必须人工审核

- 身份、企业关键事实和影响真实操作的约束；
- 与当前 Head 冲突但无法确定时间关系的内容；
- 模型推断而非用户明确陈述的 User Profile；
- Procedural Candidate；
- 涉及第三方或高敏感分类的允许保存请求。

## Scope 规则

每条 Memory Record 只属于一个 typed scope：

| `scope_kind` | `scope_id` | 示例 |
|---|---|---|
| `tenant` | null | 租户公开且获授权的共享约定 |
| `user` | user ID | 用户跨项目偏好 |
| `project` | project ID | 项目内事实与经验 |
| `agent` | agent ID | 特定长期 Agent 的非权限型信息 |

- Tenant 总是必填；
- `tenant` scope 以外的 `scope_id` 必填；
- 一条记录不能同时属于 user 和 project；需要共享时创建有明确来源的独立记录；
- Client 提供的 Scope 只用于请求收窄，服务端根据认证和 Task Context 派生最终允许 Scope 集合。

## 读取决策

读取必须计算：

```text
task_need
AND principal_membership
AND tenant_match
AND authorized_scope
AND purpose_allowed
AND consent_valid
AND policy_allows
AND head_is_current
AND effective_time_valid
AND not_contested_or_forgotten
```

任一条件未知或失败时不得注入。拒绝原因使用稳定 reason code，例如 `SCOPE_DENIED`、`CONSENT_REVOKED`、`STALE_VERSION`、`CONTESTED`，审计日志不得记录被拒绝的敏感正文。

## 冲突与修正

- 内容相同且 normalized key 相同：保留当前 Version，追加来源；
- 用户明确修正自己提供的信息：创建新 Version；
- 权威项目来源更新旧事实：创建新 Version，并记录 effective time；
- 来源等级相当且无法判断：Head 进入 `Contested`，默认不注入；
- normalized key 只是候选匹配信号，不能单独决定覆盖。

当前实现边界（2026-07-28）：

- user/preference 的相同 semantic identity 与相同 content hash 已由事务 advisory
  lock、数据库唯一索引和 `memory_ingest_results` 实现单 Record/Version 与多来源；
- 显式 Correction 必须指定 Record、`expected_revision`、来源、reason 和幂等键，
  已实现不可变新 Version、`MemoryCorrected`、Head 推进、Pack 失效和投影替换；
- 相同 semantic identity 但正文不同会提交 `NeedsReview` Candidate，使 Head
  进入 `Contested`，立即阻止读取并删除投影；
- Review 接受时追加新 Version，拒绝时保留原 Version；两者都推进 Candidate/Head
  revision、追加最终 Decision/ContestResolved Event 并重建当前投影。

## Retention 与 Consent

每个被接受 Version 必须记录：

- `retention_policy_id` 和计算后的 `retain_until`；
- 需要时记录 `consent_ref`；
- 数据分类和允许用途；
- Policy 版本及 Decision actor。

Consent 撤销、Retention 到期和成员关系失效必须触发 Head 失效与投影删除；是否进一步执行物理擦除由数据分类和适用政策决定。
