---
status: accepted
---

# Keep demo identity selection outside the Memory Service trust boundary

本地 Demo Web 提供 Alice、Bob、Carol 三个固定身份，用于直观验证“个人隔离、同项目
共享、项目外不可见”。浏览器只持有不透明的 Demo Viewer Session，并选择预置身份；
它不接收 Memory Service JWT，也不能提交任意 tenant、user、project 或 permission。
Demo Web API 根据服务端 allowlist 构造 Active Principal，为每个请求签发短期 JWT，
再以该 Authority 调用 Memory Service。

## Considered Options

- 浏览器直接提交 `user_id` 或长期 JWT：实现简单，但会把演示选择器误当成权限来源，
  也会把服务凭据暴露给浏览器。
- 为每个演示身份启动一套服务：隔离明显，但无法验证同一 Tenant 内的项目共享，
  运维和重置成本也过高。
- 不透明 Viewer Session + 服务端身份 allowlist + 短期 JWT：在不伪装生产登录的前提下，
  复用真实 Authority、Scope 和 canonical re-authorization 链路。

## Consequences

- Chat Session、Turn 和 User Scope Memory 必须按当前 Active Principal 隔离。
- Alice 与 Bob 同属 `lazytt-memory-system-demo`，可读取同一 Project Scope；
  Carol 只属于 `external-sandbox-demo`，不能读取 LazyTT 项目记忆。
- 当前用户重置只清理该用户的 User Scope 与对话；项目重置是独立、显式且受权限保护的
  操作。
- 身份切换时必须关闭旧 SSE、清空旧 UI 状态，并拒绝带过期身份 revision 的请求。
- 该机制只用于 loopback 演示；生产必须替换为外部身份、OIDC/JWKS、正式 membership
  authority、撤权和会话安全控制。
