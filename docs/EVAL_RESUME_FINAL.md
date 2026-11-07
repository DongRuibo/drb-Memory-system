# Memory项目最终简历材料：只使用已测证据

当前状态：已完成分阶段评测建设、性能诊断与本地语义工程对照；首次收费运行完成真实Judge校准，但关键错误门禁未通过，正式LongMemEval 14题与真实Reader实验未执行。

## 可选的3条项目描述

1. 为Rust/PostgreSQL长期记忆模块构建隔离评测体系，覆盖Formation、Retrieval、Governance和QA；通过SQL执行计划定位统计聚合重复执行与基数估计问题，复用已有索引优化BM25，在801块固定本地夹具中将请求P95由约3.05秒降至0.20秒，并保留超时及排名一致性证据。
2. 在统一20候选预算下对比BM25、TF-IDF、真实Qwen3语义向量和RRF混合；自建195文档/20正例工程集上，Semantic Recall@5为100%、MRR@10为0.925，Hybrid Recall@5为95%，通过逐题候选及融合贡献分析定位融合退化。
3. 实现模型评测的按角色调用限制、请求前持久费用预留和关键错误门禁；在16例真实Judge校准中发现完整性与证据支持性的混淆并自动阻断正式计分，保留18次调用的完整Trace、Token与费用记录，避免将不可信评分用于Benchmark结论。

第一条的耗时来自固定本地夹具，不是生产SLO；第二条是自建工程集，不是盲测或官方公开Benchmark；第三条是校准及控制流程的实际验证，不是已完成真实Reader评测。简历可选2条；面试时需能说明这些边界。

## 面试时应主动说明

- 完整公开Benchmark成绩尚无；旧LongMemEval 12题用于开发诊断，新锁定14题未产生预测。
- 真实Judge整例匹配15/16（93.75%），但partial_answer的supported误判属于关键错误，不能将这次运行称为“校准通过”。
- 实际收费请求18次，输入7659、输出822 Token，按usage和公示价估算0.035568元，不是核对过的账户账单。
- Reader和正式Judge均为0次；Oracle、正式回答准确率、拒答率、幻觉率以及新公开集检索对照均未验证。
- 模型是托管别名，权重版本不可固定；Reader/Judge同模型的相关偏差以及小样本限制依旧存在。

## 证据入口

- 性能、治理和PG集成测试：[第二阶段报告](EVAL_PHASE2.md)
- 自建真实语义对照：[工程集报告](../evals/reports/phase3-semantic-holdout/report.md)
- 本次真实执行与停止原因：[完整报告](../evals/reports/phase3-real-models-20261107/report.md)
- 面试解释、指标及追问：[EVAL_INTERVIEW](EVAL_INTERVIEW.md)

不要使用“完成真实LLM＋LongMemEval对照”“官方排行榜成绩”“生产级验证”“Hybrid必然更好”“Judge已可靠校准”或“真实Reader拒答/幻觉已验证”等表述。
