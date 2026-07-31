# 后端与 Demo 拆仓及本地联调边界

状态：**Phase 1 implementing**。当前仍是单一 Git 仓库，但运行时和数据库边界已经开始
按两个未来仓库收敛；在独立构建和契约测试通过前不删除历史目录。

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

### `lazytt-memory-demo`

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

## 本地联调目标

后端仓库未来提供：

```bash
docker compose up -d
curl http://127.0.0.1:8080/health/ready
```

Demo 仓库未来提供：

```bash
docker compose up -d
open http://localhost:3000
```

两个 Compose 使用同名本地网络 `lazytt-local`。Demo 通过
`MEMORY_BASE_URL=http://memory-service:8080` 连接后端，不依赖两个仓库的相对目录。
也可以让 Demo 的 `bundled` profile 拉取固定版本的后端镜像，用于只下载 Demo 仓库的
普通测试人员。

## 版本契约

- 后端 Tag、Docker image 和 `memory-client` 使用相同 SemVer；
- Demo 固定一个后端版本范围，禁止默认依赖 `main` 或 `latest`；
- `/health/ready` 继续表示依赖和模型清单可用；根接口暴露不含 Secret 的
  `service_version`、`api_version` 和 `capabilities`；
- Demo CI 必须用固定后端镜像运行契约和 Web E2E。

## 拆分顺序

1. Demo 自有 Prompt Context 快照和自有 Migration；
2. Demo 仅依赖公开 `memory-client`/OpenAPI，不依赖内部 Domain crate；
3. 后端和 Demo 各自建立 Dockerfile、Compose、健康检查和 `.env.example`；
4. 在当前仓库验证后端独立测试、Demo 独立数据库测试和组合 E2E；
5. 保留当前仓库为 `lazytt-memory-system`，将 Demo 目录迁入新仓库；
6. 发布固定后端 Tag/image，Demo 固定该版本；
7. 一个兼容窗口后，从后端当前源码树移除 Demo 应用，历史 Migration 不重写。
