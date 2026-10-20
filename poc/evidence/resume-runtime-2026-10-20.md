# 2026-10-20 本地运行验证回执

基线 `2b48ba9`，恢复分支 `codex/resume-pgvector`。所有数据库和容器均属于新建的
`lazytt-resume-pgvector` 项目，原工作区与已有 Docker 项目未修改。未提交或推送。

## 已执行结果

| 验证 | 本轮结果 | 范围 |
| --- | --- | --- |
| 普通 Rust 工作区测试 | 142 passed，0 failed，21 默认 ignored | 本地缓存、合成 HTTP；新增 1 项数据库回归 |
| Memory PostgreSQL 集成测试 | 17 passed | 去重并发、修正、冲突与审核、权限交集、时间检索、Context Pack、Expiry/Erasure、generation 状态机 |
| Memory Service 集成测试 | 3 passed | 空租户初始化与非空保护、已完成轮次的幂等提取、10 轮 FakeExtractor consolidation |
| Demo Chat 集成测试 | 1 passed | 独立聊天库的持久化、重放与 reset |
| SQL invariant | `ledger invariants OK` | 在非 Owner 测试角色下检查账本与 Head 状态约束，事务 rollback |
| 启动与脱敏替身测试 | 4 passed | 端口、CORS、显式禁用 bootstrap、配置失败不回显详情 |
| Mem0 `health/cycle` | 通过 | 真实 REST + pgvector，固定向量，写入/namespace/generation 过滤/删除 |
| Mem0 重启持久化 | 通过 | 写 marker，同时重启 Mem0 与向量 PostgreSQL，重读并清理 marker |
| Mem0 duplicate probe | 通过预期断言 | 相同 operation 返回两个 provider objects；Mem0 不提供操作级幂等 |
| `scripts/demo-v0.sh` | 通过 | 显式记忆 → PostgreSQL → Outbox → Mem0 → 权威回查 → Forget 立即拒读 → 物理投影删除 |
| Qwen 模型加载与 smoke | 通过 | 复用只读缓存、CPU、1024 维、L2 norm、两文本排序，无云调用 |
| 固定集真实检索评测 | **未通过完整门槛** | v2.1.1，Recall@3=1、MRR=1、负例命中率=0.8、遗忘注入=0 |
| Clippy | 放行基线 `manual_clamp` 一项后通过 | 未改动 Elasticsearch 的原有写法 |
| `git diff --check`、shell/Python 语法 | 通过 | 修改范围检查；前端此前构建、2 项渲染测试与 lint 通过，后续未改前端 |

详细日志保存在恢复工作区 `.run/resume-validation/`，全部为本轮合成数据。
`memory_core_test.log` 记录 17 + 3 项通过，`demo_chat_test.log` 记录 1 项通过；
`mem0-contract.log`、`demo-v0-contract.log`、`qwen-models.json` 分别记录投影契约、业务链和模型 smoke。

## 运行依赖与隔离

本次数据库为 PostgreSQL 17.10、pgvector 0.8.5，契约 collection 是 `vector(1536)`。
真实模型使用新的 `resume_qwen3_1024` collection 和新租户。

镜像全部来自已有本地缓存，所有启动均指定 `--pull never --no-build`：

| 镜像 | 本机 Image ID |
| --- | --- |
| postgres:17-alpine | `sha256:742f40ea20b9ff2ff31db5458d127452988a2164df9e17441e191f3b72252193` |
| pgvector/pgvector:pg17 | `sha256:d2ef61f42ef767baa5a1475393303cc235bcd92febd9d7014eddb48b41f3bad0` |
| lazytt-mem0-api:b357a5a1 | `sha256:ae73c38d9083983da1925b7c6d190b43016ce7b876c4842e603c79f489e9d0a3` |
| python:3.12-slim | `sha256:57cd7c3a7a273101a6485ba99423ee568157882804b1124b4dd04266317710de` |
| lazytt-retrieval-models:cpu | `sha256:c452c3110cc523cfda616867ca9c8a28828854f79e8420df7aae667b4402c257` |

Mem0、向量库及模型只连接 `internal=true` 的 contracts 网络；测试 gateway 转发至固定的
Mem0/模型目标，并将入口映射到 `127.0.0.1:19889` 与 `127.0.0.1:19081`。
权威 PostgreSQL 使用 `127.0.0.1:56432`，Rust 服务使用 `127.0.0.1:19080`。
没有重用原工作区 `.env`；新生成的本地凭据位于恢复工作区被忽略的 0600 `.env`。

真实模型运行时为 torch 2.13.0+cpu、transformers 4.57.6。Embedding revision 为
`97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`，Reranker revision 为
`e61197ed45024b0ed8a2d74b80b4d909f1255473`。
模型缓存挂载只读，同时启用 `HF_HUB_OFFLINE=1` 与 `TRANSFORMERS_OFFLINE=1`。
两条文本的向量 norm 约为 1，cosine 为 0.22273；排序 smoke 相关文本得分 0.70467、
无关文本为 0.0000866。这个 smoke 只验证模型接口与基本区分，不代表全量检索质量。

## 真实检索质量结果

本轮使用 v2.1.1、20 个文档、19 个正例、5 个负例，查询并发 1，独立租户
`retrieval-eval-v211-20261020`，generation `bootstrap-926ecc0230c4736f`。
完整产物位于 [本轮检索报告](./resume-retrieval-2026-10-20/comparison-report.md)，
包括机器可读 metrics、manifest 和 raw results。

| 指标 | 本轮结果 | 原门槛 |
| --- | --- | --- |
| Hybrid Recall@3 | 1.00 | ≥ 0.85 |
| Hybrid MRR | 1.00 | ≥ 0.75 |
| 负例命中率 | **0.80，未通过** | 0 |
| 遗忘后注入 | 0 | 0 |
| 全门槛 | **FAIL** | 全部通过 |

服务正例 Top-3 中的无关注入率另为 0.655；Recall/MRR 满分不代表返回内容都相关。
模型加载和存储治理链已恢复，不能据此宣布检索质量验收完成。

源码中的 `is_memory_applicable_without_semantic_threshold` 对所有 `preference` 返回 true，
HTTP Reranker 对此绕过最低语义分数。固定集又通过显式入口把所有文档存为 preference，
因此当前注入政策与“所有负例返回空”的标准存在冲突。变更这项政策会影响原来保留低主题
相关度通用偏好的行为和已有测试，等待用户明确取舍后再修改。

现有评测器将负例请求异常也计作失败，但 failure-cases.jsonl 只收集正例，
所以报告中的 `Failure cases: 0` 不表示负例没有失败；负例细节另行补录。

补录 [negative-traces.json](./resume-retrieval-2026-10-20/negative-traces.json) 使用同一版本的
20 文档重新摄入并查询 5 个负例，之后提交 Forget 清理。此次 5 个请求均成功、未降级，
确认为 n01/n03/n04/n05 四个查询发生误召回；n02 返回空。
前三个查询的最高得分分别约 0.00990、0.01534、0.01133，低于当前 0.1 阈值，
仍被 preference 绕过规则保留。n05 对 `consume_route` 得分为 0.75171，
属于高分难负例，单改 preference 的阈值绕过规则不能保证负例门槛通过。
这些命中均映射到当前评测租户的合成文档，不能把它解读成读到了其他租户的数据。

## 本轮发现与修复

1. Demo 独立 snapshot 与 Memory 历史迁移不能先后运行在同一个测试库。
   最初错误的全工作区单库命令失败，随后改用 `memory_core_test` 与 `demo_chat_test`，
   保留初次失败日志，分库后 21 项全部通过。
2. `d963232` 的候选保留代码与旧唯一索引不兼容，实库报 `uq_memory_candidates_open_review_target`。
   新增 migration `0032`，不修改历史 checksum；候选回执按摄入来源返回正确 candidate，
   审核结束整组冲突并为其他候选追加 `REVIEW_ALTERNATIVE_SUPERSEDED` 决策。
   accept/reject、幂等重放及旧候选不能再覆盖当前值均有实库断言。
3. 原 `retrieval-v2.1.0` 两条发布记录共用显式偏好键，摄入时被当作冲突并删除投影，
   评测退出，未产生质量指标。v2.1.1 仅区分两个标签，日期、查询、目标 ID 和阈值不变。
   标签变化会影响 Embedding/BM25，新指标不能冒充旧版完全同条件复现。

## 尚未完成

没有调用云 API。自然对话抽取、模型流式回答和完整浏览器聊天 E2E 尚未执行；
Episode/consolidation 当前通过的是 FakeExtractor + 真实 PostgreSQL，不是真实抽取质量。
本轮 HTTP 业务链使用 loopback `demo` 认证，JWT 仍只有离线认证测试证据。
未验证生产角色配置、30 次预热后的性能统计、容量、高可用切换或全副本物理擦除。
至少 20 例真实抽取评分集及评分结果仍待补充。

阶段判定：本地数据库、治理与投影链可运行；已有 Qwen 模型可严格离线运行；
真实检索负例门槛未通过，不能把本次恢复描述成完整验收或生产可用。

收尾时已停止本轮原生 Memory Service 及本项目测试容器，保留配置、日志、数据库卷与模型缓存。
成功评测和负例补录共 40 条合成记忆均已进入 forgotten，停止前没有待处理投影操作。
最初失败的 v2.1.0 租户保留争议记录作为失败证据，未删除账本。
原有 `lazytt-memory-local` 项目未停止或修改。


后续已确认并实现偏好分类与事实写入修复；最新 v2.2.0 结果和未通过项见
[偏好政策验收回执](preference-policy-2026-10-20.md)。本文件保留前一阶段的原始结果。
