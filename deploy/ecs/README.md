# Memory ECS dev 操作流程

GitHub 构建并推送固定摘要镜像 → 向 ECS 交付唯一 Compose 和管理员工具包 → 等待数据库只读验收 →
管理员在 ECS 执行原生迁移 → 流水线重新检查 dev 提交并发布同一候选 → 运行验收。

本包部署 Memory Service、项目侧 Mem0 HTTP 入口，连接已有 PostgreSQL、Elasticsearch；不部署数据库、Agent 或模型服务。
源码在 GitHub Runner 构建，ECS 不检出 Git 或编译。变更经 PR 进入 dev，不合并 main。
本地与 dev 各一份完整 Compose，无 override/include/extends。当前尚无目标 ECS 发布成功回执。

## 0. ECS 参数尚未确定时

可以先完成代码审阅、PR 检查、部署配置审计和隔离测试，不需要填写假的 ECS 地址或秘密。

| 阶段 | 工作 | 继续条件 |
|---|---|---|
| PR | Python 部署检查、Compose 解析、Rust 测试；不连接 ECS | 审阅通过并单独获准合并 dev |
| 镜像交付 | dev push 测试、构建两个 amd64 镜像、推送 GHCR、镜像内隔离验收和上传 artifact | 合并前确认有效的 `ECS_DEPLOY_ENABLED` 为 false 或未设置；包括组织继承值 |
| ECS prepare | 交付 Compose、固定摘要镜像和管理员工具包，不启动应用 | 已核验的 SSH 连接材料、专用目录和主机条件 |
| 迁移和发布 | 管理员数据库变更后由流水线发布同一候选 | PG/ES、运行配置、身份/模型材料及数据库验收 |

PR 的绿色结果只覆盖 test job；build/deploy 显示 skipped 是当前触发规则的预期结果。
关闭自动部署不等于停止已有服务；它也不限制显式的 workflow_dispatch prepare/release。
尚未获得发布授权时保留 PR，不为触发构建自行合并 dev 或 main。

以后先按[材料来源和交接方式](环境变量.md#材料来源和交接方式)补齐真实信息，再继续第 2～3 节。
已构建候选若因 SSH 或迁移等待失败，按原运行重跑失败 job；如果当时只执行 build 或 prepare，
仅补配置不会自动继续已完成的运行，需用下面的 `release-prepared` 入口复用已准备的同一 SHA/digest。
只有 build、尚未 prepare 时，仍需先交付候选。

## 1. 先准备配置和目标登记

配置消费者、填写位置见[环境变量清单](环境变量.md)。
GitHub 只保存 SSH/发布连接材料及路径 `ECS_ENV_FILE`；模型、ES、数据库运行密码与应用认证 Key 留在该变量指定的 ECS 文件中，例如 `/etc/lazytt/memory.env`。
管理员 owner 密码单独保存在 `/etc/lazytt/memory-migrate.env`，不进入常驻应用或 GitHub。
两个文件都要求普通文件、root:root、0600，采用 Bash `export NAME='value'` 格式。
模板为 [.env.dev.example](.env.dev.example)；实际秘密不得提交或粘贴到日志。
在 GitHub Settings → Secrets and variables → Actions → Variables 设置必填的 `ECS_ENV_FILE`，值为该应用文件的绝对路径。
prepare、等待检查、release 和交付的 `database.sh` 都显式使用该路径；直接运行 `release.py` 必须传 `--config`。
更换路径前核对新文件的内容、root:root 0600 权限和可加载性，再重新交付管理员入口；旧入口不会自动读取新变量。

首次配置时，在 ECS 终端先创建应用配置文件并设置权限；已有文件的内容会保留。当前用户若为 root，可省略 `sudo`。

```bash
sudo mkdir -p /etc/lazytt &&
sudo touch /etc/lazytt/memory.env &&
sudo chown root:root /etc/lazytt/memory.env &&
sudo chmod 600 /etc/lazytt/memory.env &&
sudo stat -c '%U:%G %a %n' /etc/lazytt/memory.env
```

预期输出为 `root:root 600 /etc/lazytt/memory.env`，表示只有 root 可以读写。
确认权限后再按模板填写配置；此时空文件尚不能用于部署。核验时只提供权限输出，不展示文件内容。

| 项目 | 固定值与待核验项 |
|---|---|
| 仓库/分支 | lazyaix/lazytt-memory-system / dev；PR 审阅后才能合并 |
| ECS/部署目录 | 由 ECS_HOST/ECS_REPO_DIR 指定独立绝对目录；实际主机、权限、容量待核验 |
| 数据库 | lazytt_memory_mem0；owner/runtime 为同名前缀的 `_owner` / `_runtime`；本库最小权限及跨库隔离 |
| 迁移 | 原版 Alembic 001～006；5 张应用表 + alembic_version；[结构注释字典](database/comments.json) |
| Store | 既有 PG/ES 的实际版本、私网地址/端口、证书、路由、索引归属、备份待核验 |
| 镜像 | Memory、Mem0 两个最终 linux/amd64 镜像，固定 digest 与同一完整 Git SHA；admin 复用 Mem0 镜像 |
| 入口 | Memory 仅 127.0.0.1:18081，Mem0 不映射宿主端口；本轮公网/Nginx 不适用 |
| 身份 | Memory 强制 jwt_hs256；内部 Mem0 使用独立服务 Key；实际签发方/租户由负责人登记 |
| 预算 | Mem0 2 CPU/2 GiB，Memory 1 CPU/512 MiB，admin 1 CPU/512 MiB；运行进程连接池 3+2，角色上限 8 |
| 持久位置 | 外部 PG、专用 ES 索引、lazytt-memory-ecs-dev_mem0-history 卷，备份分别覆盖 |
| 网络 | Docker 自动 IPAM；使用已登记可达地址连接 Store，不调整其他系统网络 |

启动门禁要求 MemAvailable ≥4 GiB；发布前核对网络路由、资源归属、端口、备份和整机预算。
常驻容器 restart: always，PID/日志/停止等待见 Compose。任何新公网暴露、未知项目接管、schema 迁入需另行审阅。
Mem0 上游源码保持原样；项目侧入口范围见 [Mem0 说明](../mem0/README.md)。

## 2. GitHub 流水线交付

首次准备好配置后，设置 `ECS_DEPLOY_ENABLED=true`、`ECS_DEPLOY_ACTION=release`。
PR 合入 dev 后运行：

1. Python/部署及 Rust 测试。
2. 串行构建 Memory/Mem0，发布 GHCR 固定摘要，核对上游源码并执行隔离迁移/HTTP/ES 验收。
3. SSH 严格校验主机密钥；拉取并检查 linux/amd64、OCI Git revision，交付 Compose 和管理员工具包。
4. 只读检查候选数据库；未通过时等待管理员，默认 1800 秒，每 15 秒检查一次。
5. 检查通过后，Runner 再核对 dev HEAD；未变化才发布同一 SHA/digest，不重新构建。
6. 只读检查配置格式及 PG 已存索引/模型兼容性 → 更新 Mem0 → 首次/变更配置通过认证 API 写入并回读（相同配置不写）→ 更新 Memory → 镜像及健康验收。

设置 `ECS_DEPLOY_ACTION=prepare` 可只交付，不等待/启动；`ECS_DEPLOY_ENABLED=false` 只测试/构建。
手工 dispatch 提供 build/prepare/release/release-prepared，但按钮通常要求 workflow 在默认分支；本轮不为按钮合并 main。
正常流程使用 dev push；迁移等待超时或 SSH 失败可在**原运行**选择 Re-run failed jobs，只重跑部署 job，
复用该运行已成功 build 的镜像输出。普通 prepare/release 完整重跑会重新构建；同一 SHA 若产生不同 digest，prepare 会拒绝覆盖。

如果 prepare 已经成功，管理员完成迁移后使用 `release-prepared`：

- 有 dispatch 入口时，选择 dev 和 `release-prepared`，把原 artifact `candidate.txt` 中两个完整镜像摘要填入 `memory_image`、`mem0_image`。
- 只有 dev push 入口时，设置 `ECS_DEPLOY_ENABLED=true`、`ECS_DEPLOY_ACTION=release-prepared`，
  将同一 artifact 的两个摘要填入 `ECS_CANDIDATE_MEMORY_IMAGE`、`ECS_CANDIDATE_MEM0_IMAGE`，重跑该 SHA 的原 push 运行。
- 此入口执行测试、跳过构建和 prepare；发布器先核对 SHA、两个 digest 与 ECS 候选完全一致，再只读验收数据库和配置。
  数据库未迁移或 dev HEAD 已变化都会拒绝启动。不要为了触发旧候选发布而创建新提交。
- 下一次发布新代码前，将 `ECS_DEPLOY_ACTION` 恢复为 prepare/release，并清空两个候选 Variables。

ECS 最终目录：

```text
<ECS_REPO_DIR>/
  compose.yml                 # 唯一运行 Compose，固化镜像摘要和候选信息
  database.sh                 # 当前候选的正式数据库入口，绑定完整 Git SHA 和两个镜像摘要
  admin/<GIT_SHA>/
    candidate.json            # 同一 SHA 的固定镜像组合，拒绝重新绑定
    release.py                # 仅发布/管理员工具，不是应用源码 checkout
    README.md
    .env.dev.example          # 只有占位模板
    database/
      01_roles.sql
      02_database.pgadmin.sql
      03_verify.sql
      comments.json
```

上游原生迁移文件、项目连接环境和结构清单位于**同一 Mem0 镜像**中，database.sh 经唯一 Compose 的
admin 服务调用它们；不维护第二份迁移 Compose 或复制修改过的 revision。
管理员包按 SHA 保留，用于人工交接与追溯；临时上传目录、SSH 和 registry 登录凭据正常退出后清理。
Actions artifact `memory-ecs-<SHA>` 保存 candidate.txt、Compose 和部署工具，保留 30 天。

## 3. 管理员只执行数据库变更

首次先在 Store 管理员窗口按顺序执行：

1. [01_roles.sql](database/01_roles.sql)：只创建本系统身份；通过 pgAdmin 角色属性或 psql `\password` 设置密码。
2. [02_database.pgadmin.sql](database/02_database.pgadmin.sql)：A 创建库，B 核对归属和库权限，C 切换目标库配置 schema 权限。
   CREATE DATABASE 必须独立于事务。不要把 A/B/C 在同一错误数据库中一次执行。
3. 共享 Store 管理员备份 ACL、核对其他项目后处理跨库 CONNECT；本包不自动撤销其他库 PUBLIC 权限。

应用文件使用 runtime 密码，迁移文件只含：

```bash
export MEM0_MIGRATION_PASSWORD='<REDACTED>'
```

确认流水线已输出 `ADMIN_BUNDLE_READY`，核对候选 SHA、备份及连接目标。在 ECS 执行：

```bash
cd <ECS_REPO_DIR>
./database.sh migrate --migration-config /etc/lazytt/memory-migrate.env
./database.sh database-check
```

迁移内部执行：身份核验 → 官方 Alembic upgrade 006 → 中文注释/运行权限 → 结构/迁移头/隔离检查 → 03_verify.sql。
成功输出 `MIGRATED`；流水线下一次只读检查通过后自动继续，不需要管理员手工启动应用或执行 configure。
迁移窗口仅要求 PG 连接配置，但自动发布前必须填完整应用文件。
若需要在 pgAdmin 独立复核，连接 lazytt_memory_mem0 执行 `admin/<SHA>/database/03_verify.sql`，应返回 `MEMORY_SCHEMA_SQL_OK`。
管理员工具和 release 都核对 prepare 的完整 SHA + 两个镜像摘要；期间候选改变时拒绝操作。
验收 SQL 比较全部 6 张表的字段、类型、非空和数据库默认值，以及完整索引/主外键/唯一约束定义、有效性及角色连接上限。
验收基线来自固定原版 001～006 在临时 PostgreSQL 18.4 中执行的结果；不是建表或手写迁移账本的替代入口。

迁移失败保留数据库和 Alembic 账本，停在该阶段调查；不 stamp、不手工拼表、不 DOWN/删库。
owner 凭据只进入一次性迁移任务，管理窗口结束后按秘密管理规则清理或保管。

## 4. 日常更新、迁移升级与恢复

已具备正确 schema 的普通更新无需管理员操作：只读检查直接通过，流水线发布。
新增迁移前先审阅原生 revision、注释/字典/验收 SQL、锁/耗时、备份恢复和旧新版本兼容窗口。
prepare 只更新候选 admin 镜像，保留现有两个应用镜像；管理员使用候选迁移，随后才发布。
不得修改已执行 revision 的内容或校验信息。升级 Mem0 必须同步固定版本、预期 head 和验收，不只改 tag。

等待超时：ECS 候选/工具包保留，旧应用保持原状；完成迁移后重跑原部署 job。
dev HEAD 已前进：流水线拒绝旧候选发布，重新审阅新候选，不能跳过提交检查。
普通发布不会迁移、建库、重建 Store 或删除卷；首次/显式变更配置由流水线经正式 API 写入。
模型/维度/索引变化会在改写运行 Compose、启动或停止任何应用之前拒绝，须另行制定数据迁移。
配置预检只读 PG，不调用 Mem0 API、模型或 ES，也不创建索引；服务连通性和真实模型能力仍在后续验收。

发布与管理员命令用宿主文件锁互斥；等待期间不持锁，管理员可执行迁移。
自动恢复仅适用于旧应用原本健康、旧镜像兼容当前 schema 且应用配置未变化的情况。
失败停止本次应用，兼容时恢复旧镜像并验健康；不兼容/首次发布时保留数据并向前修复，不回退数据库。
强杀或掉电不保证内存中的旧 Compose 可恢复；重连后核对候选、实际容器和 Actions 阶段。
本版本拒绝已运行 Compose 的拓扑/资源变化，需先补该类升级操作卡。

## 5. 分层验收

| 层次 | 目标 ECS 所需证据 |
|---|---|
| 制品/配置 | Actions/SHA、两个 digest、amd64/revision、配置文件权限和变量存在性，不打印值 |
| 数据库 | 原生 head、结构/注释/权限、跨库拒绝、备份及恢复位置 |
| 运行 | 两容器健康、ES/PG 可达、无 OOM/异常重启、现有 Store 未受影响 |
| 真实业务 | 获准 JWT 的 extract → retrieve → delete，确认真实模型与实际 ES 持久记录 |
| 拒绝/隔离 | 缺失/伪造 JWT、未授权 tenant、跨用户检索、无 forget 权限删除被拒绝 |
| 重启/恢复 | 重建应用后可召回、实际失败发布恢复/停止与数据一致性 |
| 公网 | 本包不适用 |

隔离测试的确定性模型替身只证明接口链路，不证明真实 Provider 的提取质量或可用性。
旧 smoke 脚本使用可信用户头，不适用于当前 JWT 部署。B 模式尚不宣称具备持久幂等去重。
实际业务测试会产生 API 用量并写入测试记忆，需在目标授权窗口执行。

2026-09-07 准备阶段核对：代码候选 `c705f8c86e22dda6e37a511ad78a280e3827339c` 的
[PR 检查](https://github.com/lazyaix/lazytt-memory-system/actions/runs/34096439699)通过，
Deployment checks 和 Rust tests 成功，build/deploy 未执行。
本机 Docker Linux 的 MemAvailable 当次约 1.31 GiB，未达到标准的 4 GiB 门槛，因此本轮未追加大型构建。
这些是当次记录；后续以最新候选的 Actions、镜像和目标 ECS 回执为准，不能把本地测试写成 ECS 部署成功。

2026-09-08 修复验收：42 项宿主 Python 测试、6 项镜像内失败路径测试通过。
临时 PostgreSQL 18.4 原生迁移/重复执行通过，11 类结构及权限漂移均被拒绝，修复测试对象后恢复通过；临时资源已清理。
本次将项目管理工具只读挂载到已有 arm64 镜像测试，未重新构建最终镜像；本机当次 MemAvailable 约 1.24 GiB。
新 amd64 镜像及完整 PG/ES 链路仍由 dev build job 验证，目标 ECS 和真实 Provider 验收仍未执行。

```text
时间/时区：
目标 ECS / 部署目录 / 候选 SHA / Actions：
Memory digest / Mem0 digest / Store 版本：
配置修订号（无秘密）/ 原生迁移 / SQL 验收：
运行 / 业务 / 拒绝 / 重启 / 发布失败恢复：通过、失败、阻塞、未执行；证据：
备份 / 恢复位置 / Store 前后状态 / 临时资源清理：
```
