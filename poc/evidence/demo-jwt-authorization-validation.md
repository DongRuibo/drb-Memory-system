# JWT Principal 与跨用户隔离验证证据

验证日期：2026-07-28  
环境：`memory-service` JWT HS256 模式、本机 PostgreSQL、Mem0、合成 Tenant/User 和
合成短期令牌。未使用真实用户数据或生产 Secret。

## 配置与校验边界

本次服务启动配置：

```text
MEMORY_AUTH_MODE=jwt_hs256
MEMORY_JWT_ISSUER=https://identity.integration
MEMORY_JWT_AUDIENCE=lazytt-memory
MEMORY_TENANT_IDS=demo-tenant
MEMORY_BIND=127.0.0.1:8081
```

验签 Secret 是仅用于本地集成测试的合成值，未写入仓库。实现固定接受
`alg=HS256, typ=JWT`，并校验：

- HMAC-SHA256 签名；
- issuer 与 audience；
- `tenant_id` 属于服务 allowlist；
- `sub == user_id`；
- `iat`、可选 `nbf`、`exp`；
- token lifetime 不超过一小时；
- permission 格式与数量上限。

## HTTP 结果

使用 Alice 和 Bob 两个不同签名 Principal：

```text
无 Authorization 调用 Export        → 401
Alice 只有 memory:read 调用 Export  → 403
Alice memory:export                  → 17 records
Bob memory:export（写入前）          → 0 records
Bob memory:write                     → memory_accepted
Bob Inspect 自己的 Record            → 200
Alice Inspect Bob 的 Record          → 404
Bob memory:export（写入后）          → 1 record
Bob 的 Projection Operation          → ready
```

Bob 合成写入标识：

```text
memory_record_id = 536f4e3c-8cfd-538d-b4e7-676a9fb9e5fe
operation_id     = 15592244-5047-5853-b710-c8062b2b8662
```

第一次测试同时发现 Alice 可以读取 Bob 的 Operation 状态：Repository 当时只执行了
Tenant 过滤，没有执行 Record owner 复核。修复后重新构建和启动相同 JWT 配置：

```text
Alice GET Bob operation → 404
Bob GET own operation    → 200
```

对应 PostgreSQL 回归测试也加入了 cross-user Operation `NotFound` 断言。

## 单元与配置负例

自动测试覆盖：

- 合法短期 token 派生 Tenant/User 与 permissions；
- 缺失 Authorization；
- 被篡改签名；
- 已过期 token；
- audience 不匹配；
- Secret 少于 32 bytes 和非法配置；
- Demo auth 由主程序限制为 loopback bind。

## 尚未覆盖

- 本次 HTTP 证据未覆盖 Project/Agent；后续 PostgreSQL 集成测试已覆盖签名
  typed-scope 交集、membership removal 的 Pack 拒绝和 Project Erasure，详见
  `demo-typed-scope-validation.md`；
- 外部 membership directory、主动撤权事件和 claim freshness SLA；
- OIDC/JWKS 非对称签名、key rotation 和撤销；
- Runtime Task/Attempt ownership token；
- 管理操作双人审批、配额和审计告警；
- 动态 Tenant registry。当前 Worker 根据启动时 allowlist 每 Tenant 启动一个实例。
