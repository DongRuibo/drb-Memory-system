# context-token（待人工确认的工程结果）

非公开Benchmark、非生产HTTP成绩；收费API为0。分母、计数范围及方法见protocol.json。

```json
{
  "bm25": {
    "n": 48,
    "token_successful_queries": 48,
    "token_failed_queries": 0,
    "full_memory_count_mean": 240.0,
    "pack_memory_count_mean": 2.25,
    "full_tokens_mean": 12489.0,
    "pack_tokens_mean": 109.27083333333333,
    "full_tokens_p50": 12489.0,
    "full_tokens_p95": 12489.0,
    "pack_tokens_p50": 50.5,
    "pack_tokens_p95": 500.79999999999995,
    "token_saving_micro": 0.9912506338911575,
    "token_saving_macro": 0.9912506338911576,
    "all_evidence_delivery_rate": 0.9761904761904762
  },
  "vector_tfidf": {
    "n": 48,
    "token_successful_queries": 48,
    "token_failed_queries": 0,
    "full_memory_count_mean": 240.0,
    "pack_memory_count_mean": 1.3125,
    "full_tokens_mean": 12489.0,
    "pack_tokens_mean": 63.166666666666664,
    "full_tokens_p50": 12489.0,
    "full_tokens_p95": 12489.0,
    "pack_tokens_p50": 0.0,
    "pack_tokens_p95": 474.15,
    "token_saving_micro": 0.9949422158165853,
    "token_saving_macro": 0.9949422158165852,
    "all_evidence_delivery_rate": 0.35714285714285715
  },
  "hybrid": {
    "n": 48,
    "token_successful_queries": 48,
    "token_failed_queries": 0,
    "full_memory_count_mean": 240.0,
    "pack_memory_count_mean": 3.7291666666666665,
    "full_tokens_mean": 12489.0,
    "pack_tokens_mean": 179.0625,
    "full_tokens_p50": 12489.0,
    "full_tokens_p95": 12489.0,
    "pack_tokens_p50": 149.5,
    "pack_tokens_p95": 500.79999999999995,
    "token_saving_micro": 0.9856623828969493,
    "token_saving_macro": 0.9856623828969493,
    "all_evidence_delivery_rate": 1.0
  },
  "hybrid_rerank": {
    "n": 48,
    "token_successful_queries": 48,
    "token_failed_queries": 0,
    "full_memory_count_mean": 240.0,
    "pack_memory_count_mean": 3.7291666666666665,
    "full_tokens_mean": 12489.0,
    "pack_tokens_mean": 179.04166666666666,
    "full_tokens_p50": 12489.0,
    "full_tokens_p95": 12489.0,
    "pack_tokens_p50": 149.5,
    "pack_tokens_p95": 498.19999999999993,
    "token_saving_micro": 0.9856640510315745,
    "token_saving_macro": 0.9856640510315744,
    "all_evidence_delivery_rate": 1.0
  }
}
```
