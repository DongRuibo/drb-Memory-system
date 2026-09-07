# Memory ECS dev 操作流程

本包只部署 Memory Service 和 Mem0，不部署 Agent、演示页面、评测程序、PostgreSQL 或 Elasticsearch。
目标是阿里云 ECS 开发环境，发布来源为 `dev`；代码经 PR 审阅后进入 dev，不合并 main。
准备/迁移/运行就绪不等于真实 Provider 业务验收。当前包尚无目标 ECS 部署回执。

## 1. 当前发布边界

```text
Agent（外部调用方，本包不部署）→ Memory Service → Mem0 → Elasticsearch
                                                   → PostgreSQL 认证与配置库
                                                   → history 持久卷
                                                   → 聊天模型与 Embedding API
```

`MEMORY_BACKEND_MODE=mem0_primary`；不运行根目录 31 个 governed SQLx Migration。
Mem0 固定 `b357a5a1b03c299ec8229c268e63cfac0f7c6566`，使用其原生 Alembic 001～006。
数据库运行账号无 DDL 权限；迁移使用本库 owner，不把管理员密码注入常驻应用。
不使用 `deploy/mem0/sql/mem0-schema-v006.sql` 手工建表/写账本。
若既有环境已使用该历史 SQL，先另行审计现有结构、历史和归属；本包不接管未知数据库/项目。

ECS 镜像对固定上游做断言式小补丁：正确编码数据库 URL、将连接池限制为 5、
未配置时先提供认证配置接口、仅接受 Elasticsearch、配置持久化失败明确报错。
首次通过 API 配置后才初始化向量存储；配置回读同时检查脱敏 API 和数据库中的实际持久值，均不打印秘密。

仓库维护一个独立 [compose.dev.yml](../../compose.dev.yml)，使用 JSON 形式的合法 YAML，
让标准库脚本保留变量占位符并安全更新镜像，无需在 ECS 安装 YAML 库。
ECS 唯一生效文件为 `<部署目录>/compose.yml`；没有源码 checkout、服务器构建或第二份运行 Compose。
`admin` profile 复用同一 Mem0 镜像运行一次性任务，不新增第三种应用镜像。

workflow 仅引用 GitHub 官方 `actions/*`，并固定到完整 commit SHA；镜像构建、登录与推送使用 Runner 的 Docker CLI，
适配仓库的 Action 使用限制。两种镜像分别使用 GHCR `buildcache-dev` 缓存标签，
运行制品仍固定 digest；缓存是否命中以构建日志为准。临时 builder 与登录配置在退出时清理。

## 2. 先填目标登记表

| 项目 | 本包确定值／目标机器待补内容 |
|---|---|
| 仓库／分支／候选 | `lazyaix/lazytt-memory-system`；dev；实际 SHA 和两个 digest 从 Actions candidate.txt 取得 |
| ECS／工作目录 | 目标 ECS 尚待确认；独立绝对目录由 ECS_REPO_DIR 指定 |
| 硬依赖 | Mem0 PostgreSQL、Elasticsearch；真实提取与检索还依赖模型 API；不依赖 Agent、Identity 或 Trust 启动 |
| Store | 实际 PostgreSQL/Elasticsearch 版本、架构、私网地址、端口、容量、可达路径均待目标核验；PostgreSQL 仅用于认证与配置 |
| 数据库 | `lazytt_memory_mem0`；owner `lazytt_memory_mem0_owner`；runtime `lazytt_memory_mem0_runtime`；跨库 CONNECT 拒绝 |
| 结构 | Alembic head 006，5 张应用表 + alembic_version；[字段说明](database/comments.json)作为数据库注释和验收字典 |
| 业务配置 | Mem0 configure API 设置真实 Provider/索引；不预置用户、记忆或假业务记录 |
| 镜像 | Memory Service、Mem0 两个 linux/amd64 最终镜像；同一 Git revision；以 digest 拉取 |
| 网络 | Docker 自动 IPAM 的独立项目网络；通过已登记可达地址连接 Store；不改 Store 容器网络 |
| 入口 | Memory 仅宿主回环默认 18081；不映射 Mem0 端口；本包无公网入口，域名/Nginx 不适用 |
| 身份 | 强制 jwt_hs256；签发方、issuer、audience、tenant 由负责人明确；不宣称已经接入 Identity |
| 资源 | Mem0 2 CPU/2 GiB，Memory 1 CPU/512 MiB，admin 1 CPU/512 MiB；启动门禁 MemAvailable ≥4 GiB；PID/日志轮转见 Compose |
| 持久数据 | PostgreSQL、专用 ES 索引、`lazytt-memory-ecs-dev_mem0-history` 卷；备份须覆盖三者 |
| 后续环境 | staging/production 未交付，不复用 dev 配置、数据或成功状态 |

首次执行前核对既有网络和宿主机路由、Store 资源余量、18081 占用与备份位置。
预算是本包的限制，不是目标机器容量已足够的证明。不得接管归属不明的同名卷/网络。
不要为此次部署改变共享 Store 的端口、防火墙、版本或其他项目。

## 3. 准备配置和不可变制品

1. 按 [变量清单](环境变量.md)核对已有获准来源。应用文件 `/etc/lazytt/memory.env`，
   owner 临时文件 `/etc/lazytt/memory-migrate.env`，均为 root:root 0600。
   迁移阶段应用文件只需 DB 主机、端口和运行密码；owner 文件只含 `export MEM0_MIGRATION_PASSWORD='<REDACTED>'`。
   不必为了迁移先准备 JWT、模型材料。
2. 在 GitHub 设置发布连接。首次设置 `ECS_DEPLOY_ENABLED=false`：PR 合入 dev 后先测试、构建并推送镜像。
   若 ECS 连接已准备好，可设置 `true`、`ECS_DEPLOY_ACTION=prepare`，同一次 dev push 自动交付制品但不启动。
3. Actions 下载 `memory-ecs-<完整SHA>` artifact，保存 `candidate.txt` 中的 SHA 和两个 digest。
   验证 SHA 为当前已审阅 dev 候选。镜像拉取权限和 amd64/OCI revision 由 prepare 检查。
4. `workflow_dispatch` 提供 build/prepare/release，但 GitHub 通常要求 workflow 存在默认分支才显示手动入口。
   当前默认分支是 main；不得为显示按钮而擅自合并 main。dev push 路径不依赖手动按钮。

手动使用已交付 artifact 时，在管理员本机下载，临时上传其 `compose.dev.yml`、`deploy/ecs/release.py`
及所需 SQL 到 ECS 的独立临时目录。只传辅助文件，不传应用源码。
手动 prepare 需要已有获准的 GHCR 只读登录会话；正常流水线负责临时登录和清理。
以下示例在 ECS 的该临时目录中由获准管理员执行；先把 `<...>` 换为实际值，不能原样运行。
若 GitHub 已 prepare，通过同一候选直接进入数据库阶段，不重复准备。

```bash
python3 deploy/ecs/release.py prepare \
  --directory <ECS_REPO_DIR> --source compose.dev.yml \
  --revision <GIT_SHA> --memory-image <MEMORY_DIGEST_REFERENCE> --mem0-image <MEM0_DIGEST_REFERENCE>
```

prepare 只拉取镜像、验证架构/revision并更新唯一 Compose 的候选管理任务。
已有应用镜像保持原值，不启动、不迁移、不调用 configure。首次与已有服务的新迁移都使用此入口。

## 4. 首次数据库阶段：管理员明确执行

1. 在 pgAdmin 以 Store 管理员连接入口库，执行 [01_roles.sql](database/01_roles.sql)。
   在角色属性中分别设置 owner/runtime 密码，或在 psql 用 `\password` 交互输入；不把密码写进 SQL。
2. 按 [02_database.pgadmin.sql](database/02_database.pgadmin.sql) 的 A/B/C 分段操作。
   A 的 CREATE DATABASE 必须独立于事务，且只在确认不存在时执行；C 必须切换到目标库。
3. 共享 Store 管理员保存 ACL 恢复 SQL、核对既有角色后处理跨库 CONNECT。
   本包不自动撤销其他库 PUBLIC 权限；未隔离时迁移/检查会拒绝继续。
4. 在 ECS 对已 prepare 的候选执行：

```bash
python3 deploy/ecs/release.py migrate --directory <ECS_REPO_DIR> \
  --migration-config /etc/lazytt/memory-migrate.env
```

完成条件为 `MIGRATED`：原生 Alembic 升级、中文注释、本库 GRANT、head/结构/运行权限检查通过。
运行账号不能更改 `alembic_version`，没有建表/建角色/建库/超级/复制/绕过 RLS 权限。
迁移失败保留原库和已生效账本；不 stamp、DOWN、删库或重写已执行的上游 revision。
owner 文件在管理窗口结束后按管理员秘密管理规则清除或移至获准保管位置。

## 5. 首次配置与发布

补齐应用文件中的模型、索引和 JWT 配置。ES 身份仅授予指定 Memory 索引所需访问权限，
先确认索引归属。模型与 embedding 维度必须核验；本包不部署本地模型服务。
`MEM0_API_KEY` 用于受认证的 Mem0 配置入口，不能给 Agent 使用。

```bash
python3 deploy/ecs/release.py configure --directory <ECS_REPO_DIR>
python3 deploy/ecs/release.py release --directory <ECS_REPO_DIR>
python3 deploy/ecs/release.py check --directory <ECS_REPO_DIR>
```

configure：先检查数据库，再启动候选 Mem0，通过正式 configure API 写入并回读模型/索引配置。
它不启动 Memory Service，不创建业务记忆。已有索引的模型、维度或地址变化会拒绝，需单独迁移方案。
release：候选 schema 只读检查 → Mem0 更新 → 配置只读回读 → Memory 更新 → 镜像与健康检查。
普通 release 不迁移、不重新配置 Provider，不重建数据库、共享 Store 或已有 history 卷。

首次完成后，将 GitHub `ECS_DEPLOY_ENABLED=true`、`ECS_DEPLOY_ACTION=release`，
以后审阅 PR 合入 dev 自动执行测试、构建、prepare 和 release。修改环境文件需要重新发布，restart 不更新容器环境。
ECS 上传的临时 artifact 辅助文件使用后按其确切路径清理；唯一运行 Compose 和持久数据保留。

## 6. 已有数据库的新迁移及失败恢复

结构升级前将 push 自动部署关闭或切回 prepare；先审阅 Migration、备份恢复、锁/耗时、兼容窗口和暂停写入安排。
对 PostgreSQL、Elasticsearch、history 分别记录可恢复备份，验证恢复步骤；ACL 备份不能代替数据备份。
新候选 prepare 后 `admin` 服务使用新镜像，现有两个应用仍使用旧镜像。
使用同一候选 migrate → 验收 → release。升级 Mem0 upstream 时还需同步 HEAD、字典、结构检查和测试，不能只改镜像 tag。

本版本拒绝 prepare 时改变已有 Compose 拓扑/资源定义，避免丢失旧服务恢复信息。
端口、卷、网络、服务环境结构或预算变更需先补相应升级步骤；普通代码与原生迁移候选交付已提供入口。
首次新库是支持范围；未知既有 schema/项目迁入不自动接管。

发布在主机以文件锁串行。自动恢复仅在旧应用已健康、旧镜像通过现有 schema 检查且应用配置 hash 未变化时执行。
失败先停止本次应用，再恢复旧镜像及 Compose 并验证健康；不回滚数据库。
不兼容、首次发布或配置已变化时停止应用并保留数据，记录为向前修复；不会假装已恢复。
进程被强杀/主机掉电时内存中的旧 Compose 不保证存在。恢复后核对唯一 Compose、实际容器 digest、
Actions 最后阶段与候选 revision，选择已审阅兼容版本继续；不能直接 down -v。

## 7. 分层验收与回执

本机 `check` 只证明数据库、配置回读及应用健康，readiness 本身只探测 Mem0 setup-status。
它不能证明真实模型可用、记忆写入成功、租户隔离或恢复已通过。

| 层次 | 目标 ECS 必须保存的证据 |
|---|---|
| 制品/配置 | SHA、两个 digest、amd64/OCI revision、配置权限与存在性；不打印值 |
| 数据库 | head 006、表/字段/约束/注释、本库权限、跨库拒绝、备份及恢复位置 |
| 运行 | 两容器 healthy，实际镜像匹配；无 OOM/异常增长重启；部署前后 Store 状态 |
| 真实业务 | 使用获准测试租户 JWT 完成 extract → retrieve；确认模型 Provider 实际调用与存储回读 |
| 拒绝/隔离 | 缺失/伪造 JWT、未授权 tenant、跨用户读取、无 memory:forget 删除被拒绝 |
| 重复请求 | 验证相同 Idempotency-Key 的实际行为；当前 B 路径只记录摘要，不宣称已具备持久幂等去重 |
| 恢复 | 重建应用后记忆可召回；真实失败发布恢复/停止行为与数据一致性 |
| 公网 | 本包不适用；新增外部调用需求需先审阅网络、TLS 与认证边界 |

真实 Provider 业务测试会产生 API 费用并写入测试记忆，需在目标授权窗口执行。
现有 `scripts/smoke-elasticsearch-memory-service.py` 使用本地可信头，不能原样用于本包 JWT 验收。
JWT 需符合 `apps/memory-service/src/auth.rs` 的实际 claims；由获准签发方提供，不猜造生产身份。

```text
时间/时区：
目标 ECS / 环境 / Compose 项目：
PR / 发布分支 / Git SHA / Actions：
Memory digest / Mem0 digest / Store 版本：
配置修订号（不含秘密）：
数据库 / 原生迁移 / 注释 / 权限隔离：
运行就绪：通过/失败/未执行；证据：
真实业务 / 拒绝 / 幂等：通过/失败/未执行；证据：
重建持久性 / 发布失败恢复：通过/失败/未执行；证据：
公网：不适用；
备份 / 恢复验证 / 共享 Store 前后状态：
临时资源清理 / 剩余故障 / 下一步：
```

本地验证方式：`python3 -m unittest discover -s tests/ecs -v`，
`python3 tests/ecs/database_docker.py <本次Mem0镜像>`（临时 PG18，无宿主端口，自动清理）。
前者为模拟发布失败路径；后者为真实隔离数据库验证，都不是目标 ECS 或真实 Provider E2E。
容器测试还会启动隔离 Elasticsearch/Mem0，验证空库首次启动、认证 configure、脱敏回读和重启加载。
测试使用无外部出口的专用网络，结束后检查容器/网络无遗留；不会调用付费模型。
