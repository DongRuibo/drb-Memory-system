# Judge V4 第一阶段：离线执行证据

状态：**已封存，等待第一阶段 32 次明确授权**。本轮收费调用 0、正式 LongMemEval 调用 0、真实 V4 结果 null。没有读取 API Key 或创建真实收费 Claim。

完整实现、边界与后续授权条件见 [执行器报告](../../../docs/EVAL_JUDGE_V4_STAGE1.md)。本报告的模型响应全部来自脚本夹具，不代表语言理解或真实 Judge 质量。

## 封存

| 项目 | 值 |
| --- | --- |
| 协议版本 | judge-v4-stage1-executor-v1 |
| 协议文件 SHA256 | ec4327b6962444e59fd49c7d3456277cfb36d2132f0ec52ea8c18825e6b3b232 |
| Plan canonical seal | 6d0380d443587a0cf34de8a2b2fc73e601385b3a745189d4f9e0755561064ded |
| V4 Prompt canonical digest | ba7ab1acc3aec7b379313b7b20ca61a2226c05feb40418c6fd43022f4eed6d05 |
| 第一阶段请求上限 | smoke 2 + calibration 16 + controls 14 = 32 |
| 第一阶段费用上限 | ¥1.240064 |
| 历史 usage 公示价费用 | ¥0.035568 / 18 次 |
| 含历史的第一阶段最坏累计 | ¥1.275632 |
| 未来正式阶段 | 独立计划、独立授权；本程序允许调用数 0 |

价格为封存假设：输入 ¥4 / 百万 Token、输出 ¥6 / 百万 Token。执行前需重新确认，尚未在本轮读取账户权限或实时探测。总时限45秒、重试0、每次预留先落盘。缺有效新批准时，在读取 Key/创建 Claim 之前拒绝。

## 模拟结果：两个集合分别报告

| 场景 | 模拟调用数 | 结果 |
| --- | ---: | --- |
| success | 32 | 原校准16/16、新对照14/14分别通过；formal_calls=0 |
| gate1-partial-answer | 16 | partial_answer_marked_unsupported，立即停止，新对照未运行 |
| gate2-c01 | 19 | 第一道通过，c01错误后立即停止 |
| gate2-c02 | 20 | 第一道通过，c02错误后立即停止 |
| transport-failure | 3 | 传输失败保留1笔预留，不重试 |
| invalid-json | 3 | 原始非法输出留存，保留1笔预留，不重试 |

见 [机器汇总](simulation-summary.json)、各 simulation-* 目录的 calls.jsonl、traces、results 和 summary。未执行部分保留固定分母，两个集合不合并。所有 mock-response-*、模拟 usage 及成本都不是实际调用或实际支出。模拟交接凭据明确不授权正式实验。临时模拟批准及 Claim 已隔离并随测试临时目录清理，输出中的 approval-record 只是测试审计摘录，不能作为真实批准使用。

## 测试与保护

全部 **119 tests passed、0 skipped**，其中新增25项。[完整日志](tests.log) 包含门禁、缺失/重复结果、拒答null、角色/Token/费用限制、HTTP总超时、失败预留、进程崩溃和两进程竞争。首次受限沙箱的回环端口 PermissionError 保存在 [首次日志](tests-restricted-sandbox.log)，不计通过；之后完整119项运行通过。语法编译、git diff --check通过。

[保护核验](protection-check.json)：原有32个文件无缺失、无字节变化；551个既有路径中只修改本轮明确涉及的README、共享客户端和旧hash回归测试；306个历史报告文件全部未变；旧收费Claim/18次原账本未变；新增真实Claim为0；锁定数据、参考答案、Reader/V3/V4 Prompt和RRF未改。无暂存、提交、推送或main合并。

## 审查入口

- [完整计划及32份输入](plan.json)、[协议](protocol.json)、[Hash汇总](hashes.json)、[V4 Prompt](prompt-v4.txt)。
- [共享客户端相对V3的全部差异](shared-client-v3-to-stage1.diff)、[本轮实现/配置/测试/README差异](implementation.diff)、[历史V3→V4 Prompt差异](v3-to-v4-prompt.diff)。
- [免费复现与未来授权命令](../../README.md)。

联合 Judge 仍看参考答案和证据，support_only_view尚无调用级隔离。新14例未进入Prompt示例，但属于同源合成对照。若再次发生完整性/支持性混淆，本程序直接停止；改用两阶段Judge需要新方案和授权。不得根据真实锁定集成绩修改Prompt、标签、权重或评分规则。

后续112次需另行实现并封存正式执行器，核对第一阶段真实账本及两个门禁，通过人工复核后再明确授权。按现价格假设费用上界¥9.691136，含历史与本阶段上界累计¥10.966768；累计硬上限仍¥12。当前没有这112次的批准或收费Claim。
