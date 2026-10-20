# Retrieval Evaluation Comparison

Dataset `retrieval-holdout-v3.0.0`，样本 20 条；Release Gate：**PASS**。

| 方法 | Recall@1 | Recall@3 | MRR | nDCG | 无关注入率 |
|---|---:|---:|---:|---:|---:|
| current_provider | 0.775 | 0.800 | 0.850 | 0.850 | 0.700 |
| hybrid_service | 0.925 | 1.000 | 1.000 | 1.000 | 0.000 |
| local_chinese_bm25 | 0.600 | 0.675 | 0.675 | 0.682 | 0.615 |
| local_lexical | 0.700 | 0.725 | 0.750 | 0.750 | 0.724 |
| no_memory | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| oracle | 0.925 | 1.000 | 1.000 | 1.000 | 0.000 |

- Negative query hit rate: 0.000
- Forgotten injection count: 0
- Quality gate: true
- Safety gate: true
- Failure cases: 14

本报告只使用合成数据。`local_chinese_bm25` 表示使用相同 Jieba 规则的离线 BM25；`current_provider` 表示直接 Mem0 结果；`hybrid_service` 表示在线 Memory API 的 Provider + PostgreSQL 本地候选、RRF、canonical 重新授权和服务当前配置的 Reranker。
