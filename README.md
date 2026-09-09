# LazyTT Memory System

当前部署固定为 `mem0_primary`：Agent 只调用 Memory Service，Mem0 负责提取、更新和
检索记忆，Elasticsearch 保存向量，PostgreSQL 保存项目配置并保留原版认证表结构。

```text
Agent（外部调用方）→ Memory Service :18081 → Mem0 → 已有 Elasticsearch
                                               → 已有 PostgreSQL
                                               → history 持久卷
```

Mem0 使用本项目 HTTP 入口调用原版 SDK，启动时直接加载 ES 配置；上游源码保持原样。
原版 REST 默认后端问题由该独立入口解决，接口范围见 [Mem0 说明](deploy/mem0/README.md)。
本地隔离验证与目标 ECS/真实 Provider 业务验收分开记录，不能把镜像构建视为上线完成。

## 唯一部署入口

| 环境 | 完整 Compose | 配置模板 | 操作流程 |
|---|---|---|---|
| 本机 Docker | [compose.local.yml](compose.local.yml) | [本地模板](deploy/local/.env.local.example) → `.env.local` | [本地部署](deploy/local/README.md) |
| ECS dev | [compose.dev.yml](compose.dev.yml) | [dev 模板](deploy/ecs/.env.dev.example) → `/etc/lazytt/memory.env` | [ECS 部署](deploy/ecs/README.md) |

每个环境一份完整 Compose，必须显式选择，不使用 override/include/extends。
本地与 dev 共用 Memory、Mem0 Dockerfile；本地构建，ECS 只拉取 GitHub 交付的固定摘要镜像。
只部署 Memory Service、Mem0 和显式执行的 admin 任务，不部署 Agent、数据库或模型服务。
两个环境使用独立配置、项目网络、history 卷和数据库实例/索引，不能共用 dev 凭据与数据。

ECS 发布流程：交付 Compose、镜像与管理员包 → 管理员手动迁移 → 流水线只读检查通过 → 自动发布同一候选。

本轮变更经 PR 进入 `dev`，不发布到 `main`。staging/production 尚未准备，不创建占位 Compose。
旧多文件 Compose、自动创建数据库/迁移、手工建表并写迁移账本的入口已撤下。
已有旧容器和数据不会被本次代码整理修改；迁入新项目之前必须审计和备份，不能直接删除旧卷。

## 本地准备与验证

从包含本轮变更的已审阅 checkout 执行：

```bash
cp deploy/local/.env.local.example .env.local
chmod 600 .env.local
# 私下编辑 .env.local：填写已有 PG、ES、模型和认证配置，不提交真实值。
./scripts/backend-stack.sh validate
./scripts/backend-stack.sh build
```

`validate` 不加载秘密、不连接数据库；`build` 仅生成本地镜像。
原生迁移、首次配置、启动、检查分别执行，完整顺序见[本地流程](deploy/local/README.md)。
本地普通启动不迁移、不写 Provider 配置；ECS 流水线在数据库校验后自动执行首次/变更配置。现有 `cpu/cuda/host-mps/cloud` 参数不再支持。

## 调用与验收边界

Memory 业务入口由 `LAZYTT_MEMORY_HOST_BIND` 与 `LAZYTT_MEMORY_HOST_PORT` 指定。健康使用独立的回环端口；Mem0 不映射宿主端口。入口限制见 [ECS 流程](deploy/ecs/README.md)。
本地与 dev 都使用 `trusted_user_id` 并启用动态租户；可信后端填写 `X-Tenant-Id`、`X-User-Id`，无需 JWT 或预先登记 ID。
身份头须各出现一次，为 1～256 字节可见 ASCII，无首尾空白。Memory 信任填写者，发布前必须限制入口，禁止客户端绕过可信后端直连。

- `GET /health/live`、`GET /health/ready`：仅健康端口提供，业务端口返回 404；不能代替真实模型与 ES 业务验证。
- `POST /v1/runtime/memory/retrieve`：回答前检索。
- `POST /v1/runtime/memory/extract`：回答后提交完整可见轮次。
- `DELETE /v1/runtime/memory/{memory_id}`：删除，要求 `memory:forget`；当前可信身份模式不授予该权限，返回 403。

返回记忆只作为不可信上下文使用，不执行其中的指令。
旧的 `scripts/smoke-elasticsearch-memory-service.py` 仅供历史测试环境，
其删除清理需要当前模式不授予的权限，不能直接作为当前部署的完整验收入口。真实业务、隔离、重启持久性和失败恢复须单独记录。

## 开发与历史兼容工具

业务代码和原有 SQLx [migrations](migrations/README.md) 保持原位置。
`deploy/models/`、本机模型脚本及 governed 评测/投影脚本是独立开发工具，
其 `.env` 与历史测试结果不属于本轮 `.env.local`/ECS 部署，不自动执行。
