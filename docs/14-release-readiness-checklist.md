# 开发和发布准入清单

清单项必须附 PR、测试结果、评测报告或运行记录；口头确认不算完成。

## Architecture Freeze Gate

- [x] 统一领域语言并区分 Source、Candidate、Version、Head、Projection、Context；
- [x] 记录 Authority before Index、Ledger、最终一致投影和读取权限交集 ADR；
- [x] 禁止 `extract_and_store` 作为正式写入原语；
- [x] 定义写入、修正、争议、Forget、Erasure 和重建流程；
- [x] 定义数据表职责、事务边界和数据库不变量；
- [x] 定义 Provider 与 Context Pack 契约；
- [x] 定义安全、故障、评测、测试、SLO 和运行基线；
- [ ] 架构与安全评审通过；
- [ ] ADR-0001 的 Mem0 POC 结果完成评审。

## POC Gate

- [x] 固定 Mem0 OSS source commit、本地镜像 digest 和实际 OpenAPI 契约；
- [x] 用领域类型和 `infer=false` Adapter 证明 Candidate 与正式 Projection 写入隔离；
- [ ] Rust spike 完成 extract/upsert/search/delete/health；
- [ ] 中文、关键词、更新、冲突、删除和隔离 smoke 数据集跑通；
- [x] Provider 超时、重启、unknown result 和幂等行为有证据（结果证明 Mem0 本身不提供 operation-level idempotency）；
- [ ] 从 `memory_core` 全量重建新 generation 成功；
  - [x] generation/transition Schema、RLS、稳定 Operation/Outbox、写冻结、
    canonical/local 校验、canary、幂等 finalize/rollback/abort 已实现；
  - [ ] 最新 migration 在真实 PostgreSQL 上应用，并完成
    `projection_generation_rebuild_switch_rollback_and_finalize_are_safe` 与真实
    Mem0 全链验收；
- [x] 数据外发、日志和遥测边界已确认（`infer=false` 仍向 Embedder 发送正文；合成本地 POC 使用 hash-only Embedder 并关闭遥测）；
- [ ] 记录资源、延迟、token、成本和已知限制；
- [ ] 根据证据接受、修订或拒绝 ADR-0001。

## MVP Development Gate

- [ ] OpenAPI/事件契约通过 lint 和契约测试；
- [ ] PostgreSQL Migration、RLS、复合外键和 append-only 权限完成；
- [ ] Candidate/Decision/Ledger/Head/Outbox Repository 完成；
- [ ] Policy reason codes、Review API 和 typed Scope 完成；
- [ ] 并发单赢家、幂等和事务回滚测试通过；
- [x] Provider Adapter 契约测试通过；
- [x] Context Pack、Usage Receipt、原子 Consume 和 Correction/Forget invalidation 完成；
- [ ] Forget/Tombstone/Erasure/重放测试通过；
- [ ] 所有领域不变量与测试建立映射；
- [ ] Agent Runtime 只通过 Memory API/port 集成；
  - [x] 可复用 `memory-client` / `MemoryPort` 已实现，并由 `demo-agent` 使用；
  - [ ] LazyTT Agent Runtime 本体接入该 Port，并通过独立契约测试。

## Demo v0 Gate

- [x] 明确 `请记住` 请求能确定性生成低风险 Preference Candidate；
- [x] Source/Candidate/Decision/Version/Event/Head/Outbox 在一个 PostgreSQL 事务中提交；
- [x] 相同 Idempotency-Key 返回相同 canonical Operation；
- [x] Outbox Worker 实现 lease/fencing、Head/Tombstone 复查和 Provider reconciliation；
- [x] Search 仅使用 Mem0 返回的映射字段，并从 PostgreSQL 加载和授权 canonical 正文；
- [x] Forget 提交后由 Head/Tombstone 立即阻断读取，并异步删除 Provider Projection；
- [x] `scripts/demo-v0.sh` 覆盖 remember/idempotency/ready/search/forget/deleted；
- [x] 最新代码在全新 PostgreSQL + 固定 Mem0 runtime 上完成端到端演示验收（2026-07-27，`scripts/demo-v0.sh` 连续两次通过）。

## Demo Agent v1 Gate

- [x] 独立 `demo-agent` 仅通过 Memory API 访问 canonical memory，并通过 Kimi Code OpenAI 兼容协议调用 `k3`；
- [x] Kimi 官方 Base URL 和 `k3` 被显式约束，API Key 只从非空环境变量读取，错误与输出不包含 Key；
- [x] 新 Task/Attempt 通过服务端 Context Pack API 获取 PostgreSQL 同事务重新授权的 Preference；
- [x] Agent 将 Context Pack item 编码为未信任记忆数据，并按 section 限制为回答风格、
  事实个性化或相关经验背景，不允许改变权限、身份与工具行为；
- [x] Forget 提交后立即确认目标 Memory 不可读，再发起不含该记忆的新 Task/Attempt，并等待 Provider Projection 删除；
- [x] `scripts/demo-agent-v1.sh` 预检 Key 和 Memory Service，就绪后优先运行 Cargo binary；
- [x] 使用本地确定性 Chat Completions 假服务完成完整 HTTP 编排验收，并证明 `memory_context_count` 从 1 变为 0；
- [x] 使用已轮换的真实 Kimi Key 完成端到端运行验收，并保存不含凭据的运行证据（2026-07-28）；
- [x] Demo Context Pack API、确定性预算、Usage Receipt 和 Forget invalidation 已替换客户端选择逻辑，并完成本地端到端验收（2026-07-28）；
- [x] 服务端 Context Pack 改造后再次完成真实 Kimi `k3` 回归，并核验 Pack/Receipt/失效数据库证据（2026-07-28）；
- [x] 本地 Web 演示台通过“记住、用上、忘掉”向非技术观众展示真实链路，浏览器不接触 Kimi Key；
- [x] Web API 使用 loopback、受限 CORS 和单飞锁，并通过本地假模型端到端验收（2026-07-28）；
- [x] Context Pack Compile 不返回正文，Consume 在同一事务中完成 Attempt/Authority/Head/Version/Tombstone/来源复核和幂等消费，并通过 Forget 并发测试（2026-07-28）；
- [x] 相同 semantic identity/content 的并发写入只建立一个 live Record/Version，并将所有 Source 附加到该 Version（2026-07-28）；
- [x] 显式 Correction 追加不可变 Version，使用 Head revision 保证并发单赢家，使旧 Pack 立即失效，并在 Mem0/FTS 中收敛为新 Version（2026-07-28）；
- [x] 不同正文的同一 semantic identity 进入 NeedsReview/Contested，立即 fail closed；Review 接受/拒绝、幂等、Candidate revision 单赢家和投影恢复通过 PostgreSQL 与 HTTP/Mem0 验证（2026-07-28）；
- [x] Inspect 返回完整 Version/Source/Event 历史，用户 Export 在一个快照中导出全部非 Erased 记录并隔离其他用户（2026-07-28）；
- [x] Expiry Worker 根据 retain/effective deadline 将 Head fail closed、使 Pack 失效并异步清理 Mem0/FTS，重复扫描不重复执行（2026-07-28）；
- [x] 记录级、用户级和已授权项目级 Erasure 擦除 canonical/Context/Source 正文，并在投影 Receipt 后完成；Project target 只持久化 hash（2026-07-28）；
- [x] JWT 模式从签名令牌派生 Tenant/User，校验短期时效与 Tenant allowlist，执行独立 API permission，并通过 Alice/Bob HTTP owner 隔离测试（2026-07-28）；
- [x] Tenant/User/Project/Agent typed-scope 贯穿 Search、Inspect、Operation、Export 和 Context Pack；多类型记忆 section 映射、Project membership removal fail-closed 与 Project Erasure 通过 PostgreSQL 集成测试（2026-07-28）；
- [x] Search 将 Mem0 与 PostgreSQL 本地候选通过 RRF 融合，在 canonical 授权后
  使用版本化透明/Kimi `k3` Reranker，并在模型返回后再次执行 canonical 授权；
  Provider/Reranker 失败可观测降级（2026-07-28）；
- [x] 固定 `retrieval-v1.0.0` 合成集比较 no-memory、本地词法、Mem0、混合服务
  和 oracle；Kimi 混合服务 15 条正查询 Recall@3/MRR 为 `1.000/1.000`，3 条
  负查询命中率和 Forget 后注入数均为 0（仅 smoke，不代表生产质量，2026-07-28）；
- [ ] 接入外部 membership authority、OIDC/JWKS、key rotation 和主动撤权事件，证明权限变化在批准时限内传播；
- [ ] 补齐正式 tokenizer、完整多特征排序解释、Pack 签名和 Policy/ACL/Consent 全量失效；
- [x] Attempt-bound、Tenant 隔离、幂等且不保存回答正文的 Outcome Receipt 已实现；
- [ ] LazyTT Agent Runtime 完成正式 Memory API/port 集成并通过独立契约与端到端测试；
  - [x] SDK/Port、HTTPS/Bearer 边界、有界重试和 Context Pack 一致性复核已实现；
  - [ ] Runtime 身份、Attempt 生命周期、Prompt 注入、Outcome 关联和端到端验收完成。

## Semantic Automatic Memory Gate

- [x] 真实 Embedding 适配器和 readiness probe 校验模型、维度、有限非零向量与 L2 norm；
- [x] generation 固化 index/Embedding provider、model、revision、维度、归一化、
  指令版本、semantic 标志和 manifest hash；在线读写与 Worker 拒绝 manifest 漂移；
- [x] 本地网关实现固定 revision 的 Qwen3 Embedding 和独立 Qwen3 Reranker；
- [x] 独立 Reranker 使用 bounded contract，拒绝未知/重复/缺失 index 与非法分数，
  并保留 rerank 前后 canonical Authority 复核；Preference 始终按行为适用性保留，
  用户明确请求工作角色、项目事实或历史经验时按查询意图保留对应低分候选且不篡改分数；
- [x] Kimi `k3` Extractor 只输出服务端按 Scope 授权的严格 Proposal；User Scope 的
  `preference`、`profile_fact`、`episodic_summary` 门槛为 0.80，Project Scope 的
  `project_fact` 门槛为 0.90，模型不能自行选择 Scope；
- [x] 自动提取在模型前后执行 Secret/结构校验，候选继续经过 Policy、精确去重、
  来源合并、冲突/Review、Ledger 与 Outbox；
- [x] 抽取主 Idempotency-Key 固化首个完整候选快照（包括零候选），候选子操作使用
  稳定派生 Key，重试不会接受第二组漂移结果；
- [x] `memory-client`、Demo Agent 自动模式和 `scripts/demo-auto-memory.sh` 已实现；
- [x] 在 PostgreSQL 应用 Migration `0013/0014` 并通过 10 个数据库集成测试（2026-07-29）；
- [x] 实际下载固定 revision 的两个 Qwen 模型，保存 `/health` 与 Embedding/Rerank
  smoke 证据；
- [x] 新建 `memories_qwen3_1024` collection，从 PostgreSQL 重建并切换到
  `qwen3-semantic-20260729b`，核验 3/3 投影和 manifest（2026-07-29）；
- [x] 跑通“自然对话 → 自动提取 → 去重/投影 → Context Pack → Kimi 回答 →
  Outcome → Forget”的完整 HTTP/会议 Demo，并保存不含凭据的验收记录（2026-07-29）；
- [x] 重新跑通 Web 的四类记忆统一演示：自动 User/Project Scope、跨对话召回 5 条、
  项目事实自动 supersede 并提示、形成 v2、指定经历 Forget 零注入；真实 Kimi `k3` 回答、
  Context Pack 与 Outcome Receipt 均完成，并保存不含凭据的验收记录（2026-07-29）；
- [ ] 用固定评测集比较 POC/Kimi Rerank 与 Qwen3 Embedding+Reranker 的质量、P95、
  内存和成本，再批准默认模型 profile。

## Production Gate

- [ ] 固定评测集达到已批准阈值且优于 no-memory baseline；
- [ ] 跨 Tenant/Project、Secret、Stale/Deleted 注入均为 0；
- [ ] 威胁模型和隐私评审通过；
- [ ] 负载、Provider 降级、backlog 和重建压测完成；
- [ ] SLO、告警、dashboard 和值班处理人明确；
- [ ] Backup/PITR、Tombstone replay 和投影重建演练完成；
- [ ] RPO/RTO 有实际演练数据；
- [ ] Migration、canary、rollback 和 incident runbook 演练完成；
- [ ] 数据导出、Forget、Erasure 端到端验收通过；
- [ ] SBOM、镜像签名、漏洞和 secret scan 通过；
- [ ] 发布负责人签署 Production Readiness Review。

## 自动阻断条件

出现以下任意情况不得上线：

- Mem0/Provider 仍被当作权威数据源；
- Candidate/NeedsReview 内容能进入正式 Context；
- Memory 能影响 ACL、Approval 或工具权限；
- 跨租户、Secret、Forgotten/Erased 内容出现一次；
- 无法从 Ledger 重建 Head/Projection；
- 删除依赖 Provider 在线才能阻止读取；
- 只有指标名称，没有评测数据集、阈值或原始报告；
- 未完成恢复演练却声称可生产运行。
