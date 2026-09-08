# Memory 本地部署流程

唯一文件为根目录 `compose.local.yml`，唯一包装命令为 `scripts/backend-stack.sh`。
本地与 ECS 共用业务 Dockerfile、数据库管理和配置回读代码；本地不执行 ECS 的
GHCR/SSH 发布，也不要求 root。实际配置为当前用户拥有的普通文件、权限 `0600`。

Mem0 使用项目侧 HTTP 入口直接加载 ES 配置，保持上游源码不变；
接口与认证范围见 [Mem0 说明](../mem0/README.md)。本地顺序和目标 ECS 验收分开，
不把隔离测试结果视为现有数据库或真实 Provider 已经验证。

## 1. 外部依赖与配置

PostgreSQL、Elasticsearch 和模型 API 必须已存在。本仓库不启动或升级它们。
容器通过实际可达的主机地址/发布端口连接；Docker Desktop 上宿主机服务可使用
`host.docker.internal`，容器里的 `127.0.0.1` 指容器自身。无需把 Store 加入本项目网络。
ES HTTPS 必须有有效证书；不自动关闭证书验证。

```bash
cp deploy/local/.env.local.example .env.local
chmod 600 .env.local
```

私下编辑 `.env.local`，使用模板的 Bash `export NAME='value'` 格式。
所有变量与 dev 同名，用途和实际消费者见[环境变量清单](../ecs/环境变量.md)；
本地存放位置改为 `.env.local`，不能复制 dev 真实值。该文件是受信任的可执行 Bash
配置，只允许管理员本人编辑，不加载下载或未经审阅的脚本。

数据库固定 `lazytt_memory_mem0`，owner/runtime 分别为 `lazytt_memory_mem0_owner`、
`lazytt_memory_mem0_runtime`。本地应使用独立 PG 实例，不能连接 dev 的同名库；
如共享实例无法满足这个隔离要求，先规划库命名与管理代码的变更，不复用 dev 数据。
ES 使用本地专用索引，两个认证密钥独立且至少 32 字节。填写已有合法凭据，脚本不生成数据库密码。

## 2. 验证与构建

要求 Docker Compose v2（支持 `up --wait`）、Python 3 和 Bash。

```bash
./scripts/backend-stack.sh validate
./scripts/backend-stack.sh build
```

validate 使用空环境解析本地 Compose，不读取 `.env` 或 `.env.local`，不访问数据库。
build 串行构建 Mem0、Memory，均不启动。Mem0 构建会核对上游 Python 源码摘要。
Memory 使用 `apps/memory-service/Dockerfile`，Mem0 使用 `deploy/mem0/Dockerfile`；
不会创建另一份 Compose。构建标签为 `lazytt-memory-service:local` 和
`lazytt-mem0-api-elasticsearch:b357a5a1-es-gateway-v1`，本地按主机架构构建。

## 3. 管理员显式迁移

先按 [数据库角色](../ecs/database/01_roles.sql) 和
[建库步骤](../ecs/database/02_database.pgadmin.sql) 在**本地目标 PG** 创建库/角色。
它们只初始化身份和数据库，表必须通过原生 Alembic 创建。
共享实例的跨库权限需管理员核对并单独处理；不要自动撤销其他系统权限。

在忽略的 `secrets/` 目录创建当前用户拥有的 `0600` 文件 `local-migrate.env`，仅包含：

```bash
export MEM0_MIGRATION_PASSWORD='<REDACTED>'
```

运行密码留在 `.env.local`，owner 密码只进入一次性 admin 容器：

```bash
./scripts/backend-stack.sh migrate --migration-config secrets/local-migrate.env
```

迁移只要求 PG 主机、端口和运行密码，不要求先填模型或 JWT。
原生 Alembic 升级 → 注释/权限 → 运行身份结构检查；成功为 `LOCAL_MIGRATE_OK`。
失败停在原库调查，不删库、不 stamp、不手工补迁移账本。
管理窗口结束后将 owner 文件移至获准秘密保管位置或按其管理规则清除。

## 4. 首次配置、日常启动与检查

填写完整 `.env.local` 并完成数据库阶段后执行：

```bash
./scripts/backend-stack.sh configure
./scripts/backend-stack.sh up
./scripts/backend-stack.sh check
./scripts/backend-stack.sh ps
```

configure：检查数据库 → 只启动 Mem0 → 经认证接口写入并回读 ES/模型配置。
up：数据库只读检查 → 等待 Mem0 → 配置只读回读 → 启动并等待 Memory 健康。
两者均不运行迁移；up 不写配置、不构建或拉取镜像。check 不创建业务记录，
校验数据库、已保存配置和运行容器镜像/健康；不代表真实 Provider 或 ES 业务成功。
程序隐藏可能包含秘密的子命令输出，失败只报告阶段；私下排查，不公开原始日志或配置。

变更模型、维度、索引或已有库必须另行制定迁移和备份计划。常规更新先 build，再 up；
本地脚本没有 ECS 的镜像回滚机制，失败保留数据并停止推进，按已验证备份/候选向前修复。

```bash
./scripts/backend-stack.sh down
```

down 只操作 `lazytt-memory-local` 项目，保留 history 卷；不加 `-v`、不删除共享数据。
迁入前应先确认旧项目的容器、卷与数据归属，单独完成切换，本脚本不自动接管旧项目。

## 5. 资源与验收

- Memory：1 CPU/512 MiB；Mem0：2 CPU/2 GiB；admin：1 CPU/512 MiB。
- 常驻服务 `restart: always`，PID 上限 256，停止等待 45 秒；日志每文件 10 MiB、最多 3 份。
- 仅 Memory 暴露 `127.0.0.1:18081`（可配置端口）；Mem0 不映射端口；Agent 不部署。
- 网络为 Docker 自动 IPAM；持久卷为 `lazytt-memory-local_mem0-history`。

执行前人工核对 Docker 可用容量、网络/端口占用和数据库/ES/history 备份；本地不使用
ECS 的 `/proc/meminfo` 容量门禁。不要因 Compose 资源上限存在就认定宿主容量足够。
真实 JWT 的提取/检索、未授权请求、用户隔离、重建持久性需分别验收；付费模型与真实数据
测试在获准窗口执行。本次部署文件整理没有连接现有数据库，也没有该层成功回执。
