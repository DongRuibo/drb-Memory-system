# 安全、隐私与威胁模型

## 安全目标

- Tenant、User、Project、Agent 之间不发生越权读取或写入；
- 模型、Provider 和记忆正文不能授予 Authority；
- Secret 和未授权敏感个人信息不进入 Extractor、日志或索引；
- Forget/Erasure、Consent 撤销和成员关系变化可确定传播；
- 所有管理、审核、导出和擦除操作可追溯。

## 数据分类

| 分类 | 默认处理 |
|---|---|
| Public | 可按明确 Scope 保存 |
| Internal | 需要 Tenant 身份和 Purpose |
| Personal | 需要主体/业务依据、Retention 和导出/删除能力 |
| Sensitive | 默认 NeedsReview，强加密和最小化 |
| Secret | 禁止写入、禁止发送 Provider，触发安全审计 |

分类在调用外部 Extractor 前完成。日志、Trace 和错误信息不得复制 Personal/Sensitive/Secret 正文。

## 身份与授权

- 外部 API 使用短期服务/用户身份令牌；
- Runtime 请求必须携带可验证的 Task/Attempt 归属；
- Scope 和 principal 从认证上下文派生，不能信任请求体；
- 管理、Review、Export、Forget、Erasure 使用独立 permission；
- PostgreSQL RLS 作为应用校验之外的第二道边界；
- Authority snapshot 有短 TTL，Resume/Retry 必须重新验证。

当前实现边界（2026-07-28）：

- `MEMORY_AUTH_MODE=jwt_hs256` 从短期 Bearer JWT 派生 Tenant/User Principal，不读取
  请求体中的身份；
- 固定校验 `alg=HS256`、签名、issuer、audience、Tenant allowlist、`iat`、`nbf`、
  `exp`，令牌最大寿命一小时；
- API 分别要求 `memory:read/write/extract/correct/review/forget/export/erase/outcome/admin`，缺少
  令牌返回 401，缺少权限返回 403；generation rebuild/switch 仅允许
  `memory:admin`；
- 每个 allowlisted Tenant 启动独立 Worker，数据库事务通过 `app.tenant_id` 触发
  RLS；JWT 可携带 `tenant_scope`、`project_ids` 和 `agent_ids`，Operation、Inspect、
  Export、Search、Context 和治理接口执行 Tenant/User/Project/Agent typed-scope
  交集；
- `MEMORY_AUTH_MODE=demo` 只允许绑定 loopback，用于现有会议脚本，不能作为部署
  认证；
- Tenant/User 已通过真实 JWT HTTP 隔离验证；Project/Agent scope、membership
  removal 的 Pack 拒绝和 Project Erasure 已通过 PostgreSQL 集成测试；
- 签名 claim 目前是 membership snapshot，而不是在线目录查询。非对称 OIDC/JWKS、
  key rotation、主动权限撤销事件和 Authority snapshot cache 仍未完成。

## 主要威胁与控制

| 威胁 | 示例 | 必须控制 |
|---|---|---|
| 跨租户检索 | Provider filter 失效 | Provider 前置收窄 + canonical mapping + RLS + 安全测试 |
| Memory poisoning | 网页/工具输出要求保存恶意规则 | 来源等级、显式/推断区分、Policy/Review、不得授予权限 |
| Prompt Injection | 记忆内容要求忽略系统指令 | 作为引用数据封装、trust 标记、禁止工具参数直通 |
| Secret exfiltration | Token 被送入外部模型 | 本地 secret scan、拒绝、日志脱敏、Provider egress allowlist |
| Extractor poisoning/drift | 模型把短期话语当长期事实，重试返回另一组候选 | 类型/置信度/identity 白名单、Policy/Review、主操作候选快照、稳定子操作幂等 |
| Extractor data egress | 原始对话被发送到外部提取模型 | 独立 permission、模型前 Secret 拒绝、大小限制、官方 HTTPS allowlist、供应商条款 |
| Reranker data egress | 已授权正文被发送到外部排序模型 | 显式启用、官方 HTTPS allowlist、候选限长、Secret 拒绝、数据处理协议 |
| Reranker injection/越权 | 候选正文诱导模型返回新 ID，或模型调用期间撤权 | JSON-only schema、ID 全覆盖/唯一校验、只接受输入 ID、返回后 canonical 二次授权 |
| Stale authorization | 用户离开项目后旧索引仍命中 | 每次读取权威复核、短 TTL、fail closed |
| Replay resurrection | 旧 Outbox 恢复已删除内容 | revision、Tombstone、generation、幂等 operation ID |
| Insider abuse | 管理员批量导出 | 最小权限、双人审批可配置、审计、限流和告警 |
| Dependency compromise | Mem0 镜像或模型 SDK 被篡改 | 固定版本、镜像签名/SBOM、漏洞扫描、隔离网络 |
| Denial of service | 超长 Source/大量写入 | 大小限制、租户配额、背压、超时、熔断 |

## Prompt Injection 处理

记忆正文永远是 untrusted data。Context Compiler 必须：

- 使用稳定分隔和引用格式；
- 不把记忆正文放入 system policy 区；
- 不从正文解析权限、工具名或执行参数；
- 对程序性文本只生成 Procedural Candidate；
- 在高风险工具调用前由 Runtime 重新执行 Policy，与 Memory Pack 无关。

## 数据最小化与加密

- Source 默认保存 hash、引用和最小摘录，不复制完整聊天；
- Personal/Sensitive 内容字段使用应用层 envelope encryption 或等价控制；
- Key 按环境隔离并支持轮换；
- Provider 只获得检索所需的最小正文和 metadata；
- 备份加密，恢复权限与生产写权限分离。

## 外部语义 Reranker

`MEMORY_RERANK_MODE=kimi` 是显式的数据外发开关。启用时，服务只把当前请求 query
和已通过 canonical Authority 交集的限长候选正文发送到固定的 Kimi Code 官方
HTTPS endpoint，模型固定为 `k3`。API Key 只能来自进程环境或权限为 `0600` 的
本地演示 `.env`，不得进入请求日志、评测产物或错误消息。

候选正文仍是 untrusted data。Reranker 的 JSON 输出必须覆盖且只能覆盖输入 ID，
不接受未知、重复、缺失 ID、非法分数、超大或截断响应。服务不直接返回 Reranker
看到的正文；完成后会再次从 PostgreSQL 校验当前 Head、Version、Scope 和
Tombstone。正式部署前还必须完成供应商数据保留/训练/地域条款评审；不能仅因技术
上能够调用就默认允许真实用户正文外发。

`MEMORY_RERANK_MODE=dedicated` 使用相同信任边界。loopback 模型网关可避免把候选
正文发送给第三方；非 loopback 地址必须 HTTPS，并使用独立凭据。无论本地或外部，
模型响应都不能增加候选或绕过第二次 canonical 授权。

## 自动 Extractor

`MEMORY_EXTRACT_MODE=kimi` 是另一项显式外发开关，只接受具有 `memory:extract`
permission 的 `conversation_message`。服务在模型调用前拒绝 Secret，调用后再次检查
每个 Proposal；只允许有限类型、规范化 semantic key、至少 0.80 的 derived
confidence 和最多 8 个候选。

模型输出不会直接进入索引。首个完整候选集合先以 request hash 和
Idempotency-Key 固化到 Tenant 隔离的 `extraction_operations`，然后逐个进入正式
Policy/Review/Ledger 链。同一 Key 不能在重试时接受另一组模型输出。自动提取外发
真实用户对话前仍须完成 Kimi 的数据保留、训练、地域和删除条款评审。

## Forget、Erasure 与导出

- Forget 提交后立即从权威读取集合移除；
- Erasure 必须覆盖 Version content、Source excerpt、Provider/FTS/vector、cache 和可控 Context 快照；
- 审计仅保留政策允许的非识别 ID/hash；
- 导出按当前 Authority 重新过滤，不包含其他主体或内部 Provider 字段；
- 备份删除窗口、法律保留和恢复后 tombstone replay 必须写入运行手册。

## 安全发布门槛

- 跨租户/跨项目矩阵测试 100% 通过；
- Secret corpus 进入 Provider/日志的数量为 0；
- 旧索引、撤权、Forget、Erasure 重放测试全部通过；
- Prompt Injection 测试不能改变 Authority 或直接形成工具调用；
- 高危依赖漏洞为 0，并完成威胁模型评审。
