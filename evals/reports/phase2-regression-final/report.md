# Memory 评测报告

代码基点 `4a9e7632ef9f58c47f49fc6632c7dbd4bb38e147`；分支 `codex/memory-evaluation-system`。实际工作区哈希见 manifest.json。

所有在线调用与费用均为 0。Vector 是 TF-IDF cosine；Rerank 是现有透明词法规则；不是语义模型对比。
公开子集使用原文分块，Formation/自建 QA 使用本地规则。均不是官方完整 Benchmark 或排行榜成绩。

数据库每条语句硬上限 3000 ms。超时保留在质量分母中，回答与拒答均不能因超时算对；延迟含返回失败的耗时，成功查询延迟另见 JSON。

## self_constructed

| 方法 | N / 正例 | 错误 | R@1 | R@5 | MRR@10 | nDCG@10 | P50 ms | P95 ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| bm25 | 36 / 36 | 0 | 0.7639 | 0.9444 | 0.8843 | 0.8940 | 15.7700 | 17.0932 |
| vector_tfidf | 36 / 36 | 0 | 0.8194 | 0.9444 | 0.9097 | 0.9160 | 7.4281 | 9.2237 |
| hybrid | 36 / 36 | 0 | 0.7639 | 0.9444 | 0.8819 | 0.8955 | 15.7700 | 18.5583 |
| hybrid_rerank | 36 / 36 | 0 | 0.7778 | 0.9306 | 0.8981 | 0.8929 | 22.3970 | 30.4320 |

## Formation（规则基线，严格事实别名匹配）

{"n": 24, "tp": 10, "fp": 3, "fn": 6, "precision": 0.7692307692307693, "recall": 0.625, "f1": 0.6896551724137931, "empty_cases": 10, "empty_correct": 8}

## QA（抽取式句子回答，严格 normalized exact match）

```json
{
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
