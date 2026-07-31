# LazyTT Memory Agent Web

这是面向产品、研发和管理人员的本地交互式演示。页面用一条真实链路展示四种正式记忆
类型、Typed Scope、Event Time、Structured Content、不可变版本、Context Pack 证据和
Forget/Erase。

每轮由服务端生成唯一的当前时间锚点，和被选中记忆的 Event Time 一起固化到 Context
Pack。最终提示词会收到 `runtime_context + memory_context`；证据抽屉直接展示同一份 Pack
快照，不再用当前记忆状态重建模型输入。

## 六步演示

```text
自然形成记忆
→ 查看类型、范围、事件时间和结构化内容
→ 新对话按需回忆
→ 按事件时间回忆
→ 项目事实生成新版本
→ 指定经历立即忘记
```

四种正式类型是 `preference`、`profile_fact`、`semantic_fact` 和
`episodic_summary`；`user`、`project`、`tenant`、`agent` 是独立的可见范围，不是
第五类记忆。

## 演示用户

页面提供三个服务端 allowlist 身份：

| 身份 | 项目 | 预期行为 |
| --- | --- | --- |
| Alice | LazyTT 项目 | 个人记忆隔离；可重置共享项目 |
| Bob | LazyTT 项目 | 看不到 Alice 的个人记忆；可共享 LazyTT 项目事实 |
| Carol | 外部沙盒项目 | 看不到 Alice/Bob 的个人记忆和 LazyTT 项目事实 |

浏览器只持有不透明 HttpOnly Viewer Session。`demo-web-api` 根据当前固定身份签发短期
HS256 JWT 调用 Memory Service；JWT 不进入浏览器。该选择器不是生产登录系统。

Web 和 `demo-web-api` 属于可选 Demo/测试客户端，不属于正式 Memory Service。拆仓
边界与本地联调约定见
[`docs/21-repository-split-and-local-integration.md`](../../docs/21-repository-split-and-local-integration.md)。

## 推荐启动

先启动 PostgreSQL、Mem0 和 Qwen3 Embedding/Rerank 网关，并在当前 shell 中提供
`KIMI_API_KEY`。然后从仓库根目录执行：

```bash
./scripts/demo-web-stack.sh
```

脚本生成本次进程使用的临时 JWT secret，以相同 issuer/audience 启动 Memory Service
和 Demo Web API，再启动 Web。打开 `http://localhost:3000`。

若 Memory Service 已由你以相同 JWT 配置启动，也可直接运行：

```bash
MEMORY_JWT_HS256_SECRET='至少 32 字节的本地临时值' ./scripts/demo-web.sh
```

Demo 对话可通过 `DEMO_DATABASE_URL` 使用独立 PostgreSQL 数据库；为兼容当前单仓启动，
未设置时暂时回退到 `DATABASE_URL`。

默认端口：

| 服务 | 本机端口 |
| --- | ---: |
| Web | 3000 |
| Demo Web API | 8090 |
| Memory Service | 8080 |
| Qwen3 模型网关 | 18082 |
| Mem0 | 18888 |
| PostgreSQL | 55432 |

“重置当前用户”只清理当前用户的 User Scope 和对话；“重置共享项目”是独立操作，仅
Alice 可执行，并会影响同项目成员。

## 验证与边界

```bash
cd apps/demo-web
npm run lint
npm test
```

HTTP 契约见
[`docs/api/demo-web-openapi.yaml`](../../docs/api/demo-web-openapi.yaml)，讲解路线见
[`docs/demo-web-presentation-plan.md`](../../docs/demo-web-presentation-plan.md)。
当前应用默认只允许 loopback；显式容器模式也必须由 Compose 仅发布到宿主机 loopback。
它是本地真实业务演示和集成参考；没有正式认证、TLS、OIDC/JWKS、撤权和公网网关时
不得作为多用户产品发布。
