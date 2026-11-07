# 第三阶段真实模型执行报告：校准关键错误触发停止

本次已获用户明确授权，但只完成可用性检查和Judge校准。**15/16整例匹配（93.75%），触发1个关键错误，校准未通过。正式LongMemEval 14题没有运行，所有正式质量指标为null。**

执行代码：`63847f2bda73e97108444e8e0febc977634c6561`；计划seal：`c176fb91445566fe688bf7a284c3b2f54e55b289d0015f5abf355ec5e9346e2f`。
执行时间UTC：2026-11-07T08:57:40.521254+00:00 至 2026-11-07T08:59:42.390474+00:00。

## 固定协议与前置检查

- 模型：SiliconFlow CN / deepseek-ai/DeepSeek-V3.2；实际18个响应model均匹配。服务没有不可变权重revision，返回fingerprint为空。
- temperature=0，enable_thinking=false，thinking_budget=128，JSON输出；校准可见输出512 Token。Reader/Judge Prompt、RRF、参考答案和锁定数据均未改。
- V3 SHA256：`0c3ca2749d095bacf8a44af97c0802b15280b2fbbbf5646c9914b6d634b410c4`；16例人工审核由本次明确授权确认，原JSON和v1/v2原文保留。
- 78项离线回归、离线Rust桥编译、临时PostgreSQL和本地Qwen MPS配置检查通过。原有五个服务未停止；免费数据库探针使用随机临时库，已清理。
- 官方实时价格页及公告已核实输入4/输出6元每百万Token，不采用缓存折扣，详见price-verification.json。没有调用额外模型或重试。

## 门禁结论与根因

门禁同时要求16个有效唯一结果、至少15例accurate/supported整例匹配、关键错误0。本次满足前两项，但不满足第三项，Runner按协议停止。

| 项目 | 结果 |
|---|---|
| 完整标签匹配 | 15/16，93.75% |
| semantic_accuracy标签一致 | 16/16 |
| supported标签一致（含拒答null合同） | 15/16 |
| 关键错误 | 1：partial_answer_marked_unsupported |
| 校准放行 | 否 |

这些是一组给定回答的**评分标签一致性**，不是Reader回答准确率。不得把31/32标签一致改成通过标准，也不能将93.75%宣传为公开Benchmark成绩。

### 唯一失败案例：partial_answer

- 问题：Which two cities did Ada visit?
- 证据：Ada visited Paris and Lyon.
- 参考：Paris and Lyon；待评回答：Paris。
- 预期：accurate=false、supported=true；实际：accurate=false、supported=false。
- Judge理由：The answer 'Paris' is incomplete; the reference requires both 'Paris and Lyon'.

Paris这一已陈述事实直接由证据支持，但回答遗漏Lyon，所以不完整。Judge理由只解释了遗漏，却同时否定支持性；这证明本例发生了“完整性错误污染支持性判断”。不能据此推断所有真实问题都会出现该错误。
原始failure.json中的question_id是最后处理的unnecessary_abstention；实际门禁失败ID以judge-calibration-summary.json的critical_errors为准，明确是partial_answer。原记录均保留，没有改标签或重新调用。

## 16例真实Judge逐题结果

下面A=semantic_accuracy，S=supported；null为不适用。每例均只调用一次，完整输入、输出、usage及费用在traces/calibration/<id>.json。

| Case ID | 预期 A/S | 实际 A/S | 整例匹配 | 关键错误 | Judge理由 |
|---|---|---|---|---|---|
| preference_rubric_positive | true/true | true/true | 是 | 无 | The answer recommends a quiet indoor chess evening, which fully aligns with the reference's requirement for a quiet indoor activity reflecting an interest in chess while avoiding traffic and crowds. All material claims are directly supported by the evidence. |
| full_sentence | true/true | true/true | 是 | 无 | The answer correctly states Ada lives in Paris, matching the reference and fully supported by the evidence. |
| synonym | true/true | true/true | 是 | 无 | The answer 'carbonated water' is equivalent to the reference 'sparkling water' and is fully supported by the evidence. |
| number_format | true/true | true/true | 是 | 无 | The answer '1,000' matches the reference '1000' exactly in meaning, and the citation correctly points to the evidence stating 'There are 1000 files.' |
| date_format | true/true | true/true | 是 | 无 | The response 'February 1, 2024' is an unambiguous date conversion of the reference '2024-02-01' and contains the same factual information without contradiction. |
| supported_explanation | true/true | true/true | 是 | 无 | The answer correctly states Paris and includes the additional detail 'since 2020', which is present in the evidence and does not contradict the reference answer. |
| invented_explanation | false/false | false/false | 是 | 无 | The answer states 'since 2019' but the evidence says 'since 2020', making the date fact incorrect. The core location 'Paris' is correct, but the added detail contradicts the evidence. |
| decimal_error | false/false | false/false | 是 | 无 | The answer '15' contradicts the evidence stating the dose is 1.5 mg. |
| entity_swap | false/false | false/false | 是 | 无 | The answer 'Lyon' is incorrect; the evidence states Ada lives in Paris. |
| correct_abstention | true/null | true/null | 是 | 无 | The question is unanswerable based on the evidence, which contains no information about where Ada lives. The response correctly abstains by stating 'Insufficient evidence.' and setting abstained=true, matching the reference answer 'unknown'. |
| unsupported_guess | false/false | false/false | 是 | 无 | The reference answer is 'unknown' because the evidence does not state where Ada lives. The model's answer 'Paris' is a concrete claim not present in the evidence, making it inaccurate. The claim is also unsupported as it does not follow from the given evidence. |
| lucky_guess | true/false | true/false | 是 | 无 | The answer 'Paris' matches the reference, but the provided evidence ('Ada likes chess') does not support the claim about where Ada lives. |
| arithmetic | true/true | true/true | 是 | 无 | The answer '17 days' matches the reference exactly and is directly derived from the evidence: 10 days + 7 days = 17 days. |
| partial_answer | false/true | false/false | 否 | partial_answer_marked_unsupported | The answer 'Paris' is incomplete; the reference requires both 'Paris and Lyon'. |
| preference_rubric_negative | false/false | false/false | 是 | 无 | The answer recommends attending a busy parade, which contradicts the evidence that Ada wants to avoid traffic noise and crowds. The reference answer requires a quiet indoor activity reflecting an interest in chess. |
| unnecessary_abstention | false/null | false/null | 是 | 无 | The evidence directly states 'Ada lives in Paris.' The question is answerable, but the model abstained, which is incorrect. |

## 调用、Token、延迟与费用

| 角色 | 实际/授权次数 | 输入Token | 输出Token | P50/P95 ms | usage公示价估算元 |
|---|---:|---:|---:|---|---:|
| smoke | 2/2 | 32 | 16 | 1815.969/2003.147 | 0.000224 |
| calibration | 16/16 | 7627 | 806 | 3799.109/18586.258 | 0.035344 |
| reader | 0/56 | 0 | 0 | 未测 | 0 |
| judge | 0/56 | 0 | 0 | 未测 | 0 |

合计18次、输入7659 Token、输出822 Token，总计8481 Token。思维Token=0，缓存命中Token=0。
按usage及输入4/输出6元每百万Token估算 **0.035568元**；不是已核对的供应商账单。全部请求已结算，无未决预留。
硬上限12元；原130次最坏预估11.467264元。停止原因是关键校准错误，不是预算耗尽。未使用额度不授权自动换模型、改Prompt或复跑。
HTTP/API响应成功18/18，客户端重试0、传输超时0。16次校准延迟仅是本次API端到端观测，最大40099.105ms，样本量不足以证明服务SLO；不把它当作检索或用户QA延迟。

## 正式公开子集：未运行

固定LongMemEval-S 14题（12正例、2无答案）仍未产生预测。本次不报告检索质量、Reader回答、拒答、幻觉或Oracle效果，不能将未执行填成0%或仅保留成功样本。

| 计划组 | 正式检索 | Reader | 正式Judge | 质量指标 |
|---|---|---|---|---|
| Oracle Evidence | 未运行 | 未运行 | 未运行 | null |
| BM25 | 未运行 | 未运行 | 未运行 | null |
| TF-IDF（仅检索控制） | 未运行 | 不适用（未计划） | 不适用（未计划） | null |
| Semantic Vector | 未运行 | 未运行 | 未运行 | null |
| Hybrid | 未运行 | 未运行 | 未运行 | null |

正式QA未执行，因此没有“检索成功但回答失败”的真实公开题目案例，也没有真实Reader拒答/幻觉率。不能用本次Judge校准替代这些结论。

## 与历史工程实验的关系

此前Qwen3-Embedding-0.6B在自建195文档/20正例上的Semantic Recall@5=100%、MRR@10=0.925，Hybrid为95%/0.8738095；见../phase3-semantic-holdout/report.md。它们是历史工程集成绩，本次没有重跑，不能混写成新14题或生产效果。
BM25在801块本地固定夹具上的P95约3.05秒→0.20秒及排名一致性证据仍属于第二阶段，不是本轮新测或生产SLO。

## 追溯与复现边界

- calls.jsonl保留18次原始模型choices、usage、响应ID、返回模型、耗时、单次费用和累计费用；未保存Authorization或Key。
- traces保存所有完整请求payload和原始输出。输入由执行前快照及冻结函数重建，逐次与实际账本input_sha256核对一致；不是事后修改题目或提示词。
- runner-manifest.json、failure.json和runner-summary.json保留原始停止结果；budget-claim.json保留原稳定预算标识。
- 可离线重算16例门禁，读取calibration-v3.json与judge-calibration.json调用calibration_summary；不需要任何模型请求。
- 原执行命令、环境变量来源（不含值）及完整性检查见validation.json和authorization.json；不提供删除claim后重开的自动步骤。
- 模型别名及空fingerprint不能固定权重；同模型Reader/Judge的相关偏差仍存在。16个合成校准案例不能证明真实Judge泛化能力；新公开子集本来只有2道负例，也不足以估计稳定拒答率。

## 下一步决策

当前证据已经证明Judge会在partial_answer上混淆完整性与支持性，冻结门禁成功阻止了不可信的正式计分。下一轮若要修改Judge提示、模型或校准协议，必须另行确认并重新封存；不得使用尚未执行的锁定测试结果调参。本轮不修订、不复跑、不新增API调用。

简历只能使用历史已测的工程集/性能证据，以及本次真实校准与门禁事实；不得声称正式公开Benchmark或真实Reader评测已经完成。最终表述见docs/EVAL_RESUME_FINAL.md。
