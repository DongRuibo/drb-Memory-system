# 第三阶段真实执行：Judge门禁未通过，正式测试停止

用户已明确授权最多130次、人民币12元。本次实际完成2次可用性检查和16次真实Judge校准，**整例匹配15/16，但partial_answer触发关键错误，按冻结协议停止**。Reader调用0次，正式Judge调用0次，LongMemEval锁定14题没有运行；不能填报正式QA、拒答、幻觉或公开检索成绩。

- [完整实验报告及16例实际判定](../evals/reports/phase3-real-models-20261107/report.md)
- [机器可读汇总](../evals/reports/phase3-real-models-20261107/summary.json)
- [原始调用账本](../evals/reports/phase3-real-models-20261107/calls.jsonl)
- [partial_answer完整输入与输出](../evals/reports/phase3-real-models-20261107/traces/calibration/partial_answer.json)
- [最终简历材料](EVAL_RESUME_FINAL.md)

## 核心证据

问题要求两个城市，证据是“Ada visited Paris and Lyon.”，待评回答只有“Paris”。预期accurate=false、supported=true；实际Judge返回false/false，理由仅指出答案遗漏Lyon。

这暴露的是评分器将答案完整性与已陈述事实的支持性混淆。完整性不足应使准确性失败，却不否定Paris这个事实的证据。不能修改预期标签或接受这次关键错误来放行。

标签一致性分别为accurate 16/16、supported 15/16；整体门禁仍失败。它们是Judge对给定回答的评分一致性，**不是Reader问答正确率**。符合15/16总门槛不抵消关键错误。

## 实际消耗与边界

| 用途 | 次数 | 输入/输出Token | usage公示价估算元 |
|---|---:|---:|---:|
| 可用性检查 | 2 | 32 / 16 | 0.000224 |
| 校准Judge | 16 | 7627 / 806 | 0.035344 |
| Reader、正式Judge | 0 | 0 / 0 | 0 |
| 合计 | 18 | 7659 / 822 | **0.035568** |

估算不等于已核对供应商账单。所有18次请求均有usage并完成结算，思维Token及缓存命中Token均为0，客户端重试0、传输超时0。校准P50/P95为3799.109/18586.258ms，n=16；这是API耗时，不是检索延迟或生产SLO。

模型是SiliconFlow CN的deepseek-ai/DeepSeek-V3.2；18次响应model相同，fingerprint为空，不能固定服务权重。本次没有本地Embedding推理，也没有任何正式Reader输出，不能借校准宣称已经验证长期记忆QA能力。

## 保护与复现

协议、Prompt、RRF、参考答案、V1/V2/V3校准文件和锁定14题均未改；原有32文件保持完整。预算标识与账本保留，不删除claim，不自动续跑。仅使用已核对的同提供方凭据做进程内注入，报告不保存Key。

所有18次完整输入已从冻结函数及执行前快照保存为Trace，并逐次与实际账本input_sha256匹配。原始输出、usage、费用、响应ID、模型名、延迟及失败均保留。可通过读取calibration-v3.json和judge-calibration.json，离线调用calibration_summary重算门禁，无需任何模型API。

收费前78项离线测试、Rust桥编译、临时PG与MPS模型参数检查通过。免费数据库探针只操作随机临时库并已清理；本轮启动的本地模型已停止，原有5个服务保持运行。正式QA因门禁失败未启动，不存在只保留成功题的问题。

原始failure.json的question_id记录最后处理的unnecessary_abstention，真正门禁失败项以judge-calibration-summary.json的critical_errors为准：partial_answer。两份原始记录均不改写。

若要继续，需要用户另行确认下一轮Judge修订/选择及新的执行安排。当前不会改Prompt、放宽评分、增加模型、重试校准或消耗剩余预算。

本轮仅新增执行报告、Trace和简历材料，并同步文档状态；未修改生产业务代码、未合并main、未自动推送新结果。
