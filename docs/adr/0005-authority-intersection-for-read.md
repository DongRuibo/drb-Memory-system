---
status: accepted
---

# Authorize memory reads by intersection

一条 Memory Version 只有同时满足 Task Need、当前身份与成员关系、Tenant、Memory Scope、Purpose、Consent、Policy、有效版本、冲突状态和预算约束时才能进入 Memory Context Pack。相比只依赖 Provider metadata filter，这会增加一次权威数据库校验，但能够使成员变更、撤权、遗忘和索引陈旧立即生效。

## Consequences

- Scope 由服务端根据认证与 Task 上下文派生，客户端只能请求进一步收窄。
- 不确定授权时返回省略或拒绝，不能猜测。
- Memory Context Pack 不得承载或授予安全策略、ACL、Lease、凭证或工具权限。
