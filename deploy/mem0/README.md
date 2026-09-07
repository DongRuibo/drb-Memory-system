# Mem0 项目侧 HTTP 入口

Memory Service → 项目侧 HTTP 入口 → 原版 Mem0 SDK → Elasticsearch。
SDK 固定 `b357a5a1b03c299ec8229c268e63cfac0f7c6566`，未修改、删除或猴子补丁上游源码。
`gateway.py` 是本项目维护的入口，替代写死默认向量后端的上游 `main:app`；它不是官方 REST 镜像的配置开关。

## 接口与认证边界

提供当前 Memory adapter 使用的接口：`POST /memories`、`POST /search`、
`GET/DELETE /memories/{id}`、`GET/POST /configure`、`GET /auth/setup-status`，另有 `/health/ready`。
内部业务/配置接口只接受独立 `MEM0_API_KEY`（映射为 `ADMIN_API_KEY`），不对 Agent 发放。
Memory 对 Agent 的 JWT、租户、用户和删除权限仍由原有 Rust 服务负责；Agent 不能直接访问 Mem0。
不提供原版 dashboard、Mem0 用户登录、JWT 刷新或每用户 API Key 管理；`MEM0_JWT_SECRET` 已撤下。
共享内部 Key 是可信 Memory 服务的凭据，不是各最终用户的身份；用户隔离由 Memory 生成的 user_id 和 ES filters 保证。

## 启动与配置

普通启动只读验证 PostgreSQL schema，读取 `settings.config_overrides`。
空配置时只启动管理接口，不初始化任何向量后端，`/health/ready` 返回 503。
流水线通过受认证的 configure API 提交 ES/模型配置：只接受 Elasticsearch 和 OpenAI-compatible 模型。
先构造 SDK，再提交持久配置，数据库提交失败不会激活新配置。GET 配置隐藏密码和 API Key。
已保存 ES 配置暂不可用时仍开放认证管理入口，业务 readiness 为 503；流水线 ensure 可重试初始化或更新凭据。
已保存配置的启动直接调用官方 `Memory.from_config`；不合并任何默认 PG 字段。

`/auth/setup-status` 允许空库配置阶段的容器健康等待；配置成功后也探测 ES 和 PostgreSQL。
`/health/ready` 要求已配置及两项存储可达。模型 API 的真实有效性仍需独立业务验收。
模型、维度或索引变化被拒绝，必须先审阅数据迁移；日常配置相同不写数据库。

PostgreSQL 只保存配置。为保留原生迁移历史和既有 schema，原版的用户/认证等表继续由原生 Alembic 管理，
本入口不创建假用户、不使用这些旧认证接口。SQLite history 使用项目持久卷，ES 保存向量。
`storage.py` 使用 SQLAlchemy URL 对象和 3+2 连接池；密码不拼到 URI，不修改上游 db.py。
`migration/env.py` 使用同一连接实现，Alembic `version_locations` 直接引用原版 `/app/alembic/versions`；
原版 001～006 的内容和迁移账本不变。镜像构建从原版 models 生成结构清单，不在运行时导入其 REST 数据库连接。

## 构建与验证

```bash
./scripts/build-mem0-elasticsearch.sh
python3 tests/ecs/database_docker.py lazytt-mem0-api-elasticsearch:b357a5a1-es-gateway-v1
```

本地与 CI 共用 `deploy/mem0/Dockerfile`，附带 ES 客户端、项目入口和管理工具。
构建核对 169 个上游 Python 文件摘要；不使用原版 main:app，不移除第三方包内部未使用的插件。
隔离测试使用临时 PG18、ES8.11.3 和确定性的模型替身，无宿主端口、无外部网络出口，结束自动清理。
它覆盖特殊字符密码、原生迁移、权限、首次配置、HTTP 推理写入/搜索/读取/删除、重启加载和拒绝路径。
模型替身不证明真实模型 Provider 可用，也不证明已连接目标 ECS 的 PG/ES。

完整流程见 [本地](../local/README.md) 和 [ECS dev](../ecs/README.md)。
