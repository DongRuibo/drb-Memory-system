# 评测规范

评测用于回答四个问题：该不该记、记得是否正确、能否在需要时找回、是否实际帮助 Agent 且不造成伤害。

## 数据集

评测集必须版本化，并为每条样本保存输入、Scope、时间、预期 Candidate/Decision、允许读取者、预期当前事实和禁止行为。

首个 MVP 数据集 SHOULD 覆盖：

- 明确偏好、隐式偏好、临时信息；
- 稳定项目事实、事实更新、时间范围、冲突；
- 精确关键词、中文同义表达、无关相似文本；
- Secret、第三方 PII、Prompt Injection；
- Tenant/User/Project/Agent 隔离；
- 修正、Forget、Erasure、撤权和旧索引；
- Provider 超时、重复事件和并发更新；
- Procedural Candidate 不自动执行。

20～50 条只用于 smoke POC；MVP gate 使用的固定集必须足以覆盖上述每一类别的正例和负例，并报告置信区间或样本数量，不能只报告总平均。

## 对照组

每次效果评测至少比较：

1. no-memory baseline；
2. local exact/FTS baseline；
3. 当前 Provider；
4. oracle context 上界。

Provider 更换、模型/Embedding/Reranker/Prompt/Policy 版本变化必须跑同一数据集。

## 当前可执行的 Retrieval v2 smoke

仓库内 `evals/retrieval-v2/dataset.json` 固定为 `retrieval-v2.0.0`，包含 18 条
合成记忆、18 条正查询和 5 条负查询。除中文释义外，v2 增加错误码、接口路径、
表名、中文项目代号和 tokenizer version 等精确标识符。隔离的评测
Tenant/User 写入完成并等待投影 ready 后，执行：

```bash
./scripts/eval-retrieval-v2.sh
```

评测程序比较 `no_memory`、`local_lexical`、`local_chinese_bm25`、`current_provider`、
`hybrid_service` 和 `oracle`，随后 Forget 所有合成记录并再次检索，确保遗忘记录
注入数为 0。查询默认使用 2 路、最多 4 路有界并发；可通过
`EVAL_QUERY_CONCURRENCY=1..4` 调整，实际值写入 Manifest。每次运行生成：

```text
target/evals/retrieval-v2/<evaluation-id>/
  evaluation-manifest.json
  metric-results.json
  raw-results.json
  failure-cases.jsonl
  comparison-report.md
```

Retrieval v1 仍保留为历史回归集，可用 `./scripts/eval-retrieval-v1.sh` 执行。
2026-07-28 的 v1 透明排序对照为 Recall@3/MRR `0.667/0.667`，Release Gate 未通过；
相同数据集接入 `kimi-k3-semantic-v1` 后，混合服务 Recall@1、Recall@3、MRR、
nDCG 均为 `1.000`，无关注入率、负查询命中率和 Forgotten 注入数均为 `0`，smoke
Gate 通过。完整证据见
[真实语义检索固定评测](../poc/evidence/demo-retrieval-evaluation-validation.md)。

这组 18 条正查询只能证明实现与回归门槛可运行，不能用于声明生产检索质量。MVP/
Production gate 仍需按类别扩充样本、加入真实但脱敏的代表性分布、报告不确定性，
并进行延迟、费用、限流和供应商失败评测。

## 指标定义

### 写入

- `Write Precision = 正确接受 / 全部接受`；
- `Write Recall = 应接受且被接受 / 全部应接受`；
- `False Memory Rate = 错误或无来源接受 / 全部接受`；
- `Secret Escape Rate = 进入 Provider 或正式存储的 Secret / Secret 样本`；
- Duplicate、Incorrect Update、Conflict Detection 分类别报告。

### 检索与 Context

- Recall@K、MRR、nDCG；
- Current Fact Accuracy；
- Relevant/Irrelevant Injection Rate；
- Stale/Contested Injection Rate；
- Scope Isolation Accuracy；
- Token efficiency：有效注入 token / 总记忆 token。

### Agent 结果

- Task Success delta 相对 no-memory；
- Memory Helpful/Harm Rate；
- Citation/Source correctness；
- Unauthorized action delta 必须为 0。

## MVP 建议门槛

正式数值在 POC 基线后由团队批准并写入本节。批准前以下是不可协商门槛：

- 跨 Tenant/Project 越权召回：0；
- Secret Escape：0；
- Forgotten/Erased/Stale Version 注入：0；
- Memory 导致权限提升或自动执行：0；
- 所有注入项具备可解析的 Version 和 Source 引用：100%。

其余质量指标不得写“效果良好”，必须记录数据集版本、样本数、模型/Provider 版本、原始结果和阈值。

## 评测产物

每次发布保留：

```text
evaluation-manifest.json
dataset-version
system/provider/model versions
metric-results.json
failure-cases.jsonl
comparison-report.md
```

LLM Judge 只能辅助评分；关键安全、Scope、版本、删除和来源正确性必须由确定性断言判断。
