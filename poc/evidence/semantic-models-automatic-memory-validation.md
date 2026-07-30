# 真实语义模型与自动记忆验收

## 结论

2026-07-29（Asia/Shanghai）在 macOS Apple Silicon 上完成了一次不含凭据的真实链路
验收。结果为通过：自然对话由 Kimi `k3` 提取出偏好，PostgreSQL 受控写入，Qwen3
Embedding 建立投影，独立 Qwen3 Reranker 排序，Context Pack 注入后 Kimi 按偏好
回答；Forget 后新的 Attempt 立即读不到该记忆，随后 Mem0 投影删除完成。

该结果证明当前本机会议 Demo profile 可用，不代表 Production Gate 已通过。

## 固定运行身份

| 组件 | 实际身份 |
|---|---|
| Extractor | Kimi Code `k3` / `kimi-k3-memory-extractor-v1` |
| Answer Model | Kimi Code `k3` |
| Embedding | `Qwen/Qwen3-Embedding-0.6B` @ `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3` |
| Embedding contract | 1024 dimensions, L2, `mem0-symmetric-document-v1` |
| Reranker | `Qwen/Qwen3-Reranker-0.6B` @ `e61197e` |
| Model gateway | loopback `127.0.0.1:18082`, Apple MPS |
| Canonical store | PostgreSQL `memory_core` |
| Retrieval projection | Mem0 collection `memories_qwen3_1024` |
| Active generation | `qwen3-semantic-20260729b` |
| Manifest hash | `926ecc0230c4736f94d365892e1672ea591f25ea12164989308e3aefc1be14ef` |

没有把 API Key、Authorization header、`.env` 正文或模型请求正文写入本证据。

## 模型与投影检查

`tools/smoke_retrieval_models.py` 的真实结果：

```text
embedding dimension: 1024
embedding norms: 1.0, 1.0, 1.0
related similarity: 0.737395
unrelated similarity: 0.300029
rerank order: 0, 1, 2
rerank scores: 0.214217, 0.160614, 0.001056
```

Migration `0013_projection_model_manifest.sql` 与
`0014_extraction_operation_snapshots.sql` 已应用。PostgreSQL 10 个隔离集成测试通过，
包括 generation 重试状态机、Extraction 首次结果快照和低主题分数 Preference 的
Context Pack 类型适用性回归测试。

首次 generation `qwen3-semantic-20260729` 暴露了“可重试 failed 被过早视为 generation
终态失败”的状态机问题；修复后创建全新 generation
`qwen3-semantic-20260729b`，结果为 `expected=3, ready=3, failed=0` 并成功激活。
失败 generation 保留为审计证据，没有篡改历史状态。旧 1536 维 collection 也未删除。

## 完整业务链结果

使用隔离用户 `semantic-demo-20260729` 运行：

```bash
DEMO_USER_ID=semantic-demo-20260729 ./scripts/demo-auto-memory.sh
```

关键结果：

| 阶段 | 验收结果 |
|---|---|
| Canonical write | record `e2039048-7511-5812-b979-30cba9c4eccc` accepted |
| Projection | operation `ce3b5cf3-346d-5f3f-ab11-dcbb345a1026` ready, attempts `1` |
| Context Pack | `a4e876e2-5f7b-52b5-a5fd-c7ecbb099b19`, memory count `1` |
| First answer | Kimi request `chatcmpl-44qDG25PcuLiy3rzUT1bb5iy`; answer used生活类比、`【原理】`、`【示例】` |
| Outcome | receipt `dd92d900-74b5-5c68-a644-e5edc5dccf1a` persisted |
| Forget | operation `f3b11919-e1e2-5a9a-8348-738a119f0027` accepted at revision `1` |
| Second Attempt | Context Pack memory count `0`, `used_memory_ids=[]` |
| Projection delete | Forget operation reached `deleted`, attempts `1` |

最终脚本断言：

```text
Demo Agent v1 PASSED: the first fresh attempt used memory
e2039048-7511-5812-b979-30cba9c4eccc, the second used none.
```

随后把语义服务和自动演示脚本的共同默认身份固定为隔离用户 `semantic-demo`，再次不带
任何额外参数执行 `./scripts/demo-auto-memory.sh`，同样通过：record
`1971d4b1-2c8e-5bf2-9eb0-988445aa7180` 在第一轮被唯一注入，Forget operation
`029287bd-f209-55ff-b2b5-6fa91ce7d8ea` 后第二轮 `memory_context_count=0`，最终投影状态
为 `deleted`。因此文档给出的默认会议演示命令已经按原样验证。

## 验收过程中修复的真实问题

Qwen Reranker 对“Outbox 问题”和“回答时使用生活类比/固定标题”的主题相关分数很低。
这不是错误模型输出：回答风格偏好本来就不是 Outbox 事实。原实现把一个统一语义
阈值同时当成“主题相关性”和“记忆是否适用”，导致合法 Preference 被过滤。

最终规则为：

- Fact/Episode 仍必须达到语义阈值；
- 只有调用方显式请求、已召回并通过 canonical 授权的 `preference` 可由类型适用性
  保留；
- 保留原始模型低分，不抬高或伪造分数；
- 专用 reason code 为 `DEDICATED_RERANK_PREFERENCE_APPLICABILITY`；
- Search、透明降级和 Context Compiler 使用同一策略；
- Rerank 后仍再次执行 PostgreSQL Authority 复查。

这避免了简单降低全局阈值造成的无关 Fact 注入。

## 尚未通过的 Production Gate

- 固定真实业务评测集、正式质量阈值以及 no-memory 对照；
- P50/P95/P99、吞吐、内存、CPU/GPU 和成本数据；
- 目标部署平台 lock、SBOM、模型 artifact 签名和内网分发；
- 外部 OIDC/JWKS、membership/Consent 撤权传播；
- 正式 tokenizer、Context Pack 签名和高可用双 generation 切换；
- 负载、故障、Backup/PITR、RPO/RTO 与 incident 演练。
