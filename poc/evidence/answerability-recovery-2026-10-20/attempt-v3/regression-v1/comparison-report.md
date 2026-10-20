# Retrieval Evaluation Comparison

Dataset `retrieval-holdout-v1.0.0`，样本 24 条；Release Gate：**FAIL**。

| 方法 | Recall@1 | Recall@3 | MRR | nDCG | 无关注入率 |
|---|---:|---:|---:|---:|---:|
| current_provider | 0.917 | 0.958 | 0.958 | 0.958 | 0.653 |
| hybrid_service | 0.771 | 0.812 | 0.833 | 0.833 | 0.000 |
| local_chinese_bm25 | 0.792 | 0.833 | 0.833 | 0.833 | 0.560 |
| local_lexical | 0.875 | 0.958 | 0.931 | 0.938 | 0.643 |
| no_memory | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| oracle | 0.938 | 1.000 | 1.000 | 1.000 | 0.000 |

- Negative query hit rate: 0.000
- Forgotten injection count: 0
- Quality gate: false
- Safety gate: true
- Failure cases: 10

本报告只使用合成数据。`local_chinese_bm25` 表示使用相同 Jieba 规则的离线 BM25；`current_provider` 表示直接 Mem0 结果；`hybrid_service` 表示在线 Memory API 的 Provider + PostgreSQL 本地候选、RRF、canonical 重新授权和服务当前配置的 Reranker。
