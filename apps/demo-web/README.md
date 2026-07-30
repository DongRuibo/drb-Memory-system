# LazyTT Memory Agent Web

这是面向产品和管理人员的本地交互式演示。界面采用常见的大模型产品布局：左侧是
对话，中间是真实 Kimi 流式聊天，右侧是“它记住了什么”。

推荐按四个按钮依次演示一条连续业务链：

```text
自然认识用户与项目 → 新对话按需带回 → 项目事实自动更新 → 指定经历立即忘记
```

第一步不使用“请记住”等口令。Kimi Extractor 最多生成 8 个受限候选；偏好、用户
事实和经历进入 User Scope，明确且置信度不低于 0.90 的项目事实进入
`DEMO_PROJECT_ID` 对应的 Project Scope。普通记忆自动保存并提供撤销；同一
semantic key 出现明确新值时，系统自动追加版本、切换当前值并显示结果提示。

浏览器不读取 Kimi API Key，也不直接访问 PostgreSQL 或 Mem0。`demo-web-api` 负责
服务端编排；对话数据位于 PostgreSQL `demo_chat`，正式记忆位于 `memory_core`。

## 启动

先按 [`deploy/models/README.md`](../../deploy/models/README.md) 启动 PostgreSQL、Mem0、
Qwen3 Embedding/Rerank 网关和 Memory Service，再从仓库根目录执行：

```bash
./scripts/demo-web.sh
```

打开 `http://localhost:3000`。默认端口如下：

| 服务 | 本机端口 |
| --- | ---: |
| Web | 3000 |
| Demo Web API | 8090 |
| Memory Service | 8080 |
| Qwen3 模型网关 | 18082 |
| Mem0 | 18888 |
| PostgreSQL | 55432 |

Memory Service 和 Demo Web API 必须使用相同的 `DEMO_USER_ID` 与 `DEMO_PROJECT_ID`。
脚本默认项目为 `lazytt-memory-system-demo`；重置演示会同时物理清理隔离的用户 Scope、
项目 Scope 和演示对话。

Mem0 不再占用代理软件常用的 `8888`。如需修改 Web API 地址，在构建前设置
`NEXT_PUBLIC_DEMO_API_URL`；如需修改 API 监听地址，设置 `DEMO_WEB_API_BIND`，但
本演示服务会拒绝绑定非 loopback 地址。

## 验证

```bash
cd apps/demo-web
npm run lint
npm test
```

完整讲解脚本见 [`docs/demo-web-presentation-plan.md`](../../docs/demo-web-presentation-plan.md)，
HTTP 契约见 [`docs/api/demo-web-openapi.yaml`](../../docs/api/demo-web-openapi.yaml)。当前
应用是本地受控演示，不应在缺少正式认证、TLS 和公网 API 网关时作为多用户产品发布。
