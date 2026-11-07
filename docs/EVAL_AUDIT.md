# 代码与旧评测审计（2026-11-06）

基点：`4a9e7632ef9f58c47f49fc6632c7dbd4bb38e147`。工作分支：`codex/memory-evaluation-system`。
开始时根 Cargo.toml/Cargo.lock 已修改，另有 extraction-eval、demo-web、docs 等未跟踪文件；本次不覆盖、提交或清理这些内容。评测桥使用独立 Cargo 工作区。

## 两条执行路径

以代码为准，当前仓库不是只有一个后端：

1. `apps/memory-service/src/main.rs` 的 `mem0_primary` 分支创建 PrimaryAppState，直接进入 primary_router，提前返回。`primary_runtime_extract_memory` 将脱敏后的完整用户/助手消息交给 Mem0 `infer_memories`；检索通过 Mem0，服务用租户+用户派生的 ID 复核归属。这里没有走 PostgreSQL Candidate/Version/Head 和 Context Pack 治理链。README/Compose 声明的部署入口是这条路径；本次没有检查线上部署状态或写入已有实例。
2. `governed` 分支仍有完整 Repository 和 worker。它需要显式选择，不能把这一分支的实验结果写成当前 `mem0_primary` 部署效果。

governed 的自然语言执行链：

```text
runtime extract / completed turn / 单次 extract API
 → 身份、scope、输入清洗、提取快照
 → Kimi/OpenAI-compatible MemoryExtractor
 → 类型/置信度/受控别名归一化/少量规则补全
 → ExtractCandidateProposal
 → Repository.ingest_extracted / ingest_candidate
 → SourceEvent + Candidate + Decision
 → 按 tenant/scope/type/semantic_key 查找身份
 → 完全相同内容和事件时间：复用 Record/Version，附加来源
 → 不同内容：Head contested + needs_review，等待审核或显式更正
 → 新事实：Record + immutable Version + Head + Event + Outbox 同事务写入 PG
 → ProjectionWorker 同步外部 provider，确认后写本地检索文档与词项
 → provider / PG BM25 / PG lexical / 可选 temporal 多路召回
 → weighted RRF → canonical 授权 → Reranker → 再授权
 → compile Context Pack（类型、用途、预算）→ consume 时再次核对状态
 → 外部 Agent 使用上下文生成回答
```

`ConsolidationWorker` 还支持 episode 的异步合并、快照和重试。Memory Service 提供记忆服务，不独立完成通用回答生成；本次 QA 的 Reader 在 evals 中。

## 能力与证据

| 能力 | 实现证据 | 本轮运行证据及限制 |
|---|---|---|
| BM25 | repository.rs `search_bm25_projection`；Jieba 词项表；k1=1.2/b=0.75 | 新临时 PG 上真实执行，非 Python 重写 BM25 |
| 向量召回 | governed 经 Mem0Adapter/外部模型；直接 ES 装配在 main.rs 被禁用 | 模型部署工具存在；本轮未启动真实语义模型。TF-IDF cosine 仅为本地向量数学对照 |
| 混合检索 | api.rs `search_authorized_scopes` 多通道；ranking.rs weighted RRF k=60 | 本轮复用 RRF，采用 BM25 1.15 + TF-IDF 1.0 双路；不等同于生产四路完整流水线 |
| Rerank | TransparentReranker、KimiReranker、HttpReranker | 本轮运行 TransparentReranker。其词法阈值会丢候选，不能宣称运行了神经网络重排 |
| Scope | DemoAuthority、namespace、canonical 再授权；表 FORCE RLS | 真实非超级用户 PG，验证租户/用户/project；未穷举全部身份与 HTTP 鉴权组合 |
| 版本治理 | ingest/correct/decide_candidate/forget/erasure，SQL Head guard | 运行更新、旧候选拦截、冲突审核、遗忘和逻辑擦除 |
| 来源追溯 | SourceEvent→candidate_sources→memory_version_sources | 完全重复写入后一个 Version 有两个来源 |
| 语义去重 | extractor 有少量受控别名 canonicalize；PG 按语义键+内容哈希判断 | 一般同义改写不能自动合并。探针确认其进入 contested；不能宣传为通用语义去重 |
| 时间 | temporal.rs、event_time、temporal projection | 本轮公开对照只保留时间文本和输入顺序，不运行生产 temporal 通道；不能证明时间推理效果 |
| Context Pack | compile/consume、失效、保守字符预算 | 真实编译和消费；更新后上下文只含新事实；不是模型真实 tokenizer 的精确计数 |

治理中“更新”是显式更正或审核后切换，不是对任意新陈述无条件 last-write-wins；这是 LongMemEval knowledge-update 适配时必须正视的语义差别。

## 旧评测的证据质量

- retrieval-v1 为 12 文档/15 正例/3 负例；v2 为 20/19/5。extraction-v1 只有 5 个合成样例。均不足以证明普遍泛化、生产容量或长期交互可靠性。
- `apps/retrieval-eval/src/main.rs:evaluate_metrics` 中 `mean_ndcg_at_k` 只累加首个相关结果的倒对数折扣，没有累计全部相关结果、计算 IDCG、归一化。单相关文档时碰巧接近正确；多相关文档时错误。MRR 也没有明确固定为 @10，top_k 是逐题配置。
- 旧基线包含 local lexical、离线 BM25、provider native、完整服务 hybrid、no-memory、oracle；不是严格的同预算 BM25/Vector/Hybrid/Hybrid+Rerank 消融。服务 hybrid 还可能带 lexical、temporal 和降级。`provider_profile` 是标签，不是模型版本真实性证明。
- 旧 Retrieval 无规范 P50/P95；新评测固定每题总召回名额，单独记录真实延迟。重复测量只增加延迟样本，不增加独立测试题数量。
- `apps/extraction-eval/src/main.rs` 使用关键词覆盖判断 recall，缺少 Precision/FP，对额外错误、重复事实处罚不足；一候选可能覆盖多事实而无需一对一匹配。本轮新增最大二分匹配，重复候选计 FP。
- 旧代码有 oracle 对照，它直接读取相关文档 ID；只有标注为“理想上界”才合理，不能作为真实预测基线。新预测器不会读取 qrels/answer/has_answer。
- 没有证据证明公开数据进入过模型训练；也没有独立盲测/训练隔离记录可排除此事。自建样例与受控提取规则、产品演示内容有重合，存在测试设计偏向风险。不能直接断言“训练集泄漏”，也不能把高分当泛化证据。
- docs/diagrams 的 sources.json 明确基于旧 revision，含已经不存在的部署文件与旧行号；可辅助了解历史，不应作为当前部署证据。
- 原 Repository 的 16 个 PG 集成测试默认 `#[ignore]`；只跑 cargo test 不能声称它们通过。本轮使用独立 Runner 给出另外的真实 PG 断言。

## 最小实现范围与未修改项

新增 Runner、Dataset、Evaluator、Report、公开 Adapter 和评分测试；直接调用现有库，未改生产 API、算法、迁移或部署配置。新代码从不读取 `.env`，无在线模型调用入口，调用上限 0、费用上限 0 美元。

本地投影确认由评测桥模拟外部 provider 回执，之后执行真实 Repository 的 complete_ready/complete_deleted；所以可以验证 PG 投影与 canonical 治理，不能证明 Mem0/ES 网络同步、向量索引持久性、故障恢复或物理擦除。

公开对照把原文分块作为 episodic_summary 候选顺序写入，以避免把离线规则提取的损失混入检索比较。该对照绕过生产 LLM Formation，不应称为生产模型完整端到端验收。自建 QA 则执行“自然语言→本地规则提取→真实存储/检索/Context Pack→抽取式回答”。
