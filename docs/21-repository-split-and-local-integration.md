# 后端与 Demo 拆仓及本地联调边界

状态：**Phase 2 compatibility window**。后端拆分分支与独立 Demo 分支均已发布，独立
构建、独立 Demo 数据库、Docker 启动和跨仓库本地联调已经验证。本仓库暂时保留 Demo
兼容副本；固定后端 release/image 并完成一轮外部下载验证后再删除历史目录。

## 目标仓库

### `lazytt-memory-system`

正式、可独立下载的 Agent 记忆后端。它拥有：

- `memory-service`；
- Memory Domain/Application/PostgreSQL/Provider/Reranker/Extractor；
- PostgreSQL 权威账本和检索投影 Migration；
- `/v1` OpenAPI、`memory-client` SDK、后端评测与运维脚本；
- 后端镜像和本地 Compose。

它不依赖 Node、Web 页面、Demo 身份或 Demo 会话表。正式 Agent 只通过 `/v1` HTTP
契约或同版本 `memory-client` 使用它。

### `lazytt-frontend-console` / Memory Demo 分支

仓库：`lazyaix-dev/lazytt-frontend-console`

分支：`feature/ruibo-dong_lazyx/memory-agent-demo`

可选下载的参考 Agent 和测试客户端。它拥有：

- `demo-web`；
- `demo-web-api`；
- `demo-agent`；
- `demo_chat` Migration、Demo OpenAPI、讲解文档和端到端测试；
- Demo Web/API 镜像和本地 Compose。

浏览器只调用 Demo Web API。Demo Web API 使用服务端 JWT 调用 Memory Service；Memory
Service JWT 不进入浏览器。

## 稳定边界

```text
Browser -> Demo Web -> Demo Web API -> Memory Service /v1
                                      -> Answer Model

Formal Agent ------------------------> Memory Service /v1
```

禁止跨仓库依赖：

- Demo 不读取后端数据库表；
- Demo 不执行后端 Migration；
- Demo 不通过相对路径引用后端源码；
- 后端不依赖 `demo-web`、`demo-web-api` 或 `demo-agent`；
- 两边不共享未版本化的环境变量文件和 Secret。

## 数据所有权

| 数据 | Owner | 可删除影响 |
| --- | --- | --- |
| Memory Record/Version/Ledger/Context Pack | Memory Service | 影响正式记忆 |
| Mem0/向量/BM25 投影 | Memory Service | 可从权威记忆重建 |
| Demo Session/Message/Turn/Event | Demo Web API | 只影响演示对话 |
| Viewer Session 和固定演示身份 | Demo Web API | 只影响当前演示登录态 |

`demo-web-api` 把模型实际使用的 `runtime_context + memory_context` 原样保存为不可变
`prompt_context_snapshot`。证据面板读取该快照，不再通过跨 Schema SQL 函数读取
`memory_core`，因此 Demo 数据库可以独立部署。

## 当前本地联调方式

后端仍沿用现有本地服务启动方式：先启动 PostgreSQL、Mem0 和 Qwen 模型网关，再运行
Memory Service。精确依赖准备步骤见根 README 的“在新机器上启动 Web 演示”。服务就绪后
应确认：

```bash
curl http://127.0.0.1:8080/health/ready
```

独立 Demo 仓库提供 Web、Demo Web API 和自己的 PostgreSQL Compose：

```bash
cp .env.example .env
# 填写 KIMI_API_KEY，并让 JWT 配置与后端一致
docker compose up -d
open http://localhost:3000
```

Demo 通过 `MEMORY_BASE_URL=http://host.docker.internal:8080` 连接宿主机后端，不依赖两个
仓库的相对目录。SDK 只在显式开启本地开发开关且目标为 Docker 的固定宿主机网关名时
允许该 HTTP 链路；任意远程 HTTP 仍被拒绝，生产环境必须使用 HTTPS。

后端全容器 Compose、固定后端镜像和 Demo 的 `bundled` profile 属于下一阶段；完成后，
普通测试人员可以只下载 Demo 仓库并由 Compose 拉起固定版本后端。

## 版本契约

- 后端 Tag、Docker image 和 `memory-client` 使用相同 SemVer；
- Demo 固定一个后端版本范围，禁止默认依赖 `main` 或 `latest`；
- `/health/ready` 继续表示依赖和模型清单可用；根接口暴露不含 Secret 的
  `service_version`、`api_version` 和 `capabilities`；
- Demo CI 必须用固定后端镜像运行契约和 Web E2E。

## 拆分顺序

1. [x] Demo 自有 Prompt Context 快照和自有 Migration；
2. [x] Demo 只通过版本化 SDK/API 使用后端，不读取后端 Schema 或内部实现；
3. [x] 独立 Demo 建立 Web/API Dockerfile、Compose、健康检查和 `.env.example`；
4. [x] 验证后端测试、Demo 独立数据库测试、容器启动和跨仓库本地联调；
5. [x] 保留本仓库为 `lazytt-memory-system`，将 Demo 发布到独立前端仓库分支；
6. [ ] 为正式后端建立全容器 Compose，发布固定 Tag/image，Demo 固定该版本；
7. [ ] 完成兼容窗口后，从本仓库移除 Demo 应用，历史 Migration 不重写。
