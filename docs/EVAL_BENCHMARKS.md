# 公开 Benchmark 接入核对（2026-11-06）

本次接入经典 LongMemEval；其官方仓库已另行发布 V2，本次不把两者混用。这里只核对公开资料和代码，不声称运行 MAB/LoCoMo 或官方完整模型评测。

| 项目 | 官方代码 / 数据 | 许可 | 评分与适配结论 |
|---|---|---|---|
| LongMemEval | [仓库](https://github.com/xiaowu0162/LongMemEval)，[cleaned 数据](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned) | [代码 MIT](https://github.com/xiaowu0162/LongMemEval/blob/main/LICENSE)；数据卡声明 MIT | 首选；独立 Adapter 已接入 S/M/Oracle JSON，实际运行 S 的固定小子集 |
| MemoryAgentBench | [仓库](https://github.com/HUST-AI-HYZ/MemoryAgentBench)，[数据](https://huggingface.co/datasets/ai-hyz/MemoryAgentBench) | [代码 MIT](https://github.com/HUST-AI-HYZ/MemoryAgentBench/blob/main/LICENSE)，数据卡 MIT；派生上游材料仍应逐项保留许可 | 下一阶段优先 Conflict Resolution 和 Accurate Retrieval；本轮未运行 |
| LoCoMo | [仓库](https://github.com/snap-research/locomo)，`data/locomo10.json` | [仓库 LICENSE.txt：CC BY-NC 4.0](https://github.com/snap-research/locomo/blob/main/LICENSE.txt) | 非商业限制需保留；本轮不下载、不混入项目自建数据、不运行 |

核对时 Git HEAD（不等于本次模型结果）：LongMemEval `9e0b455f4ef0e2ab8f2e582289761153549043fc`；MAB `538026089d1a8a8eff05121d0db89b388f360eba`；LoCoMo `3eb6f2c585f5e1699204e3c3bdf7adc5c28cb376`。实际数据版本由报告的文件 SHA-256 固定，防止 main 变更后混淆。

## LongMemEval

官方数据是 500 题的 JSON 数组，包含 question_id/type/question/answer/question_date、haystack_dates/session_ids/sessions、answer_session_ids；turn 包含 role/content，可能带 has_answer 标注。六个 question_type 之外，`_abs` 标识拒答题。S 保留长历史和干扰会话；Oracle 只有证据会话，不能替代 S 的检索评测。[官方格式说明](https://github.com/xiaowu0162/LongMemEval#-dataset-format)

官方 QA 评分入口是 [evaluate_qa.py](https://github.com/xiaowu0162/LongMemEval/blob/main/src/evaluation/evaluate_qa.py)，读取 `{question_id,hypothesis}` 的 JSONL，用指定 LLM judge 判断正确性，按题型使用提示词；拒答题单独处理。[print_qa_metrics.py](https://github.com/xiaowu0162/LongMemEval/blob/main/src/evaluation/print_qa_metrics.py) 聚合结果。本次没有调用这个付费 judge；报告的 normalized exact match 是本地诊断，不能与官方 QA Accuracy 直接比较。

Adapter `evals/memory_eval/longmemeval.py`：

1. 固定种子 20261106，对题型及拒答组轮转抽样；先确定题目，再运行实验，不挑选成功案例。报告保存全部选中 ID、源文件哈希、抽样规则。
2. 每题独立 namespace；按源时间排序，保留全部用户/助手角色及全部干扰会话，不按 evidence/has_answer 过滤。
3. 空白归一化后每轮按 1000 字符分块，日期和角色作为文本前缀，进入当前 `ingest_extracted` 可接受的 Candidate 格式；独立语义键避免错误地把不同轮强行合成一个事实。源日期写入 occurred_at，问题日期作为 Context Pack reference_time。官方日期不含时区，统一按 UTC 解释并在 provenance 中记录；本轮仍不运行生产 temporal 通道。
4. `input` 与 `labels` 分离。存储、检索、Reader 仅收到历史/问题；qrels、答案、has_answer 不能进入预测流程。拒答题的 qrels 为空，不计正例 Recall/MRR/nDCG 分母。
5. 重排返回的是块，检索评分映射到 session_id 并去重；这是“返回的前 10 块所覆盖的会话”的顺序指标，不是独立召回 10 个完整会话，也不是 turn-level 证据充分性。
6. 保留服务原有清洗/拒绝规则。若有块被拒绝，记录输入量、索引量、拒绝原因和脱敏块数，仍保留原题及其标签，不能通过删除失败题抬高成绩。
7. 通过真实 PG、检索、Context Pack 和本地抽取式 Reader 跑通，但不运行生产 LLM Formation、语义 embedding 或官方 judge；报告强制 `official_benchmark_score=false`。

S 源文件 SHA-256：`d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`。数据不纳入仓库；下载上限 330 MB，命令见 README。

## MemoryAgentBench

官方 HF schema 包含 context 字符串、questions 数组、answers 的多答案数组，以及 metadata 中的 qa_pair_ids、question_types、来源和部分 haystack_sessions 等字段。按 Accurate_Retrieval、Test_Time_Learning、Long_Range_Understanding、Conflict_Resolution 划分，不应把每个大 context 当作只有一道独立问题。[官方数据卡](https://huggingface.co/datasets/ai-hyz/MemoryAgentBench/blob/main/README.md)

代码中 `conversation_creator.py` 负责增量分块，`main.py` 调用 [utils/eval_other_utils.py](https://github.com/HUST-AI-HYZ/MemoryAgentBench/blob/main/utils/eval_other_utils.py) 的 metrics_summarization；LongMemEval 和摘要分别有 llm_based_eval 下的 judge 脚本。其任务指标并不统一：部分用 substring exact match、分类/DetectiveQA 用 exact match、推荐用 Recall@5。[官方指标映射](https://github.com/HUST-AI-HYZ/MemoryAgentBench#-clarification-on-evaluation-metrics)

适配应复用“一个 context 增量写入、多次查询”的设计，保留 chunk 顺序和更新语义；FactConsolidation 的冲突处理尤其适合检验当前“新陈述进入 contested”与 benchmark 期望自动更新之间的差别。TTL、长文摘要、推荐需单独任务/评分器，本轮不为凑数据实现通用导入。

## LoCoMo

官方发布 10 段长对话，sample 包含 sample_id、conversation 的 session 日期/turn（speaker、dia_id、text、可选图像）、qa（question、answer、evidence、category），还有已生成的 observation/session_summary。适配时原始对话与生成摘要应作为独立实验组，避免不加说明地使用额外生成信息。[官方格式](https://github.com/snap-research/locomo#data)

[task_eval/evaluate_qa.py](https://github.com/snap-research/locomo/blob/main/task_eval/evaluate_qa.py) 组织预测，[task_eval/evaluation.py](https://github.com/snap-research/locomo/blob/main/task_eval/evaluation.py) 的 eval_question_answering 按题类计算：部分用 token/stem F1，多跳对多答案处理，对抗题按无信息回答处理。不是简单统一 EM，也不是 LongMemEval 的 judge 分数。

之后可把 dia_id 映射为来源证据，按对话隔离用户、按 session 写入，保留 category。多模态信息需要单独界定文本 caption 与图像输入条件。先做文本子集并保留非商业许可，再决定是否扩展。

## 第三阶段独立子集（准备完成，尚未预测）

经典S源SHA不变。原12题仅作开发；新14题按固定seed和题型预先锁定，题目、答案会话及全部历史会话与开发集重叠为0。完整ID、元数据、预处理和seal在 `evals/datasets/longmemeval_phase3_lock.json`，协议见 [第三阶段准备报告](EVAL_PHASE3_PREPARATION.md)。这里不把新的标记证据块代理改称人工事实标注，也不将后续改编rubric Judge称为官方排行榜配置。
