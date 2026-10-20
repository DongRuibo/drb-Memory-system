# Retrieval v2 数据集

## 当前 v2.2.0

20 条合成文档按实际语义写入：7 条 contextual 偏好、2 条 profile_fact、9 条项目
semantic_fact、2 条 episodic_summary。事件使用明确的半开 event_time 区间。
与 v2.1.1 相比，正文、19 个正例、5 个负例、document IDs 和验收阈值不变。
分类、用户／项目范围和排序适用规则变化，因此指标不是旧版完全同条件复现。

评测器显式传递类型，项目事实写入授权 project scope；在线检索和直接 Provider 基线都读取
授权的 user/project scope。负例结果保存在 raw-results.json，非空命中写入 failure-cases.jsonl；
错误或降级不能计为空命中。固定集中没有 global 偏好，通用偏好另行通过接口和实库验证。

2026-10-20 本地缓存 Qwen 实测：Recall@3=1、MRR=1，正例结果无关注入率=0，
负例命中 1/5、遗忘后注入=0；Release Gate **FAIL**。详见
[本轮回执](../../poc/evidence/preference-policy-2026-10-20.md)。

`reranker-answer-support.json` 是发现失败后增加的 16 个诊断样例，不是独立留出评测集。
旧提示词与收紧回答依据的候选提示词均为 11/16；没有采用候选提示词，也没有据此修改阈值。
诊断仅测本地模型的逐对分数，没有执行完整检索、权限、时间过滤或所有类型的服务端适用规则。

## 历史 v2.1.1

`retrieval-v2.1.1` 保留 20 文档、19 正例、5 负例及原门槛。
相较 `retrieval-v2.1.0`，只把两条不同服务的“发布记录”标签分别改成
“结算服务发布记录”和“搜索服务发布记录”。日期、查询和 relevant document IDs 不变。

原因：本评测通过显式偏好入口入库，冒号前的标签构成语义身份；原两条记录共享身份，
会进入 contested 并删除投影，导致评测在摄入阶段退出。修正使不同服务的记录具有不同身份。
它仍是通过偏好入口验证时间过滤的合成检索集，不是原生 episodic_summary 类型的验收。
修改标签会影响 Embedding 和 BM25，因此新指标不能当作旧版数据集的完全同条件复现。
