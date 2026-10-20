# PostgreSQL + pgvector 简历项目恢复

## 本轮交付边界

基线为 `2b48ba9`，分支为 `codex/resume-pgvector`，工作区位于原仓库下的
`.run/resume-pgvector`。原工作区的分支和已有修改保持原状。尚未提交或推送。

本轮恢复代码、离线测试和本地数据库验证：没有下载模型，没有调用云端推理 API，
没有创建真实云凭据。数据库测试和 SQL 不变量已通过，固定向量契约链已通过；
已有本地缓存中的真实 Qwen 模型也已离线加载，检索评测结果见本轮验证回执。
完整 Web 聊天尚未验收。
本轮测试结束后已停止测试服务，保留独立数据卷；使用下方命令可重新启动。
历史 README 和 `poc/evidence/` 中的实测结果不能当作本轮成绩。

PostgreSQL `memory_core` 保存权威记录、版本、账本和 Outbox；Mem0 作为检索投影适配层，
使用另外的 PostgreSQL + pgvector 保存向量。这个基线仍然使用 Mem0，
同时由自研层实施治理，二者不是相互排斥的方案。Demo 聊天数据使用独立数据库。

## 已补修复

| 项目 | 改动及验证范围 |
| --- | --- |
| 冲突候选保留 | 以 `d963232` 为起点，补 `0032` 迁移、正确的候选回执和审核收尾；连续冲突、重试幂等及 accept/reject 实库回归通过 |
| 模型凭据隔离 | 选择性引入 `d04b90d`：Embedding 和 Reranker 各读专用凭据，不继承聊天模型密钥；选择器单测通过 |
| 空租户首次启动 | 语义启动器默认开启空投影初始化，可显式禁用；事务内检查实际权威记录，不依赖可能过时的计数；数据库回归通过 |
| 隔离端口 | Memory/API/Web 端口、前端 API 地址及 CORS 一起传递；只允许 loopback，Web 端口占用时不自动换端口；进程替身测试通过 |
| 配置失败脱敏 | Mem0 配置失败仅输出错误类型，不打印可能包含凭据的异常文本；模拟失败测试通过 |
| 配置模板 | `deploy/local/resume.env.example` 提供独立 Compose 项目名、端口和离线开关，不含云凭据 |

空投影自动初始化只适用于没有任何 canonical memory record 的租户。
同一 manifest 的正常重启允许保留已有数据；有记录时更换 manifest 必须走受控重建。
重建与写入共用租户事务锁，重建维护期间写入会被拒绝并要求稍后重试。
去重、修正和审核的并发竞争回归已通过；尚未进行 bootstrap 与写入的并发测试或容量压测。

连续冲突的审核按整条记忆结束本次争议：accept 采用所选候选，reject 保留原值。
其余待审候选保留正文和来源，状态转为 rejected，另记
`REVIEW_ALTERNATIVE_SUPERSEDED` 决策；它们不能再通过旧 revision 覆盖审核后的值。
当前接口不支持“只拒绝一个候选并继续保持整组争议”的逐个审核模式。

## 当前验证结果

后续已完成 DeepSeek 真实抽取、聊天治理与 Episode 验证，并新增适用性复核。最新指标及
剩余风险见 [真实恢复验收](../poc/evidence/deepseek-recovery-2026-10-20.md)，
启动入口见 [本机演示](resume-demo.md)。以下保留前阶段证据。


2026-10-20 的结果见 `poc/evidence/resume-offline-2026-10-20.md`。
数据库、投影和本地模型的后续结果见 `poc/evidence/resume-runtime-2026-10-20.md`。
Rust 普通工作区测试 142 项通过、21 项数据库测试默认忽略；随后按两套测试库执行，
21 项数据库测试全部通过，SQL 不变量通过；Python 启动及脱敏测试 4 项通过；
Web 构建、2 项渲染测试和 lint 通过。这些结果不能证明真实语义效果。
前一阶段 v2.1.1 的真实 Qwen 检索 Recall@3=1、MRR=1，但负例命中率为 0.8。
随后已按确认的规则区分 global/contextual 偏好，并要求显式写入声明事实类型；
新增 Migration 0033、通用偏好独立召回与元数据修正回归。v2.2.0 实测 Recall@3=1、MRR=1、
负例命中率=0.2、遗忘后注入=0，完整门槛仍未通过。数据类型和范围已修正，不能把两个
版本的变化归因为同条件模型性能提升。最新测试和剩余风险见
[偏好政策验收回执](../poc/evidence/preference-policy-2026-10-20.md)及
[接口和分类规则](preference-applicability.md)。
严格 Clippy 有一处基线既有 `manual_clamp` 告警，单独放行该规则后整个工作区检查通过；
未为此修改不参与本次 pgvector 路径的 Elasticsearch adapter。

## 复现 PostgreSQL 集成验证

依赖：Docker Desktop 已启动、Rust/Cargo 在 PATH，以及本地存在 `postgres:17-alpine`
镜像。下面的 `--pull never --no-build` 禁止隐式拉镜像或构建。如果镜像缺失应停下，
先说明缺少的普通运行依赖，不改用会下载模型的完整启动脚本。

先进入恢复工作区；只在该工作区执行配置初始化。首次执行前确认没有已有 `.env`，
不要覆盖已有文件，也不要复制原工作区的 `.env`。

```bash
cd /Users/drb/code/lazytt-memory-system/.run/resume-pgvector
# 仅首次：test 失败即停止，不覆盖已有配置
if test ! -e .env; then
  cp deploy/local/resume.env.example .env
  ./scripts/bootstrap-backend-env.sh
fi
set -a
. ./.env
set +a
docker compose up -d --wait --pull never --no-build postgres
# 以下三个库各创建一次；若已存在则跳过 createdb
docker compose exec -T postgres createdb -U memory memory_core_test
docker compose exec -T postgres createdb -U memory demo_chat_test
docker compose exec -T postgres createdb -U memory demo_chat
export DATABASE_URL="postgres://memory:${MEMORY_POSTGRES_PASSWORD}@127.0.0.1:${POSTGRES_HOST_PORT}/memory"
export DEMO_DATABASE_URL="postgres://memory:${MEMORY_POSTGRES_PASSWORD}@127.0.0.1:${POSTGRES_HOST_PORT}/demo_chat"
export TEST_DATABASE_URL="postgres://memory:${MEMORY_POSTGRES_PASSWORD}@127.0.0.1:${POSTGRES_HOST_PORT}/memory_core_test"
cargo test --locked --offline -p memory-postgres -p memory-service -- --ignored --test-threads=1
TEST_DATABASE_URL="postgres://memory:${MEMORY_POSTGRES_PASSWORD}@127.0.0.1:${POSTGRES_HOST_PORT}/demo_chat_test" \
  cargo test --locked --offline -p demo-web-api -- --ignored --test-threads=1
docker compose exec -T postgres psql -U memory -d memory_core_test \
  -v ON_ERROR_STOP=1 < tests/sql/ledger_invariants.sql
```

不要开启 shell tracing 或把环境变量、连接串贴入聊天。上面只启动新 Compose 项目里的
canonical PostgreSQL，不启动 Mem0、模型或聊天 API。退出时可使用
`docker compose stop postgres` 保留数据，不执行删除卷的命令。

Memory 与 Demo 的测试也必须分库，不能用一个 TEST_DATABASE_URL 执行全工作区
ignored 测试：Demo 的独立 snapshot 先执行后，会与 Memory 的历史迁移发生同名表冲突。

## 复现无模型契约验证与本地模型加载

以下只使用已存在的镜像，保持 `--pull never --no-build`。Mem0、pgvector 与模型服务位于
`internal=true` 网络；本机只通过 gateway 的固定两个目标访问它们。Gateway 不记录请求，
只监听映射到 loopback 的入口；它是本地测试工具，不能用作公开代理。

```bash
# 在恢复工作区，并已加载本工作区 .env
docker compose -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  up -d --wait --pull never --no-build mem0 contract-embedder contract-gateway
docker compose -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  run --rm --no-deps --pull never contract-configurator
export MEM0_BASE_URL=http://127.0.0.1:19889
cargo run --locked --offline -p poc-cli -- cycle
cargo run --locked --offline -p poc-cli -- persist-write
docker compose -f compose.yaml -f deploy/local/resume-contract.compose.yaml restart mem0-postgres mem0
docker compose -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  up -d --wait --pull never --no-build mem0
cargo run --locked --offline -p poc-cli -- persist-verify
cargo run --locked --offline -p poc-cli -- duplicate-probe
```

这一步是常量 1536 维向量，只证明持久化、过滤和删除契约。切换真实模型前，必须停止
使用该测试配置的 Memory Service，避免跨维度调用。下面使用已有 CPU 镜像和只读模型缓存，
没有缓存或镜像时应停止，不能拉取镜像或联网下载权重。

```bash
export RESUME_HF_HUB_CACHE="$HOME/.cache/huggingface/hub"
docker compose -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  -f deploy/local/resume-semantic.compose.yaml \
  up -d --wait --pull never --no-build contract-embedder
docker compose -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  -f deploy/local/resume-semantic.compose.yaml \
  run --rm --no-deps --pull never contract-configurator
```

真实模型使用独立的 `resume_qwen3_1024` collection，不覆盖 `resume_contract_1536`。
语义模式固定完整 Embedding/Reranker revision，并通过离线开关与只读缓存阻止下载。

在前面的 DATABASE_URL 已导出的终端中启动本地测试服务：

```bash
export MEM0_BASE_URL=http://127.0.0.1:19889
export MEMORY_EMBEDDING_BASE_URL=http://127.0.0.1:19081
export MEMORY_DEDICATED_RERANK_BASE_URL=http://127.0.0.1:19081
export MEMORY_DEDICATED_RERANK_REVISION=e61197ed45024b0ed8a2d74b80b4d909f1255473
export MEMORY_DEDICATED_RERANK_TIMEOUT_SECONDS=60
export MEMORY_INDEX_TIMEOUT_SECONDS=30
export MEMORY_AUTH_MODE=demo
export DEMO_TENANT_ID=retrieval-eval-v211-20261020
export MEMORY_TENANT_IDS="$DEMO_TENANT_ID"
export DEMO_USER_ID=evaluator
./scripts/run-memory-service-semantic.sh
```

`demo` 只用于本机合成验证，不改变 `.env` 中的 JWT 默认值，也不代表正式认证上线验收。
另开终端加载相同环境并运行 `scripts/eval-retrieval-v2.sh`：设置
`MEMORY_BASE_URL=http://127.0.0.1:19080`、`MEM0_BASE_URL=http://127.0.0.1:19889`、
`EVAL_TENANT_ID=retrieval-eval-v211-20261020`、`EVAL_USER_ID=evaluator`、
`EVAL_QUERY_CONCURRENCY=1`、`EVAL_PROVIDER_PROFILE=qwen3-cpu-offline-readonly-cache` 和
`EVAL_ENFORCE_GATE=true`。`EVAL_GENERATION` 必须取该租户实际 active generation，不能用
脚本默认的旧 generation；可通过 canonical PostgreSQL 的
`memory_core.projection_generations` 查询。将 `EVAL_OUTPUT_DIR` 指向新的验证目录，保留失败案例。

验收标准：21 项已忽略数据库测试全部通过；SQL invariant 脚本通过；重点确认连续冲突
保留两个待审候选且只有一次投影删除、幂等重试不重复候选、空租户初始化可重试、
非空租户换 manifest 被拒绝且原 generation 不变。失败先修最小复现再重跑受影响检查。

## 后续：完整 Web 及其他依赖

下表为常规恢复配置；本轮验证通过 gateway 使用 19889/19081，并没有启动完整 Web 聊天。

| 依赖 | 隔离配置 | 待验证内容 |
| --- | --- | --- |
| Canonical PostgreSQL | `127.0.0.1:56432`，新卷 | migrations、账本、RLS、Outbox |
| Mem0 + pgvector | Mem0 `127.0.0.1:19888`，独立库/卷/collection | 1024 维、过滤、投影写入/删除 |
| 本机模型网关 | `127.0.0.1:19082` | 固定 Qwen3 Embedding/Reranker revision、维度及归一化 |
| Memory Service | `127.0.0.1:19080` | JWT、manifest、readiness、治理 API |
| Demo API | `127.0.0.1:19090`，`demo_chat` 数据库 | 聊天持久化、用户隔离、流式回答 |
| Web | `127.0.0.1:13000` | API 地址、CORS、跨会话回忆、修正和遗忘 |

本地模型 Python 依赖见 `deploy/models/requirements.lock.txt`，启动入口为
`scripts/retrieval-models.sh`。已有缓存目录不等于模型可运行，仍需核对完整权重和 Python
依赖。模板启用 `HF_HUB_OFFLINE=1` 与 `TRANSFORMERS_OFFLINE=1`，缓存不全时应失败，
不能自动联网补齐。本轮没有新建 Python 环境，复用了已有 CPU 镜像和本机模型缓存完成加载。

Mem0、pgvector、配置器和 CPU 模型镜像已核对存在且没有拉取。后续宿主机模型路径使用 Compose
的 `host-mps` 配置器，已支持读取 `LOCAL_RETRIEVAL_MODELS_PORT`；不要同时启动容器版
Memory Service 和宿主机启动器。模板中的离线开关不意味着任意旧容器启动命令都会继承它。

当前 `demo-web-stack.sh` 仍要求聊天模型密钥；模板将抽取设为 disabled，因此不能直接
获得自然对话记忆的完整演示。不要放入假密钥并把报错当作恢复成功。解除云 API 限制后，
需要用户自行在本机私有 `.env` 配置合法聊天/抽取凭据、端点和模型，再开启抽取。
密钥不要发给助手或提交 Git。

## 完整恢复的验收门槛

1. 数据库集成测试和 SQL invariant 通过，再启动真实 Mem0 + pgvector，确认新 collection
   的 1024 维 manifest、投影 upsert/delete 与失败重试。
2. 固定时间和时区，用合成数据完成三轮“自然对话写入 → 新会话回忆 → 事实修正 →
   版本链检查 → 遗忘 → Context Pack 零注入”，保存脱敏 API 回执和截图。
3. 单独验证 Episode/consolidation；Web 当前仍使用抽取路径，不能用 Web 成功代替该模块验收。
4. 重新运行 `evals/retrieval-v2/dataset.json`：20 文档、19 正例、5 负例；目标为
   Recall@3 ≥ 0.85、MRR ≥ 0.75、负例命中为 0、遗忘后注入为 0。历史指标不复用。
   恢复先升为 v2.1.1 修复事件身份，再升为 v2.2.0 补充正确类型和事件时间；详见数据集 README。
5. 抽取评测需补一份至少 20 例的合成用例与评分结果，覆盖偏好、事实、相对时间、冲突、
   无记忆及恶意候选。本轮尚未新增该数据集或生成模型评分。
6. 至少 30 次预热后请求，记录设备、模型 revision、P50/P95，冷启动单列；当前不声称性能达标。

物理全副本擦除、双 generation 高可用切换、生产容量和模型质量均未在本轮验证。
简历现阶段可以描述实现和离线测试，不能写成已重新完成生产部署或真实模型效果验收。
