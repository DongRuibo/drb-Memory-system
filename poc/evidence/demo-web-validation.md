# Demo Web 本地验收

验证日期：2026-07-28

## 范围

验证面向非技术观众的本地 Web 演示台和 Rust `demo-web-api` 桥接。浏览器不读取
Kimi Key，接口仅绑定 loopback。完整链路使用本地确定性 Chat Completions 假服务，
避免本轮额外消耗外部模型额度。

## 结果

Web API 返回的非敏感摘要：

```text
memory_record_id: 5959c623-62b2-5892-83a4-f9aeecfbe473
with-memory pack: 239c2ca4-f656-5ddc-ad08-e9a032512cc0
with-memory count: 1
forget revision: 1
without-memory pack: fc5549bf-0bdf-55df-bda2-71e6b066ab8f
without-memory count: 0
delete status: deleted
```

工程验证：

```text
npm run build                 PASS
npm test                      PASS
npm run lint                  PASS
cargo fmt --all -- --check   PASS
cargo clippy --workspace
  --all-targets -- -D warnings
                              PASS
cargo test --workspace        PASS
bash -n scripts/demo-web.sh   PASS
long API-key scan             NO MATCH
```

真实 Kimi 链路此前已通过服务端 Context Pack 改造后的独立验收；详见
[`demo-agent-v1-kimi-validation.md`](./demo-agent-v1-kimi-validation.md)。

## 安全边界

- Web 前端和桥接服务只在本机运行；
- API Key 继续由 Rust demo-agent 的安全 dotenv 解析器读取；
- 页面只获得 DemoReport，不获得凭据；
- 并发真实演示由单飞锁拒绝；
- 当前没有生产认证，因此没有发布到公网。
