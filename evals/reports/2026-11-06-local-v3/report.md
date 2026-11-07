# Memory 评测报告

代码基点 `4a9e7632ef9f58c47f49fc6632c7dbd4bb38e147`；分支 `codex/memory-evaluation-system`。实际工作区哈希见 manifest.json。

所有在线调用与费用均为 0。Vector 是 TF-IDF cosine；Rerank 是现有透明词法规则；不是语义模型对比。
公开子集使用原文分块，Formation/自建 QA 使用本地规则。均不是官方完整 Benchmark 或排行榜成绩。

数据库每条语句硬上限 3000 ms。超时保留在质量分母中，回答与拒答均不能因超时算对；延迟含返回失败的耗时，成功查询延迟另见 JSON。

## LongMemEval-s public subset

| 方法 | N / 正例 | 错误 | R@1 | R@5 | MRR@10 | nDCG@10 | P50 ms | P95 ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| bm25 | 12 / 10 | 9 | 0.1000 | 0.1500 | 0.1500 | 0.1387 | 3037.9106 | 3078.2980 |
| vector_tfidf | 12 / 10 | 0 | 0.6000 | 0.8500 | 0.8500 | 0.8387 | 66.2659 | 88.5283 |
| hybrid | 12 / 10 | 9 | 0.1000 | 0.1500 | 0.1500 | 0.1387 | 3030.1176 | 3065.8712 |
| hybrid_rerank | 12 / 10 | 7 | 0.1500 | 0.3500 | 0.3000 | 0.2938 | 3007.9105 | 3070.7440 |

## self_constructed

| 方法 | N / 正例 | 错误 | R@1 | R@5 | MRR@10 | nDCG@10 | P50 ms | P95 ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| bm25 | 36 / 36 | 0 | 0.7639 | 0.9444 | 0.8843 | 0.8940 | 13.9966 | 20.3772 |
| vector_tfidf | 36 / 36 | 0 | 0.8194 | 0.9444 | 0.9097 | 0.9160 | 6.8410 | 7.4762 |
| hybrid | 36 / 36 | 0 | 0.7639 | 0.9444 | 0.8819 | 0.8955 | 13.9629 | 20.3695 |
| hybrid_rerank | 36 / 36 | 0 | 0.7778 | 0.9306 | 0.8981 | 0.8929 | 19.8194 | 33.2261 |

## Formation（规则基线，严格事实别名匹配）

{"n": 24, "tp": 10, "fp": 3, "fn": 6, "precision": 0.7692307692307693, "recall": 0.625, "f1": 0.6896551724137931, "empty_cases": 10, "empty_correct": 8}

## QA（抽取式句子回答，严格 normalized exact match）

```json
{
  "LongMemEval-s public subset/bm25": {
    "n": 12,
    "answer_accuracy": 0.0,
    "answerable_accuracy": 0.0,
    "refusal_accuracy": 0.0,
    "refusal_n": 2,
    "error_count": 9,
    "retrieval_hit_rate_answerable": 0.2,
    "stale_fact_error_rate": null,
    "stale_fact_cases": 0
  },
  "LongMemEval-s public subset/hybrid": {
    "n": 12,
    "answer_accuracy": 0.0,
    "answerable_accuracy": 0.0,
    "refusal_accuracy": 0.0,
    "refusal_n": 2,
    "error_count": 9,
    "retrieval_hit_rate_answerable": 0.2,
    "stale_fact_error_rate": null,
    "stale_fact_cases": 0
  },
  "LongMemEval-s public subset/hybrid_rerank": {
    "n": 12,
    "answer_accuracy": 0.0,
    "answerable_accuracy": 0.0,
    "refusal_accuracy": 0.0,
    "refusal_n": 2,
    "error_count": 7,
    "retrieval_hit_rate_answerable": 0.4,
    "stale_fact_error_rate": null,
    "stale_fact_cases": 0
  },
  "LongMemEval-s public subset/vector_tfidf": {
    "n": 12,
    "answer_accuracy": 0.16666666666666666,
    "answerable_accuracy": 0.0,
    "refusal_accuracy": 1.0,
    "refusal_n": 2,
    "error_count": 0,
    "retrieval_hit_rate_answerable": 0.9,
    "stale_fact_error_rate": null,
    "stale_fact_cases": 0
  },
  "self_constructed/hybrid_rerank": {
    "n": 16,
    "answer_accuracy": 0.8125,
    "answerable_accuracy": 0.75,
    "refusal_accuracy": 1.0,
    "refusal_n": 4,
    "error_count": 0,
    "retrieval_hit_rate_answerable": 0.8333333333333334,
    "stale_fact_error_rate": null,
    "stale_fact_cases": 0
  },
  "self_governance_dynamic/hybrid_rerank": {
    "n": 2,
    "answer_accuracy": 1.0,
    "answerable_accuracy": 1.0,
    "refusal_accuracy": 1.0,
    "refusal_n": 1,
    "error_count": 0,
    "retrieval_hit_rate_answerable": 1.0,
    "stale_fact_error_rate": 0.0,
    "stale_fact_cases": 2
  }
}
```

## 治理

自动断言 11/11；语义改写去重为已知缺口，探针通过不表示能力已实现。

同一旧候选集的 Head 闸门消融见 governance.json。遗忘与逻辑擦除不代表备份/WAL/外部索引物理擦除。

逐题结果：retrieval.json / formation.json / qa.json；失败案例：failures.json；版本与参数：manifest.json。

小规模合成集没有独立盲测；公开子集有抽样误差。延迟为本机热启动串行测量，不是生产并发 SLO。
