# 真实语义检索固定评测验证

日期：2026-07-28

环境：本机隔离 Tenant/User、PostgreSQL canonical store、固定 Mem0 POC runtime、
hash-only POC Embedder、Kimi Code 官方 OpenAI-compatible endpoint、模型 `k3`。
所有评测内容均为仓库中的合成数据；凭据未写入源码、日志或评测产物。

## 本次验证回答什么

这次验证不再用“搜索接口返回了内容”代表语义检索有效，而是比较五条固定路径：

```text
no-memory
local PostgreSQL lexical
current Mem0 provider
Mem0 + local candidates + RRF + Memory Service reranker
oracle
```

在线混合路径严格执行：

```text
scope-bounded candidate retrieval
→ canonical authorization
→ Kimi k3 semantic rerank
→ canonical authorization again
→ score threshold
→ API response / Context Pack
```

模型不能创建新候选，也不能改变 Scope、Head、Version 或 Tombstone。外部模型失败
时服务显式降级到 `transparent-lexical-v1`，并返回 degradation reason。

## 固定数据集

`evals/retrieval-v1/dataset.json`：

- version：`retrieval-v1.0.0`；
- 12 条合成 Preference；
- 15 条正查询，其中 5 条使用较难的中文释义；
- 3 条负查询；
- Recall@3 门槛 `0.8`；
- MRR 门槛 `0.7`；
- 负查询命中率门槛 `0`；
- Forgotten 注入数门槛 `0`。

评测使用独立 `retrieval-eval-v1 / evaluator` 身份。程序写入所有合成记录、等待
投影 ready、执行所有对照查询、Forget 全部记录，再重复正查询检查被遗忘记录不会
重新注入。失败或中断路径也会尽力使用同一稳定 ID 清理已写入记录。

## 透明排序对照

Artifact：

```text
target/evals/retrieval-v1/c971379a-06b3-4df8-8c9c-154a6c789e84/
```

主要结果：

| 路径 | Recall@3 | MRR | 无关注入率 |
|---|---:|---:|---:|
| current provider | 0.200 | 0.122 | 0.933 |
| local lexical | 0.933 | 0.933 | 0.659 |
| hybrid transparent rerank | 0.667 | 0.667 | 0.000 |

负查询命中率为 `0.333`，Forgotten 注入数为 `0`，Release Gate 为 **FAIL**。这项
负证据说明 hash-only Embedder 和透明词法排序不能冒充真实语义检索。

## Kimi k3 语义 Reranker

Artifact：

```text
target/evals/retrieval-v1/d7fcbb9b-bdea-4cef-95da-219a36fd6a9a/
```

Manifest：

```text
dataset_version = retrieval-v1.0.0
provider_profile = hash-only-poc+kimi-k3-reranker
generation = demo-v0-generation-1
ranking_version = hybrid-rrf60-kimi-k3-semantic-v1
started_at = 2026-07-28T15:04:13Z
completed_at = 2026-07-28T15:09:15Z
```

主要结果：

| 路径 | Recall@1 | Recall@3 | MRR | nDCG | 无关注入率 |
|---|---:|---:|---:|---:|---:|
| no-memory | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| current provider | 0.133 | 0.333 | 0.211 | 0.242 | 0.889 |
| local lexical | 0.933 | 0.933 | 0.933 | 0.933 | 0.659 |
| hybrid service | 1.000 | 1.000 | 1.000 | 1.000 | 0.000 |
| oracle | 1.000 | 1.000 | 1.000 | 1.000 | 0.000 |

- 3 条负查询命中率：`0`；
- Forget 后 Forgotten 注入数：`0`；
- Quality Gate：通过；
- Safety Gate：通过；
- Release Gate：**PASS**；
- `q11`～`q15` 五条较难释义全部把目标记录排在第一位；
- `failure-cases.jsonl` 中的 11 条失败来自 local/provider 对照，混合服务为 0。

## 复现方式

隔离启动评测专用 Memory Service，并为其配置
`MEMORY_RERANK_MODE=kimi`、`KIMI_RERANK_MODEL=k3` 和安全注入的 Key，然后执行：

```bash
./scripts/eval-retrieval-v1.sh
```

程序写出 `evaluation-manifest.json`、`metric-results.json`、
`raw-results.json`、`failure-cases.jsonl` 和 `comparison-report.md`。搜索响应及
原始结果包含实际 `ranking_version`；如果 Kimi 失败并采用透明 fallback，评测
不会把它误记为 Kimi 正常结果。当前评测器默认使用 2 路、最多 4 路
`EVAL_QUERY_CONCURRENCY` 有界并发，并把并发度写入新运行的 Manifest；上面的
历史 Artifact 是并发改造前的串行基准。

## 结论与边界

真实 Kimi `k3` 语义 Reranker 已经解决当前 hash-only Mem0 在中文释义检索上的
明显缺口，并且没有绕过 canonical 授权或 Forget。它使“多条记忆中找到真正相关
的一条”成为可演示、可回归的链路。

但这仍是 15 条正查询的 smoke，不是 Production 质量证明。尚未完成：

- 按记忆类型、Scope、时间、冲突、噪声和长文本扩展的统计性数据集；
- 真实但脱敏业务分布上的盲测和置信区间；
- 延迟、吞吐、429/5xx、费用和并发压测；
- 外部 Reranker 数据保留、训练使用和地域合规评审；
- 完整多特征排序解释与 Outcome Receipt。
