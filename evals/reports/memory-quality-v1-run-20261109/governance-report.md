# governance（待人工确认的工程结果）

非公开Benchmark、非生产HTTP成绩；收费API为0。分母、计数范围及方法见protocol.json。

```json
{
  "Scope隔离": {
    "n": 3,
    "passed": 3,
    "accuracy": 1.0
  },
  "冲突识别": {
    "n": 3,
    "passed": 2,
    "accuracy": 0.6666666666666666,
    "label_accuracy": 0.6666666666666666,
    "confusion": {
      "tp": 1,
      "tn": 1,
      "fp": 1,
      "fn": 0
    }
  },
  "旧版本注入": {
    "n": 3,
    "passed": 3,
    "accuracy": 1.0
  },
  "明确纠正": {
    "n": 3,
    "passed": 3,
    "accuracy": 1.0
  },
  "来源追溯": {
    "n": 3,
    "passed": 3,
    "accuracy": 1.0
  },
  "版本替代": {
    "n": 3,
    "passed": 3,
    "accuracy": 1.0
  },
  "逻辑遗忘": {
    "n": 3,
    "passed": 3,
    "accuracy": 1.0
  },
  "重复识别": {
    "n": 3,
    "passed": 2,
    "accuracy": 0.6666666666666666,
    "label_accuracy": 0.6666666666666666,
    "confusion": {
      "tp": 1,
      "tn": 1,
      "fp": 0,
      "fn": 1
    }
  }
}
```
