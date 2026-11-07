# 第二阶段：评测诊断、性能修复与验证

本轮模型 API 调用 0、费用 0；在原独立分支工作，未合并/部署、未连接已有数据库。所有历史结果保持不变。

## 结论

LongMemEval 的 0/10 是多个阶段的问题：原始 cosine 与 Context 相关性阈值混用、会话命中掩盖事实丢失、Context 筛选/预算丢证据、单句 Reader 缺少推理能力，以及严格 EM 不对应官方 rubric judge。修复没有调整答案或放宽主评分。

生产改动仅在 repository.rs：BM25 的 corpus/average/dfs 三个统计 CTE 显式 MATERIALIZED，文档长度按 Version 使用现有索引做 LATERAL MAX；保留原持久预备语句缓存。已有待审候选时，后续新冲突返回业务 Conflict，事务回滚。无索引/迁移变更。

## LongMemEval 逐题诊断

同一固定 12 题：10 可回答、2 拒答。下表为 TF-IDF 控制组；事实覆盖来自预先冻结的人工证据片段，是诊断标注，不是新增的答案别名。

| ID | 问题（截取） | 会话命中 | 标注 turn 命中 | 事实召回 | 相关性重排后 | 最终 Context | 分类 |
|---|---|---|---|---|---|---|---|
| 19b5f2b3_abs | How long was I in Korea for? | False | False | — | — | — | Answer Generation Failure (abstention) |
| c6853660 | Did I mostly recently increase or decrease the limit on the number of cups of coffee in the mor | True | True | 2/2 | 2/2 | 1/2 | Context Construction Failure; Evaluator / Adapter Failure (full fact statement vs short reference) |
| 37f165cf | What was the page count of the two novels I finished in January and March? | True | True | 2/2 | 2/2 | 1/2 | Context Construction Failure |
| 51b23612 | I was going through our previous conversation about political propaganda and humor, and I was w | True | True | 1/1 | 1/1 | 1/1 | Evaluator / Adapter Failure (EM contract) |
| 1a1907b4 | I've been thinking about making a cocktail for an upcoming get-together, but I'm not sure which | False | False | 0/2 | 0/2 | 0/2 | Retrieval Failure; Evaluator / Adapter Failure (rubric vs EM) |
| 1faac195 | Where does my sister Emily live? | True | True | 1/1 | 1/1 | 1/1 | Evaluator / Adapter Failure (EM contract) |
| gpt4_2f584639 | Which gift did I buy first, the necklace for my sister or the photo album for my mom? | True | True | 2/2 | 2/2 | 2/2 | Answer Generation Failure |
| c8090214_abs | How many days before I bought my iPad did I attend the Holiday Market? | False | True | — | — | — | Answer Generation Failure (abstention) |
| d7c942c3 | Is my mom using the same grocery list method as me? | True | True | 1/1 | 1/1 | 1/1 | Evaluator / Adapter Failure (full fact statement vs short reference) |
| 6cb6f249 | How many days did I take social media breaks in total? | True | False | 0/2 | 0/2 | 0/2 | Retrieval Failure |
| 1b9b7252 | I wanted to follow up on our previous conversation about mindfulness techniques. You mentioned  | True | True | 1/1 | 1/1 | 1/1 | Evaluator / Adapter Failure (EM contract) |
| 35a27287 | Can you recommend some interesting cultural events happening around me this weekend? | True | True | 2/2 | 2/2 | 2/2 | Answer Generation Failure; Evaluator / Adapter Failure (rubric vs EM) |

分类允许多个阶段同时出现问题。questions.json 内 categories 是自动候选分类，本文与 failure-classification.json 是复核后的 TF-IDF 分类，不把 EM 合同问题等同事实错误。最终 trace、检索正文、候选分数、Context 正文、遗漏预算及 Oracle Evidence 均保存在 `evals/reports/phase2-longmemeval-final-v3/questions.json` 与 `oracle.json`，最终逐题分类在 `failure-classification.json`。

片段覆盖是诊断代理，不代表每个片段都是回答的必要/充分条件。例如咖啡题虽然旧的一杯片段未进入上下文，但预测已明确陈述 increased；不能把缺少该片段单独当成答案错误的原因。母亲购物清单题也保留了与参考Yes相符的完整事实陈述。这里将严格EM失败与事实错误区分，仍不为这些回答改判主分数或冒充官方judge。

修复前 TF-IDF 的 10 道可回答题中有 9 道最终上下文为空。compile_context_pack 会检查 score≥0.4，而 raw TF-IDF cosine 不是相关性分数。评测 Adapter 现在与生产链一样先调用现有 TransparentReranker，再进入 Context；不降低 0.4，不加入面向这 12 题的规则。该步骤也会排序和过滤，所以四组 QA 不能被解释为仅改变检索通道的严格消融；纯检索指标仍使用原始组内排名。

会话级 Recall 使用返回的前 10 块映射为会话再去重；命中会话不保证命中具体事实或全部证据。1000 字符无重叠分块会切开句子/词，时间和角色前缀也会进入 Reader。4096 的预算是保守字符估计，Context 按整块省略，不是按真实模型 tokenizer 精确计数，也不在块内部裁剪。

Mindful.org、Denver、Nu, pogodi! 等实体可能已经出现在回答中，整句仍不与短参考串逐字一致。严格 EM 的实现和官方答案未改，这应归为评分合同不适合该任务，不能把这些严格失败直接解释为语义错误；人工诊断也没有被替换成新的官方 Accuracy。

时间/算术问题的原文存储为各 turn 独立 episodic_summary，旧事实与新事实可以共存；这条 raw-history 对照没有做生产 LLM 的实体归一化和事实版本更新。未发现正确证据被错误数据库覆盖；实际损失主要在召回、相关性筛选及预算，后续仍需 Reader 做更新/时间推理。

## Oracle Evidence 与评分控制

直接把官方 has_answer turn 提供给原 Reader，另一路用生产 pinned-version 授权与预算构造 Oracle Context；两者与正常检索分开报告。金答案仅用于 identity scorer control，不参与普通 Reader。

```json
{
  "bm25": {
    "n": 12,
    "answerable_n": 10,
    "strict_answerable_correct": 0,
    "session_hits": 9,
    "has_answer_turn_hits": 8,
    "all_fact_spans_retrieved": 8,
    "all_fact_spans_in_context": 6,
    "empty_legacy_contexts": 0,
    "empty_corrected_contexts": 0,
    "timeouts": 0,
    "recall_at_1": 0.6,
    "recall_at_5": 0.85,
    "mrr_at_10": 0.85,
    "ndcg_at_10": 0.830657359638273,
    "p50_ms": 222.5840625,
    "p95_ms": 1108.5023021499992,
    "unanswerable_n": 2,
    "refusal_correct": 0,
    "refusal_accuracy": 0.0,
    "overall_strict_accuracy": 0.0,
    "retrieval_sql_calls": [
      24
    ],
    "retrieval_sql_exec_p50_ms": 195.0807075000048
  },
  "vector_tfidf": {
    "n": 12,
    "answerable_n": 10,
    "strict_answerable_correct": 0,
    "session_hits": 9,
    "has_answer_turn_hits": 8,
    "all_fact_spans_retrieved": 8,
    "all_fact_spans_in_context": 6,
    "empty_legacy_contexts": 9,
    "empty_corrected_contexts": 0,
    "timeouts": 0,
    "recall_at_1": 0.6,
    "recall_at_5": 0.85,
    "mrr_at_10": 0.85,
    "ndcg_at_10": 0.8386852807234542,
    "p50_ms": 71.4083335,
    "p95_ms": 156.49421684999993,
    "unanswerable_n": 2,
    "refusal_correct": 0,
    "refusal_accuracy": 0.0,
    "overall_strict_accuracy": 0.0,
    "retrieval_sql_calls": [
      14
    ],
    "retrieval_sql_exec_p50_ms": 57.23735399998259
  },
  "hybrid": {
    "n": 12,
    "answerable_n": 10,
    "strict_answerable_correct": 0,
    "session_hits": 9,
    "has_answer_turn_hits": 8,
    "all_fact_spans_retrieved": 8,
    "all_fact_spans_in_context": 6,
    "empty_legacy_contexts": 0,
    "empty_corrected_contexts": 0,
    "timeouts": 0,
    "recall_at_1": 0.6,
    "recall_at_5": 0.85,
    "mrr_at_10": 0.85,
    "ndcg_at_10": 0.8386852807234542,
    "p50_ms": 219.98277050000002,
    "p95_ms": 1171.426025149999,
    "unanswerable_n": 2,
    "refusal_correct": 0,
    "refusal_accuracy": 0.0,
    "overall_strict_accuracy": 0.0,
    "retrieval_sql_calls": [
      24
    ],
    "retrieval_sql_exec_p50_ms": 190.7967715000268
  },
  "hybrid_rerank": {
    "n": 12,
    "answerable_n": 10,
    "strict_answerable_correct": 0,
    "session_hits": 9,
    "has_answer_turn_hits": 8,
    "all_fact_spans_retrieved": 8,
    "all_fact_spans_in_context": 6,
    "empty_legacy_contexts": 0,
    "empty_corrected_contexts": 0,
    "timeouts": 0,
    "recall_at_1": 0.5,
    "recall_at_5": 0.85,
    "mrr_at_10": 0.8,
    "ndcg_at_10": 0.7857224139102374,
    "p50_ms": 260.1271875,
    "p95_ms": 1059.818789249999,
    "unanswerable_n": 2,
    "refusal_correct": 0,
    "refusal_accuracy": 0.0,
    "overall_strict_accuracy": 0.0,
    "retrieval_sql_calls": [
      24
    ],
    "retrieval_sql_exec_p50_ms": 186.42943799997738
  },
  "oracle": {
    "answerable_n": 10,
    "direct_reader_strict_correct": 0,
    "scorer_identity_correct": 10,
    "pinned_context_all_fact_spans_delivered": 10,
    "pinned_reader_strict_correct": 0
  }
}
```

Oracle 证明能否将证据送达，而不保证单句选择规则能算出 440+416、10+7、比较相对时间或输出偏好建议。Identity control 用原答案检验评分映射与归一化，不是系统成绩。

TF-IDF 控制组：可回答题会话命中 9/10，标注 turn 命中 8/10，全部复核事实召回 8/10，最终 Context 全事实覆盖 6/10。修复后严格回答仍为 0/10；拒答 0/2，四组结论一致。原来的两道拒答正确来自空 Context 触发规则拒答，修正上下文后该能力缺口被显露，未将其隐去。

## 性能修复前后

冻结同一 SQL/绑定参数，三个历史长度、两类查询、每类三次，分别使用 auto 和 force_generic_plan；每行 n=6 是两类查询×三次，不是六个独立题目。临时角色 statement_timeout=3000ms，失败保留在分母。

| 块数 | 计划 | 版本 | P50 ms | P95 ms | SQL P50/P95 ms | 超时/n | SQL calls |
|---:|---|---|---:|---:|---:|---:|---|
| 100 | auto | before | 10.02 | 87.13 | 3.29/80.47 | 0/6 | [11] |
| 100 | auto | materialized | 10.20 | 15.08 | 3.26/7.90 | 0/6 | [11] |
| 100 | auto | version_lengths | 10.12 | 14.68 | 2.99/7.64 | 0/6 | [11] |
| 100 | force_generic_plan | before | 59.30 | 110.25 | 52.47/102.69 | 0/6 | [11] |
| 100 | force_generic_plan | materialized | 15.02 | 17.52 | 6.20/9.86 | 0/6 | [11] |
| 100 | force_generic_plan | version_lengths | 12.87 | 16.64 | 5.58/9.12 | 0/6 | [11] |
| 400 | auto | before | 907.35 | 1768.45 | 886.21/1747.40 | 0/6 | [11] |
| 400 | auto | materialized | 65.24 | 101.85 | 45.00/81.85 | 0/6 | [11] |
| 400 | auto | version_lengths | 63.37 | 117.10 | 43.67/97.21 | 0/6 | [11] |
| 400 | force_generic_plan | before | 915.01 | 1783.02 | 896.05/1765.05 | 0/6 | [11] |
| 400 | force_generic_plan | materialized | 62.68 | 100.41 | 45.36/81.53 | 0/6 | [11] |
| 400 | force_generic_plan | version_lengths | 59.81 | 98.77 | 42.01/79.49 | 0/6 | [11] |
| 801 | auto | before | 1586.27 | 3047.53 | 1540.96/3000.95 | 3/6 | [10, 11] |
| 801 | auto | materialized | 134.05 | 205.25 | 87.84/157.61 | 0/6 | [11] |
| 801 | auto | version_lengths | 128.30 | 199.82 | 81.76/152.04 | 0/6 | [11] |
| 801 | force_generic_plan | before | 1584.40 | 3045.95 | 1540.43/3001.02 | 3/6 | [10, 11] |
| 801 | force_generic_plan | materialized | 143.89 | 226.65 | 98.13/181.78 | 0/6 | [11] |
| 801 | force_generic_plan | version_lengths | 126.14 | 198.68 | 81.71/152.50 | 0/6 | [11] |

before 为原 SQL；materialized 仅物化三处统计；version_lengths 为最终版本（物化统计+按版本索引取长度）。小规模 auto 的 P50 未保证改善，不能宣传所有规模都更快。

仅物化在完整 12 题的中间重放仍各有 2 次超时。随后检查缓存：SQLx 0.8.6 命名语句可能转为 generic plan，非持久语句每次重新规划。两题各 8 次重复的 residual 对照确实更快，但关闭缓存的完整重放仍各有 1 次超时，故没有作为最终生产修复。

```json
[
  {
    "variant": "materialized",
    "n": 16,
    "timeouts": 0,
    "p50_ms": 87.8568335,
    "p95_ms": 90.37072925
  },
  {
    "variant": "materialized_uncached",
    "n": 16,
    "timeouts": 0,
    "p50_ms": 19.239582999999996,
    "p95_ms": 20.891552499999996
  }
]
```

进一步构造新租户统计不足的规划器夹具：已有 9097 块时 ANALYZE，再插入未见租户的 827 块，不依赖后台自动统计是否及时。总计 9924 块是合成压力夹具，不是扩大的官方样本。custom plan 将目标文档估为 1，实际为 827，长度计算中 57092 条词项与文档产生 47157992 次 join-filter 排除；即使只物化或关闭缓存仍然很慢。

最终把长度计算限定为每个 Version 的索引 MAX，复用 retrieval_terms 现有主键。MAX 保留原始不一致长度处理；无词项文档产生 NULL，AVG 忽略 NULL，与旧查询的长度分母一致。原始缓存保留，避免用全局禁缓存掩盖此根因。最终 custom 计划的 DF 和匹配词项连接仍各有约 117 万次 join-filter 排除，未宣称消除了所有规模敏感性；更大租户/词项数量仍需另测。

```json
{
  "kind": "synthetic multi-tenant planner fixture, not benchmark score",
  "source_question": "35a27287",
  "target_documents": 827,
  "copies": 11,
  "total_projection_documents": 9924,
  "input_ingestion": {
    "input_documents": 827,
    "indexed_documents": 827,
    "redacted_documents": 0,
    "rejected": []
  },
  "memory_sample": "970.7MiB / 4.801GiB",
  "variants": {
    "before": {
      "timeouts": 3,
      "sql_p50_ms": 3001.1849169999996
    },
    "materialized": {
      "timeouts": 1,
      "sql_p50_ms": 2946.519291
    },
    "materialized_uncached": {
      "timeouts": 1,
      "sql_p50_ms": 2936.334416
    },
    "lateral": {
      "timeouts": 0,
      "sql_p50_ms": 2896.0818750000003
    },
    "version_lengths": {
      "timeouts": 0,
      "sql_p50_ms": 153.452042
    }
  }
}
```

同一压力夹具普通查询上限 3 秒；EXPLAIN ANALYZE 为诊断单列 10 秒，含逐节点计时开销，不能混入普通延迟。仅物化 custom 的计划执行约 6816 ms，最终约 342 ms；generic 约 34→25 ms。最终的 827 次版本索引聚合是有界的长度读取，不同于原来每个匹配词项都重算全语料统计。计划与逐次耗时在 phase2-cardinality-lengths/。

以下为原固定 12 题实际重放；前一轮含超时，质量差异部分来自可用性恢复，不能称为语义排序变强。

| 方法 | 原超时 | 仅物化超时 | 禁缓存试验超时 | 最终超时 | 原 R@5 | 最终 R@5 | 最终 MRR@10 | 最终 nDCG@10 | 最终 P50/P95 ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| bm25 | 9 | 2 | 1 | 0 | 0.1500 | 0.8500 | 0.8500 | 0.8307 | 222.58/1108.50 |
| vector_tfidf | 0 | 0 | 0 | 0 | 0.8500 | 0.8500 | 0.8500 | 0.8387 | 71.41/156.49 |
| hybrid | 9 | 2 | 1 | 0 | 0.1500 | 0.8500 | 0.8500 | 0.8387 | 219.98/1171.43 |
| hybrid_rerank | 7 | 2 | 1 | 0 | 0.3500 | 0.8500 | 0.8000 | 0.7857 | 260.13/1059.82 |

102/108 个成功查询的 top-20 排名与独立 Python BM25 参考一致，原始分值误差≤1e-12；超时的旧 SQL 没有被算作通过。最终 SQL 的 36/36 个测量全部可与参考核对，另有包含无词项文档的边界对照。

显式 PREPARE/EXECUTE 的实际计划中，100 块完整问题的旧统计 Aggregate 可执行 340 次，物化后各 1 次。801 块旧完整问题的 EXPLAIN ANALYZE 也触及超时，保留错误；不伪造未完成计划。详见 phase2-performance-final/plans.json。

SQL 次数没有随着块数线性增加；问题是单条 SQL 内的重复计算，不是每条记忆一个网络查询。调用次数包含桥的 generation 元数据、事务、租户/计划设置；失败语句在 pg_stat_statements 的计数与成功不同，不能把 10/11 的差别当优化减少了查询。

BM25 与 Hybrid 在独立实验组里会各自执行召回，这属于不同方法的端到端测量；生产单次召回按 scope 调用一次 BM25，并非同一 scope 内重复查询。RRF 复用候选，不额外重算 DF。桥的 generation 检查及多次授权会增加整体延迟，因此同时保留 SQL 与请求耗时，不能把探针延迟称为生产 HTTP SLO。

资源证据在 resources.json 和计划中。Docker 内存是分阶段采样，不是每条查询峰值；PG work_mem、排序/HashAggregate 内存与临时块 I/O保留于原始计划。物化仅保存 count/average 两行及查询词 DF，未引入全历史额外常驻缓存。

## 留出集与方法审计

新增留出集在生产 SQL 修复前冻结：195 文档、24 查询，20 正例+4 无答案，含同义表达、多跳、时间变化、未解决冲突、相近人名/数字和早期事实。160 条公共运维干扰记录不是按结果筛选。未调整检索参数或 Reader 规则。

| 方法 | R@1 | R@5 | MRR@10 | nDCG@10 | 无答案查询任意命中率 | 检索 P50/P95 ms |
|---|---:|---:|---:|---:|---:|---:|
| bm25 | 0.6500 | 0.9000 | 0.8125 | 0.8306 | 1.0000 | 57.80/65.52 |
| vector_tfidf | 0.6500 | 0.9500 | 0.8417 | 0.8635 | 1.0000 | 11.65/13.65 |
| hybrid | 0.6500 | 0.9500 | 0.8375 | 0.8600 | 1.0000 | 58.61/67.07 |
| hybrid_rerank | 0.7500 | 0.9500 | 0.9000 | 0.9131 | 1.0000 | 66.13/95.47 |

无答案任意命中率是检索误注入诊断，不等于最终回答的幻觉率。留出集是同一开发者编写、未用于调参的工程留出集，不是独立团队盲测；topic 内仍有关联。按独立查询 bootstrap 的 95% 区间保存在 summary.json，样本依然少。

```json
{
  "exact_document_overlap": 0,
  "exact_query_overlap": 0,
  "holdout_tags": {
    "早期事实": 3,
    "同义表达": 3,
    "近义干扰": 7,
    "多跳": 3,
    "时间变化": 3,
    "事实冲突": 1,
    "无答案": 4
  },
  "old": {
    "documents": 40,
    "queries": 36,
    "mean_relevant_token_jaccard": 0.1798098920820206,
    "max_relevant_token_jaccard": 0.5
  },
  "holdout": {
    "documents": 195,
    "queries": 24,
    "mean_relevant_token_jaccard": 0.12042850158986285,
    "max_relevant_token_jaccard": 0.375
  }
}
```

旧集 q23 问弃用数据库的原因，但文档只说已弃用，没有原因；已另存 errata，原题/标签/历史成绩保持不变。原 40/36 数据大多是短陈述，3 组 R@5 相同，区分力不足；词面重合只是部分原因，低噪声和标签数量也影响成绩。不存在已证实的模型训练数据泄漏证据；当前没有真实模型，不能验证预训练污染。

留出集额外暴露了数学参考校验的浮点并列次序问题：两个分数约差4e-15，Python/PG 的尾位舍入可不同。仅数学校验使用1e-12容差；真实排序、指标、答案均未改。最终24题的原SQL/最终SQL真实排名全部一致；原36查询四组×三轮的全部排名与质量指标也完全一致。

Recall/MRR/nDCG 的正例分母、@10 截断、完整 DCG/IDCG、多相关/分级标签测试仍保留。三次重复只用于延迟与确定性检查，不能将独立样本数扩大三倍。原始严格 EM 与答案全部保留。

## 集成测试、兼容性与范围

16 项旧 PG 测试原本有显式 #[ignore]（requires a migrated local PostgreSQL configured by TEST_DATABASE_URL），普通 cargo test 不会运行；本轮 Runner 向非生产临时库注入 TEST_DATABASE_URL，主动执行 --ignored，保留原保护。首次 15/16，失败为已有待审冲突时的唯一键错误；修复后 16/16，非超级用户且无 BYPASSRLS。保留首次失败及最终日志。额外验证事务回滚后无新来源、只保留一个待审候选和一个删除操作；原测试末尾缺少 RLS 租户设置也已补正。

BM25 公式、分词、范围、排序、返回结构及预备语句缓存不变；MATERIALIZED 与按版本索引计算长度改变执行计划，其他PG版本和超出本轮规模仍需验证。重复冲突行为从数据库错误改为明确 409，调用方应先处理既有审核再重试，不会隐式接受第二个候选。这是可见行为收窄，不是支持多候选队列。部署仍需走正常流程；本轮无部署、无生产数据迁移。

新增/修改主要文件：repository.rs（两处必要修复及一个集成测试）、evals/harness/src/main.rs（冻结 SQL 探针/计划/Oracle/相关性）、runner.py（分数合同、阶段 trace、稳定就绪判断）、phase2.py/phase2_report.py、三个诊断/留出/勘误数据文件及相关测试。原 32 个文件不覆盖、不清理、不提交。

| 文件/目录 | 本阶段改动目的 |
|---|---|
| crates/memory-postgres/src/repository.rs | 统计物化、版本索引长度计算；重复待审冲突返回业务错误；修复对应数据库测试及 RLS 断言 |
| evals/harness/src/main.rs | 冻结 SQL 探针、参数化实际计划、词项参考、Oracle pinned Context、遗漏明细 |
| evals/memory_eval/runner.py | QA 相关性合同、阶段 trace、临时 PG TCP 就绪及 pg_stat_statements |
| evals/memory_eval/phase2.py、phase2_report.py | 性能/压力/诊断/留出/PG CLI 与 JSON/Markdown 汇总 |
| evals/datasets/holdout_v2.json | 新增冻结留出集，195 文档/24 查询 |
| evals/datasets/longmemeval_review_v1.json、memory_v1_errata.json | 仅诊断的人工事实片段与原集勘误，不改金答案或原标签 |
| evals/evidence/phase2/*.sql、evals/reports/phase2-* | 原/中间/最终 SQL、失败日志、计划、资源、逐题/汇总及验证结果 |
| evals/tests/test_metrics.py | 原始分数不被 QA 适配覆盖、浮点并列参考的边界测试 |
| README.md、evals/README.md、docs/EVAL_PERFORMANCE.md | 入口、依赖、复现与历史证据链接 |
| docs/EVAL_PHASE2.md、EVAL_REAL_MODELS_PLAN.md、EVAL_INTERVIEW.md | 根因、边界、预算、下一轮条件与真实面试表述 |

## 仍缺的条件与下一阶段

真实语义 Embedding、真实 Reranker、生产 LLM Formation/回答、官方 judge 均未运行。TF-IDF 与透明重排仍是控制组。需要实际模型运行环境、完整 revision/截断计数、分数合同、锁定的新测试题、二次标签复核、预算强制执行的 Reader/judge Adapter 和隔离向量路径验证；详见 [真实模型计划与预算](EVAL_REAL_MODELS_PLAN.md)。

下一轮草案为一对已有 Qwen3 模型本地运行，再做 12 题四组+Oracle 的 60 次 Reader、60 次 judge、24 次 Formation。按文档明确的规划单价上界约 10.153 元，拟总硬上限 20 元；不是报价或本轮授权。本轮实际费用始终为 0。

复现命令与全部输出目录见 [评测 README](../evals/README.md)。
