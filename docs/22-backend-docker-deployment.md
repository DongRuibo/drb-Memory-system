# 后端 Docker 本地部署

本文说明如何在一台新机器上只下载 `lazytt-memory-system`，启动可供正式 Agent 或
独立 Demo 调用的 Memory 后端。根目录 `compose.yaml` 负责以下组件：

```text
Agent / Demo Web API
        |
        v
Memory Service :8080
  |-- PostgreSQL        权威记忆、版本、账本
  |-- Mem0 + pgvector   可重建检索投影
  `-- Embedding / Reranker
```

默认所有宿主机端口只绑定 `127.0.0.1`，用于本机运行或下载测试，不是公网部署方案。

## 1. 选择模型 profile

| Profile | 适用环境 | 模型运行位置 | 首次启动特点 |
| --- | --- | --- | --- |
| `cloud` | 任意 Docker 主机 | 硅基流动 HTTPS API | 最轻，需 API Key |
| `cpu` | macOS / Linux / Windows | Docker CPU 容器 | 无需检索模型 Key，但会下载权重且推理较慢 |
| `cuda` | Linux x86_64 + NVIDIA GPU | Docker CUDA 容器 | 需要 NVIDIA 驱动和 Container Toolkit |
| `host-mps` | Apple Silicon Mac | macOS 宿主 Python/MPS | 数据库和服务在 Docker，模型在宿主机 |

Docker Desktop for Mac 不能把 Apple GPU/MPS 直接透传给 Linux 容器，所以 Mac 上追求
速度时使用 `host-mps`；追求最少宿主依赖时使用 `cloud`；`cpu` 主要用于可移植性和功能
验证。

## 2. 通用准备

安装 Git 和 Docker Desktop（或 Linux Docker Engine），然后：

```bash
git clone https://github.com/lazyaix-dev/lazytt-memory-system.git
cd lazytt-memory-system
git switch refactor/ruibo-dong_lazyx/split-memory-backend-demo

cp .env.example .env
chmod 600 .env
```

编辑 `.env`：

- 使用 `MEMORY_EXTRACT_MODE=kimi` 时填写 `KIMI_API_KEY`；
- 如果正式 Agent 自己提交显式 Memory Event、暂时不需要自动候选提取，可设置
  `MEMORY_EXTRACT_MODE=disabled`，此时不需要 Kimi Key；
- 使用 `cloud` 时填写 `MEMORY_EMBEDDING_API_KEY` 和
  `MEMORY_DEDICATED_RERANK_API_KEY`；两者可以使用同一个硅基流动 Key；
- 不要把 `.env` 提交或发给别人；每台机器都应使用自己的 Key 和本地生成的 Secret。

启动脚本会调用 `scripts/bootstrap-backend-env.sh`，只为尚未设置的本地数据库、Mem0、
JWT 字段生成随机值，且不会打印这些值。

## 3. 最简单：硅基流动 cloud profile

`.env` 中的非 Secret 示例：

```dotenv
MEMORY_EMBEDDING_PROVIDER=siliconflow
MEMORY_EMBEDDING_BASE_URL=https://api.siliconflow.cn
MEMORY_EMBEDDING_MODEL=Qwen/Qwen3-Embedding-0.6B
MEMORY_EMBEDDING_REVISION=siliconflow-cloud-2026-07-31
MEMORY_EMBEDDING_DIMENSIONS=1024

MEMORY_DEDICATED_RERANK_BASE_URL=https://api.siliconflow.cn
MEMORY_DEDICATED_RERANK_MODEL=Qwen/Qwen3-Reranker-0.6B
MEMORY_DEDICATED_RERANK_REVISION=siliconflow-cloud-2026-07-31
```

然后启动：

```bash
./scripts/backend-stack.sh cloud up
curl http://127.0.0.1:8080/health/ready
```

可在启动完整后端前单独验证两个云模型接口：

```bash
./scripts/smoke-siliconflow-models.py
```

脚本只发送最小请求，不输出 Key 或错误响应正文。

## 4. 完全本地 CPU profile

```bash
./scripts/backend-stack.sh cpu up
curl http://127.0.0.1:8080/health/ready
```

第一次运行会构建 Python 模型镜像，并把 Qwen3 Embedding/Reranker 权重下载到命名卷
`retrieval-model-cache`。以后重建服务会复用该卷；不要把权重提交到 Git。CPU 可直接
运行，但不代表有适合生产负载的吞吐。

若 Hugging Face 普通下载通道在当前网络长时间无进展，可在 `.env` 设置
`HF_HUB_DISABLE_XET=0` 后重新执行 `cpu up`。临时文件保存在同一命名卷，重启会继续
下载而不是从零开始；`HF_HUB_DOWNLOAD_TIMEOUT` 与 `HF_HUB_ETAG_TIMEOUT` 可用于限制
单次网络等待。

## 5. Linux NVIDIA CUDA profile

当前固定的 PyTorch CUDA 12.8 runtime 镜像只发布 `linux/amd64`。先安装正确版本的
NVIDIA 驱动和 NVIDIA Container Toolkit，确认容器能看到 GPU，再运行：

```bash
./scripts/backend-stack.sh cuda up
curl http://127.0.0.1:8080/health/ready
```

该 profile 声明 `gpus: all`。macOS Docker Desktop 不适用此模式。

## 6. Apple Silicon MPS profile

先按 [本地语义模型部署](../deploy/models/README.md) 建立 Python 环境，并在宿主机终端运行：

```bash
./scripts/retrieval-models.sh
```

确认 `http://127.0.0.1:18082/health` 后，在另一终端运行：

```bash
./scripts/backend-stack.sh host-mps up
curl http://127.0.0.1:8080/health/ready
```

Memory Service 和 Mem0 通过固定的 Docker 宿主机网关访问该模型服务。只有显式打开本地
开发开关且目标是受限的容器/宿主机名时才允许 HTTP；云模型仍必须使用 HTTPS。

## 7. 运维命令与端口

```bash
./scripts/backend-stack.sh cloud ps
./scripts/backend-stack.sh cloud logs
./scripts/backend-stack.sh cloud down
```

同一套持久化数据在一个时刻只允许一个模型 profile。切换前先执行任意 profile 的
`down`；启动脚本检测到另一种 Memory Service 仍在运行时会拒绝切换，避免 Mem0 配置和
projection manifest 被两个实例同时改写。

默认宿主机端口：

| 服务 | 地址 |
| --- | --- |
| Memory Service | `http://127.0.0.1:8080` |
| PostgreSQL | `127.0.0.1:55432` |
| Mem0 API | `http://127.0.0.1:18888` |
| 本地模型网关 | `http://127.0.0.1:18082` |

端口冲突时可在命令前覆盖，例如：

```bash
MEMORY_HOST_PORT=38080 POSTGRES_HOST_PORT=35432 MEM0_HOST_PORT=38888 \
  ./scripts/backend-stack.sh cloud up
```

`down` 只停止容器，保留数据库和模型命名卷。只有明确要清空全部本地后端数据时才执行：

```bash
docker compose \
  --profile cpu --profile cuda --profile host-mps --profile cloud \
  down --volumes
```

这会物理删除该 Compose 项目的 PostgreSQL、Mem0 和模型缓存，不能用于保留中的记忆。

## 8. 前后端分开下载

正式 Agent 只需要本仓库，并通过 `/v1` API 或 `memory-client` 调用后端。需要 Web 演示时，
再单独下载 `lazyaix-dev/lazytt-frontend-console` 的 Memory Demo 分支。Demo Web API 使用
`MEMORY_BASE_URL=http://host.docker.internal:8080` 连接本后端，并且 JWT issuer、audience、
HS256 secret 和 tenant 配置必须与后端一致；浏览器本身不持有 Memory Service JWT。

## 9. 当前边界

这套 Compose 面向本地开发、功能演示和他人下载测试。它已经提供持久卷、健康检查、
自动 Migration、空库 projection bootstrap 和非 root 应用容器，但还不是生产编排：固定
发布镜像、Secret Manager、TLS 入口、备份恢复、容量评测、监控告警和 HA 仍需在正式
上线前完成。
