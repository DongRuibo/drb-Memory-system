# retrieval（待人工确认的工程结果）

非公开Benchmark、非生产HTTP成绩；收费API为0。分母、计数范围及方法见protocol.json。

```json
{
  "bm25": {
    "n": 48,
    "positive_n": 42,
    "negative_n": 6,
    "errors": 0,
    "recall_at_1": 0.8214285714285714,
    "recall_at_5": 0.9761904761904762,
    "mrr_at_10": 0.9200680272108844,
    "ndcg_at_10": 0.9382786402461621,
    "negative_any_hit_rate": 1.0,
    "retrieval_p50_ms": 83.82362499999999,
    "retrieval_p95_ms": 102.37583519999997,
    "by_category": {
      "中文自然语言改写": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.6666666666666666,
        "recall_at_5": 0.8333333333333334,
        "mrr_at_10": 0.7738095238095238,
        "ndcg_at_10": 0.8273771811507985
      },
      "多条件查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.6666666666666666,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.9166666666666666,
        "ndcg_at_10": 0.9251084237866075
      },
      "多轮上下文查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "实体和专有名词": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "排除条件": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "新旧事实冲突": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.4166666666666667,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.75,
        "ndcg_at_10": 0.8154648767857288
      },
      "无答案查询": {
        "n": 6,
        "positive_n": 0,
        "recall_at_1": null,
        "recall_at_5": null,
        "mrr_at_10": null,
        "ndcg_at_10": null
      },
      "日期和时间": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      }
    }
  },
  "vector_tfidf": {
    "n": 48,
    "positive_n": 42,
    "negative_n": 6,
    "errors": 0,
    "recall_at_1": 0.8095238095238095,
    "recall_at_5": 0.9761904761904762,
    "mrr_at_10": 0.9041950113378684,
    "ndcg_at_10": 0.9287612685101498,
    "negative_any_hit_rate": 1.0,
    "retrieval_p50_ms": 14.2779165,
    "retrieval_p95_ms": 17.122056249999996,
    "by_category": {
      "中文自然语言改写": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.6666666666666666,
        "recall_at_5": 0.8333333333333334,
        "mrr_at_10": 0.7738095238095238,
        "ndcg_at_10": 0.8273771811507985
      },
      "多条件查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.75,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.9166666666666666,
        "ndcg_at_10": 0.9418201549678554
      },
      "多轮上下文查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.8333333333333334,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.8888888888888888,
        "ndcg_at_10": 0.9166666666666666
      },
      "实体和专有名词": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "排除条件": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "新旧事实冲突": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.4166666666666667,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.75,
        "ndcg_at_10": 0.8154648767857288
      },
      "无答案查询": {
        "n": 6,
        "positive_n": 0,
        "recall_at_1": null,
        "recall_at_5": null,
        "mrr_at_10": null,
        "ndcg_at_10": null
      },
      "日期和时间": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      }
    }
  },
  "hybrid": {
    "n": 48,
    "positive_n": 42,
    "negative_n": 6,
    "errors": 0,
    "recall_at_1": 0.8095238095238095,
    "recall_at_5": 0.9761904761904762,
    "mrr_at_10": 0.9087301587301587,
    "ndcg_at_10": 0.9334352942519663,
    "negative_any_hit_rate": 1.0,
    "retrieval_p50_ms": 83.36379199999999,
    "retrieval_p95_ms": 89.88731920000001,
    "by_category": {
      "中文自然语言改写": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.6666666666666666,
        "recall_at_5": 0.8333333333333334,
        "mrr_at_10": 0.7777777777777778,
        "ndcg_at_10": 0.8311894901132466
      },
      "多条件查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.75,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.9166666666666666,
        "ndcg_at_10": 0.9489044006028785
      },
      "多轮上下文查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.8333333333333334,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.9166666666666666,
        "ndcg_at_10": 0.9384882922619097
      },
      "实体和专有名词": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "排除条件": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "新旧事实冲突": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.4166666666666667,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.75,
        "ndcg_at_10": 0.8154648767857288
      },
      "无答案查询": {
        "n": 6,
        "positive_n": 0,
        "recall_at_1": null,
        "recall_at_5": null,
        "mrr_at_10": null,
        "ndcg_at_10": null
      },
      "日期和时间": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      }
    }
  },
  "hybrid_rerank": {
    "n": 48,
    "positive_n": 42,
    "negative_n": 6,
    "errors": 0,
    "recall_at_1": 0.8333333333333334,
    "recall_at_5": 1.0,
    "mrr_at_10": 0.9246031746031746,
    "ndcg_at_10": 0.9446342843831657,
    "negative_any_hit_rate": 1.0,
    "retrieval_p50_ms": 102.64666700000001,
    "retrieval_p95_ms": 131.8876683,
    "by_category": {
      "中文自然语言改写": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.6666666666666666,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.8333333333333334,
        "ndcg_at_10": 0.8769765845238192
      },
      "多条件查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.5833333333333334,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.8333333333333334,
        "ndcg_at_10": 0.880308447229765
      },
      "多轮上下文查询": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.8333333333333334,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.8888888888888888,
        "ndcg_at_10": 0.9166666666666666
      },
      "实体和专有名词": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "排除条件": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      },
      "新旧事实冲突": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 0.75,
        "recall_at_5": 1.0,
        "mrr_at_10": 0.9166666666666666,
        "ndcg_at_10": 0.9384882922619097
      },
      "无答案查询": {
        "n": 6,
        "positive_n": 0,
        "recall_at_1": null,
        "recall_at_5": null,
        "mrr_at_10": null,
        "ndcg_at_10": null
      },
      "日期和时间": {
        "n": 6,
        "positive_n": 6,
        "recall_at_1": 1.0,
        "recall_at_5": 1.0,
        "mrr_at_10": 1.0,
        "ndcg_at_10": 1.0
      }
    }
  }
}
```
