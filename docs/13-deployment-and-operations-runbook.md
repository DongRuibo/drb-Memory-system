# 部署与运行手册

## 环境与组件

```text
memory-api             无状态在线服务
memory-worker          Outbox、投影、过期、擦除、对账
postgresql             memory_core / memory_search
mem0                   候选 Provider，私网访问
retrieval-models       真实 Embedding / 独立 Reranker，私网访问
memory-extractor       可选外部 Candidate Extractor
telemetry backend      metrics/logs/traces
secret/key service     Provider credentials and encryption keys
```

本地、测试、生产使用相同镜像和 Migration，配置与资源不同。生产镜像固定 digest，生成 SBOM，并保留 Provider/模型版本清单。

## 配置基线

- 非本地演示必须设置 `MEMORY_AUTH_MODE=jwt_hs256`、至少 32-byte 的
  `MEMORY_JWT_HS256_SECRET`、`MEMORY_JWT_ISSUER`、`MEMORY_JWT_AUDIENCE` 和逗号
  分隔的 `MEMORY_TENANT_IDS`；Secret 必须来自 Secret Manager；
- JWT `tenant_id` 必须位于 allowlist；服务为每个 allowlisted Tenant 启动独立
  Projection/Expiry Worker；
- JWT 可选 `tenant_scope=true`、`project_ids` 和 `agent_ids` 提供已签名的读取
  authority snapshot；每个 ID 非空且单项不超过 256 字符，三类附加 scope 合计
  不超过 32。生产签发方必须保证 membership 新鲜度并支持撤权；
- `MEMORY_AUTH_MODE=demo` 被强制限制在 loopback，禁止用于共享、测试集群或生产；
- Mem0 不暴露公网；
- API/Worker 使用不同数据库角色；
- RLS 开启且应用角色不能 bypass；
- Secret 从 Secret Manager 注入，不写配置文件；
- Provider egress 使用 allowlist；
- 每 Tenant 配额、请求大小和并发限制；
- 日志脱敏和正文采样默认关闭；
- 所有版本和 Policy/Compiler generation 显式配置。

检索排序默认使用不外发数据的 `MEMORY_RERANK_MODE=transparent`。本地真实语义
profile 使用 `dedicated`；兼容的 Kimi 排序路径仍可显式选择：

| 环境变量 | 默认值 | 说明 |
|---|---|---|
| `MEMORY_RERANK_MODE` | `transparent` | `transparent`、`dedicated` 或 `kimi` |
| `MEMORY_DEDICATED_RERANK_BASE_URL` | 无 | 独立 `/v1/rerank` 网关；外部地址必须 HTTPS |
| `MEMORY_DEDICATED_RERANK_MODEL` | 无 | 当前基线 `Qwen/Qwen3-Reranker-0.6B` |
| `MEMORY_DEDICATED_RERANK_REVISION` | 无 | 必须固定审核过的 revision |
| `MEMORY_DEDICATED_RERANK_MINIMUM_SCORE` | `0.10` | 0～1；当前合成 smoke 基线，正式阈值需评测批准 |
| `MEMORY_DEDICATED_RERANK_TIMEOUT_SECONDS` | `10` | 1～120 秒 |
| `KIMI_RERANK_API_KEY` | 无 | 优先使用的 Reranker Key；也支持 `KIMI_API_KEY` |
| `KIMI_RERANK_BASE_URL` | `https://api.kimi.com/coding/v1` | 只允许官方地址；loopback 仅测试 |
| `KIMI_RERANK_MODEL` | `k3` | 固定为 `k3` |
| `KIMI_RERANK_REASONING_EFFORT` | `low` | `low`、`high` 或 `max` |
| `KIMI_RERANK_TIMEOUT_SECONDS` | `90` | 5～300 秒 |
| `KIMI_RERANK_MIN_SCORE` | `0.60` | 0～1；低于阈值不返回 |
| `KIMI_RERANK_ENV_FILE` | `.env` | 仅本地演示；必须是权限 `0600` 的普通文件 |

服务根接口和每次 Search/Context Pack 都暴露实际 `ranking_version`；告警与评测必须
区分 Kimi 正常排序和 `RERANKER_UNAVAILABLE_TRANSPARENT_FALLBACK`，否则会把降级
结果误报成语义检索成功。

真实 Embedding 和自动提取 profile 还需要：

| 环境变量 | 参考值 | 说明 |
|---|---|---|
| `MEMORY_EMBEDDING_BASE_URL` | `http://127.0.0.1:18082` | OpenAI-compatible `/v1/embeddings`；外部地址必须 HTTPS |
| `MEMORY_EMBEDDING_PROVIDER` | `local-qwen` | 进入 generation manifest |
| `MEMORY_EMBEDDING_MODEL` | `Qwen/Qwen3-Embedding-0.6B` | 进入 generation manifest |
| `MEMORY_EMBEDDING_REVISION` | 固定 commit | 禁止生产使用漂移的 `main` |
| `MEMORY_EMBEDDING_DIMENSIONS` | `1024` | 必须与 collection/模型一致 |
| `MEMORY_EMBEDDING_NORMALIZATION` | `l2` | Readiness 验证 norm |
| `MEMORY_EMBEDDING_INSTRUCTION_VERSION` | `mem0-symmetric-document-v1` | 描述实际编码协议 |
| `MEMORY_EMBEDDING_SEMANTIC` | `true` | POC 固定向量不得声明为 true |
| `MEMORY_EXTRACT_MODE` | `disabled` | `disabled` 或 `kimi` |
| `KIMI_EXTRACT_API_KEY` | 无 | 优先专用 Key；本地可回退读取安全 `.env` |
| `DEMO_PROJECT_ID` | `lazytt-memory-system-demo` | 本地 Web 演示的已授权 Project Scope；Memory Service 与 Web API 必须一致 |

完整参考值见 `deploy/models/semantic.env.example`。Extractor 调用发生在 Secret
检测之后；User Scope 候选需要 `memory:extract`，Project Scope 还需要 `memory:write`
和已认证的项目成员关系。候选快照写入 PostgreSQL 后才进入正式写入链。

当前 HS256 模式用于内部 MVP 集成；面向多团队或外部身份域发布前，应接入受信任
OIDC/JWKS、非对称签名和密钥轮换，避免让每个验签方同时拥有签发能力。

## 健康检查

- `/health/live`：进程事件循环可用，不依赖 Provider；
- `/health/ready`：数据库、Embedding 真实探针、Mem0 health、Authority 配置以及
  每个配置 Tenant 当前 generation manifest 均可用且一致；
- Reranker 故障可以可观测降级；Embedding/active manifest 故障会使服务 not ready，
  防止错误向量继续写入；
- 当权威数据库或 Policy 不可用时必须 not ready。

## Migration

采用 expand → backfill → dual-read/write（需要时）→ verify → contract：

1. 新增向后兼容结构；
2. 部署可兼容新旧结构的应用；
3. 有界批量回填并对账；
4. 切换读取；
5. 观察至少一个发布窗口；
6. 删除旧结构。

DDL 回滚不是默认恢复方案；每次 Migration 必须有前滚修复、备份点和兼容性说明。

## 备份与恢复

- PostgreSQL 启用加密备份和 PITR；
- Tombstone/Erasure journal 的恢复点不得早于主库；
- 每个发布周期执行恢复演练；
- 恢复后先回放 Tombstone，再重建投影，最后开放服务；
- 记录实际 RPO、RTO、丢失/重复操作和人工步骤。

## 常用运行流程

### Provider 故障

1. 确认 Authority/DB 正常；
2. 打开熔断，停止新外部调用；
3. Context 降级为空或本地 FTS；
4. 观察 Outbox backlog 和磁盘容量；
5. Provider 恢复后限速重放；
6. 运行 hash/孤儿对账。

### Projection 重建

按照 [流程文档](./04-write-read-correct-forget-flows.md) 创建新 generation、校验、
切换，禁止原地覆盖唯一线上索引。当前内部 MVP 运行方式：

```bash
./scripts/rebuild-projection.sh rebuild-20260728-01
```

脚本会 start → poll ready → activate canary → finalize。会议或演练要验证回退时：

```bash
REBUILD_SWITCH_ACTION=rollback \
./scripts/rebuild-projection.sh rebuild-20260728-rollback
```

JWT 模式还需设置只含短期 `memory:admin` permission 的
`MEMORY_BEARER_TOKEN`。`SOURCE_GENERATION` 必须等于当前 active generation；
Activation 使用 expected-active compare-and-swap，防止操作者基于过期状态切换。
Building/ready/canary 期间普通写入返回 409，读取继续使用 active/canary；
finalize、rollback 或 abort 后解除冻结。

如果进度进入 `failed`，不要复用相同 generation 名称。先检查
`last_error_code`、Projection Operation/DLQ 和 Provider 状态，必要时调用幂等
`/{generation}/abort`，修复根因后使用新 generation。Retired/failed Provider
对象的批量 purge 尚未自动化，不得在保留/回退窗口前手工删除。Abort，以及
Forget/Erasure 对进行中重建的自动失效，会把该 generation 的 pending/leased/
unknown/failed Operation 和对应 Outbox 置为 `dead_letter` 并清理租约；旧 Worker
随后提交会被 fencing 拒绝，不能阻塞 active generation 的后续顺序队列。

当 Embedding model/revision/维度变化时，先准备新的 Mem0 collection/数据库，再使用
`./scripts/run-memory-service-semantic.sh` 和新 manifest 重建。当前一个 Memory
Service 只连接一个 Mem0 实例，因此跨模型迁移需要维护窗口；不能把 canary rollback
误解为能恢复已经被替换的旧维度 collection。

### Erasure 卡住

1. Authority Head 保持 Forgotten/Erased，不允许恢复读取；
2. 定位未完成 handler；
3. 使用相同 erasure/operation ID 重试；
4. deadline 前无法完成则升级安全事件；
5. 完成后生成不含正文的 Receipt。

### DLQ 重放

必须先确认根因和当前 Head/Tombstone。重放使用原 operation ID 或明确的新 repair operation，并保留关联关系；不得批量无条件重新 upsert。

## 发布和回退

- 先测试环境跑完整契约/评测/恢复；
- Canary 按 Tenant 隔离，监控安全与质量指标；
- 应用回退必须保持新旧 Schema 兼容；
- Provider generation 切换可独立回退；
- 出现越权、Secret escape、Tombstone resurrection 时立即停止相关能力，而不是仅降低流量。
