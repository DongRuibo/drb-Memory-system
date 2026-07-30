# Typed Scope、Multi-type Context 与 Project Erasure 验证证据

验证日期：2026-07-28  
环境：本机 PostgreSQL，合成 Tenant/User/Project/Agent identity，不含生产数据。

## 验证结论

- Principal authority 同时支持 Tenant、User、Project 和 Agent 四类 scope；
- 不同 scope 使用不同 opaque projection namespace；
- Inspect、Operation、Export、Search canonical recheck 与 Context Pack 都取
  “请求 selector ∩ 已签名 authority”，请求不能扩大权限；
- Context Pack 支持 `preference`、`profile_fact`、`project_fact` 和
  `episodic_summary`，映射到受控 section 与固定 inclusion reason；
- Project 成员可以使用 `project_fact` 编译 Context Pack；使用不含该 Project 的
  新 authority 消费旧 Pack 时返回 Conflict；
- 未获 Project scope 的同 Tenant 用户看不到 Record、Operation、Export 或命中；
- 已授权 Project Erasure 立即使记录不可读，擦除正文，生成 projection delete，
  并在删除 Receipt 后进入 `complete`；
- Erasure Request 中保存的是 `sha256:` target reference，不持久化 raw Project ID。

核心回归：

```text
typed_scopes_intersect_authority_and_project_erasure_is_tracked
→ passed

cargo test --workspace
→ 41 passed, 0 failed, 7 PostgreSQL tests intentionally ignored

TEST_DATABASE_URL=postgres://... cargo test -p memory-postgres \
  -- --ignored --test-threads=1
→ 7 passed, 0 failed
```

## 实现边界

生产写入 API 当前仍只接受显式的 User `preference`；本测试通过受控数据库 fixture
建立 Project/Agent 类型记录，用来验证 read/governance authority 链。项目事实的
正式提取、审核和写入策略仍需在 governed typed ingestion 阶段实现。

JWT 中的 `tenant_scope`、`project_ids` 和 `agent_ids` 是签发时刻的 authority
snapshot。当前尚未连接外部 membership directory、主动撤权事件、OIDC/JWKS 或
key rotation；短期 token 和 Consume 时 authority snapshot 复核不是完整的生产
撤权方案。
