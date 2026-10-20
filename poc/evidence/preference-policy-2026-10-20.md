# 偏好适用范围与正确事实类型验收

日期：2026-10-20。基线 `2b48ba9`，隔离分支 `codex/resume-pgvector`；代码尚未提交或推送。
本轮没有下载模型、拉取镜像或调用云 API。真实推理复用已有 CPU 镜像、只读本机缓存与
固定 revision 的 Qwen3 Embedding / Reranker 0.6B，容器开启离线模式和内部网络。

## 已实现

- `preference` 在已有 structured_content 中增加 `global / contextual`；旧记录默认 contextual，
  不批量更改历史数据。global 表示无条件的回答语言、结构、详略偏好；场景限制与 global 互斥。
- global 从 canonical 当前版本独立召回，经过本人 scope、权限、来源、有效期、保留期、
  tombstone 和最终版本回查；不被查询事件时间窗口过滤。新增审计通道，不抬高模型分数。
- contextual 必须经过相关性筛选。普通资料、项目事实、事件分别使用原有三种事实类型。
- 显式 HTTP 写入要求 memory_type，缺省拒绝；SDK、Demo 与评测同步。
  这是开发接口契约变更，旧调用方必须补字段，不让聊天最终用户手工选类型。
- 仅修改 applicability 也能 Correction 出新版本，沿用 CAS、Outbox、来源与旧 Pack 失效；
  相同幂等键改变类型或结构化元数据会冲突。
- Migration 0033 只扩展候选回执 channel 约束，不修改旧 migration checksum。
- 抽取提示明确事实／偏好与适用场景，版本号同步更新；尚未进行真实云抽取质量验证。

接口细节见 [分类规则](../../docs/preference-applicability.md)。

## 验证证据

| 验证 | 结果与边界 |
| --- | --- |
| Rust 工作区普通测试 | 147 passed，0 failed；21 项数据库测试默认 ignored |
| PostgreSQL 集成 | 17 项 Repository + 3 项 Memory Service 通过；本阶段再次执行 global 元数据修正及原 Correction/CAS 两项针对性回归，均通过 |
| Demo 数据库 | 前一恢复阶段独立 demo_chat_test 的 1 项通过；本阶段未改 Demo 数据库实现，未重复执行 |
| Clippy | 全工作区、所有 target 通过；仅豁免 Elasticsearch 基线既有 manual_clamp 告警 |
| 真实本地 HTTP 链路 | 10 项断言通过，详见下方 proof；使用真实 Mem0 + pgvector + 缓存 Qwen，不是固定向量替身 |
| 固定语义评测 | v2.2.0：19 个正例、5 个负例，门槛保持不变；Release Gate FAIL |

10 项接口断言覆盖：无类型拒绝、同请求重放、改变适用范围重放拒绝、事实类型保留、
global 跨话题及时间查询、Recall Explain 通道、contextual 饮食偏好在无关查询排除而相关查询保留、
仅元数据修正生成新版本、遗忘后立即拒读。3 条验证记录已逐条遗忘。

追加边界验证通过：本人已有 4 条 global 偏好时，limit=1 且只请求 semantic_fact，
仍返回正确项目事实，global_preference 通道不会占用候选位。该组 5 条记录也已遗忘；
汇总确认本租户 28 个 Head 均为 forgotten，58 条 Outbox 均为 processed，没有待处理项。

本阶段没有重新进行 Web 浏览器演示、自然对话生成、完整 Episode/consolidation 验收或性能测试。
前阶段的构建、SQL invariants 与固定向量链路结果仍保留，不能当作真实生成质量证据。

## 固定集与实际结果

v2.2.0 的 20 条正文、19 个正例、5 个负例和阈值沿用 v2.1.1。
新声明 7 contextual 偏好、2 profile_fact、9 项目 semantic_fact、2 原生 episodic_summary，
并在两个事件上提供明确 event_time。在线服务和 Provider 基线都使用授权 user/project scope。
类型、范围和适用规则变化，因此与 v2.1.1 负例 4/5 的结果不是完全同条件模型性能比较。

| 指标 | 结果 | 门槛 |
| --- | --- | --- |
| Hybrid Recall@3 | 1.0 | ≥ 0.85 |
| Hybrid MRR | 1.0 | ≥ 0.75 |
| 正例返回结果无关注入率 | 0 | 诊断指标 |
| 负例查询命中率 | 0.2（1/5） | 必须 0 |
| 遗忘后注入 | 0 | 必须 0 |
| Release Gate | **FAIL** | 所有门槛通过 |

剩余 n05 问“其他租户的 Context Pack 里有什么？”，却命中本租户合成项目事实 consume_route。
这是把接口说明误判为能提供实际记忆内容的依据；没有返回其他租户数据，不应表述为跨租户泄漏。
固定集中没有 global 偏好，没有通过忽略偏好命中、修改负例或调整阈值制造通过结果。
20 条固定集记录已逐条遗忘，遗忘后检查为 0。

## 排序提示诊断与未采用的尝试

增加 16 个逐对模型分数诊断样例，比较原提示和“必须支持回答”的候选提示。
两者均为 11/16，候选提示仍未解决 n05，也没有稳定改善总体结果，因此已撤回候选提示，
最终代码保留原提示及 v2-preference-applicability 排序策略。

额外诊断暴露两个适用的场景偏好低分和三个缺少回答依据的高分事实／事件例子。
这些是发现失败后构造的诊断，不是独立留出评测；且只测模型逐对分数，没有执行完整检索、
时间过滤、授权或全部服务端类型规则。它们仍说明当前小模型不能可靠处理所有场景，
不能据固定集正例满分声称语义政策已全面验收。

## 证据与复现

- [固定集报告](preference-policy-2026-10-20/comparison-report.md)、[指标](preference-policy-2026-10-20/metric-results.json)、[运行身份](preference-policy-2026-10-20/evaluation-manifest.json)
- [原始返回](preference-policy-2026-10-20/raw-results.json)、[失败案例](preference-policy-2026-10-20/failure-cases.jsonl)、[本轮数据集快照](preference-policy-2026-10-20/dataset-v2.2.0.json)
- [验证摘要与日志索引](preference-policy-2026-10-20/validation-summary.json)
- [只查事实边界断言](preference-policy-2026-10-20/preference-facts-only-proof.json)
- [10 项接口断言](preference-policy-2026-10-20/preference-runtime-proof.json)、[本机模型健康与分数探针](preference-policy-2026-10-20/qwen-models.json)
- [额外诊断原提示结果](preference-policy-2026-10-20/answer-support-baseline.json)、[未采用提示结果](preference-policy-2026-10-20/answer-support-proof.json)、[样例](preference-policy-2026-10-20/reranker-answer-support.json)

离线与实库命令沿用 [恢复说明](../../docs/resume-recovery.md)，使用独立测试库与 `--offline`。
真实检索使用同一文档中的隔离 Compose、完整模型 revision 和端口；将评测租户改为新的
独立租户、EVAL_USER_ID=evaluator、EVAL_PROJECT_ID=lazytt-memory-system-demo、
EVAL_PROVIDER_PROFILE=qwen3-cpu-offline-typed-preference-policy、EVAL_ENFORCE_GATE=true，
EVAL_GENERATION 取该租户实际 active generation，保留新的 EVAL_OUTPUT_DIR，不覆盖旧证据。
测试集 JSON 须使用当前 v2.2.0，预期当前仍因 n05 非零退出。

## 剩余工作

先保留当前分类和失败标准，再单独研究事实是否支持回答的判定，以及场景偏好召回质量，
用独立样本验证召回／误注入权衡。完整自然对话演示还需要可用的聊天与抽取模型，
当前云 API 限制下未执行；不需要提供密钥才能检查本轮结果。

本阶段已停止本轮原生 Memory Service 与 `lazytt-resume-pgvector` 容器，保留独立数据卷、
缓存及私有配置；没有停止或修复原 `lazytt-memory-local` 部署。原工作区修改保持原状。
