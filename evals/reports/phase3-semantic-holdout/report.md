# 本地真实语义检索工程留出集

195文档、20正例+4负例；已用过的开发留出集，非独立公开测试集。未运行真实Reader、Reranker或Judge。

```json
{
  "bm25": {
    "recall_at_1": 0.65,
    "recall_at_5": 0.9,
    "mrr_at_10": 0.8125,
    "ndcg_at_10": 0.8306128427182248,
    "positive_n": 20,
    "negative_n": 4,
    "errors": 0,
    "retrieval_p50_ms_excluding_embedding": 53.42475,
    "retrieval_p95_ms_excluding_embedding": 60.4610482,
    "negative_any_hit_rate": 1.0,
    "query_embedding_p50_ms_shared": 0.0
  },
  "tfidf": {
    "recall_at_1": 0.65,
    "recall_at_5": 0.95,
    "mrr_at_10": 0.8416666666666666,
    "ndcg_at_10": 0.8635002288026211,
    "positive_n": 20,
    "negative_n": 4,
    "errors": 0,
    "retrieval_p50_ms_excluding_embedding": 11.014396000000001,
    "retrieval_p95_ms_excluding_embedding": 13.44992055,
    "negative_any_hit_rate": 1.0,
    "query_embedding_p50_ms_shared": 0.0
  },
  "semantic": {
    "recall_at_1": 0.775,
    "recall_at_5": 1.0,
    "mrr_at_10": 0.925,
    "ndcg_at_10": 0.9477642955380092,
    "positive_n": 20,
    "negative_n": 4,
    "errors": 0,
    "retrieval_p50_ms_excluding_embedding": 21.303041999999998,
    "retrieval_p95_ms_excluding_embedding": 26.7452875,
    "negative_any_hit_rate": 1.0,
    "query_embedding_p50_ms_shared": 99.1095835
  },
  "hybrid_semantic": {
    "recall_at_1": 0.7,
    "recall_at_5": 0.95,
    "mrr_at_10": 0.8738095238095237,
    "ndcg_at_10": 0.9007456814812217,
    "positive_n": 20,
    "negative_n": 4,
    "errors": 0,
    "retrieval_p50_ms_excluding_embedding": 61.510125,
    "retrieval_p95_ms_excluding_embedding": 70.9369143,
    "negative_any_hit_rate": 1.0,
    "query_embedding_p50_ms_shared": 99.1095835
  }
}
```
