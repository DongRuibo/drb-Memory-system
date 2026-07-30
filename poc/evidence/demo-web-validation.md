# Demo Web 本地验收

验证日期：2026-07-30

## 范围

验证多用户 Demo Web、请求级 Active Principal、服务端短期 JWT、Chat 隔离、
Project Scope 共享、Event Time、Structured Content、版本与 Erasure 的本地真实链路。
Web、Demo Web API 和 Memory Service 均只绑定 loopback。

## 三用户 HTTP 验收

使用同一个 HttpOnly Viewer Session 依次切换身份，返回的非敏感摘要：

```text
Alice: revision=1, project=lazytt-memory-system-demo
Bob:   revision=2, project=lazytt-memory-system-demo, can_reset_project=false
Bob reads Alice session: HTTP 404 NOT_FOUND
Bob canonical memories: 1 shared project record
Carol: revision=3, project=external-sandbox-demo, canonical memories=0
request with stale revision=2: HTTP 409 STALE_DEMO_IDENTITY
Alice again: revision=4, Alice session restored, can_reset_project=true
```

现有 shared project canonical export 还验证了：

```text
memory_type=semantic_fact
scope.kind=project
version_no=1
structured_content.schema_name=lazytt.semantic_fact
structured_content.schema_version=1
```

本轮没有把新编写的演示消息发送给外部 Kimi。真实外部模型链继续以此前独立验收为准；
本轮使用本地数据库集成测试覆盖 Event Time 和 Structured Content。

## 工程验证

```text
npm test && npm run lint                                      PASS
cargo fmt --all -- --check                                   PASS
cargo clippy --workspace --all-targets -- -D warnings        PASS
cargo test --workspace                                       PASS*
demo-web-api unit tests                                      3 PASS, 1 DB test ignored
ChatStore PostgreSQL isolation test                          PASS
Correction + Structured Content PostgreSQL test             PASS
Temporal Event Time + canonical authorization test           PASS
Typed Scope + Project Erasure PostgreSQL test                PASS
OpenAPI YAML parse                                            PASS
bash -n demo-web-stack.sh demo-web.sh run-memory-service...  PASS
```

`*` 首次 workspace 测试在受限沙箱中仅有 10 个需要绑定 loopback mock server 的测试因
`Operation not permitted` 失败；在允许 loopback 的环境重跑 `demo-agent`、
`kimi-reranker`、`mem0-adapter` 后全部通过。

## 安全与发布边界

- 浏览器只持有不透明 Viewer Session，不接收 Memory JWT、Kimi Key 或数据库凭据；
- Demo Web API 只允许服务端 allowlist 身份，并为每个请求签发 15 分钟 JWT；
- Alice/Bob/Carol 是固定演示 fixture，不是生产账户；
- 当前用户 reset 与共享项目 reset 分离，项目 reset 受显式能力控制；
- 页面使用 identity revision 丢弃或拒绝切换后的旧请求与旧 SSE 状态；
- 应用依赖本机 PostgreSQL、Mem0、Qwen 和 Memory Service，因此没有发布到公网 Sites；
- 生产仍需要外部身份、OIDC/JWKS、正式 membership authority、撤权、TLS 与网关。
