# Judge Evaluation Contract V4离线结果

本轮收费API调用0。V4模型语义效果未验证；没有生成新的模型分数，也没有复用旧授权。

## 真实故障

首次真实Judge在partial_answer上将不完整但有据的Paris回答判为accurate=false、supported=false。原始理由只解释参考要求两个城市，未给出Paris无据的理由。Adapter输入、原始JSON和门禁均正确，错误来自评分维度混淆。参考锚定、先输出Accuracy和共用理由是候选机制，单例不能证明内部因果。

## 最小修复

独立V4适配器只替换Judge系统Prompt：先识别实际断言→仅按证据判断Support→独立判断完整Accuracy；输出示例Support在前，reason分开说明。原schema字段、V3 Prompt、Reader、检索、参考标签及正式Benchmark不变。完整diff见prompt.diff。

## 实际离线结果

- 全部94项测试通过、0跳过，其中16项V4新增。测试只证明输入/协议、历史回放和程序门禁，不证明真实语义稳定。
- 16原校准＋14新对照共30条完整输入，最大字节保守Token界4356，全部小于8192，没有截断。
- 14例新对照七类各2例；预期标签和事实原子待人工复核，未用于Prompt示例，未进入模型输入。
- 旧真实输出回放仍15/16，partial_answer关键错误1，passed=false；未改标签或降低门禁。
- 16有效唯一结果、至少15/16完整匹配、关键错误0保持。14/14新对照闸门只是更严格的待确认追加条件，不并入16分母。

| ID | 类别 | 预期 A/S | 结构及输入检查 | 真实V4结果 |
|---|---|---|---|---|
| c01 | incomplete_supported | false/true | 完成 | null |
| c02 | incomplete_supported | false/true | 完成 | null |
| c03 | incomplete_unsupported | false/false | 完成 | null |
| c04 | incomplete_unsupported | false/false | 完成 | null |
| c05 | complete_supported | true/true | 完成 | null |
| c06 | complete_supported | true/true | 完成 | null |
| c07 | complete_partly_false | false/false | 完成 | null |
| c08 | complete_partly_false | false/false | 完成 | null |
| c09 | entity_binding | false/false | 完成 | null |
| c10 | entity_binding | false/false | 完成 | null |
| c11 | temporal_update | false/false | 完成 | null |
| c12 | temporal_update | true/true | 完成 | null |
| c13 | abstention | true/null | 完成 | null |
| c14 | abstention | false/null | 完成 | null |

## 新轮预算提案（未授权）

| 角色 | 新请求数 | 输入/计费输出上限 | 最坏元 |
|---|---:|---|---:|
| smoke | 2 | 16384/896 | 0.141824 |
| calibration | 16 | 8192/640 | 0.585728 |
| controls | 14 | 8192/640 | 0.512512 |
| reader | 56 | 16384/896 | 3.971072 |
| judge | 56 | 24576/640 | 5.720064 |

原轮18次估算0.035568元已计入。新轮先32次校准及对照最坏1.240064元，累计1.275632元；正式112次最坏9.691136元，只有门禁通过且明确授权覆盖才进入。
新轮总144次最坏10.931200元，含原轮累计162次、最坏10.966768元，低于12元。新增14次独立对照需要新批准，不属于旧130次授权。
角色/价格/输出上限固定，校准短输入上限收紧至8192并逐条完整验证；未改变正式Context或检索。旧claim/ledger保留，新分阶段claim必须一次性，重启不清零；未决请求先对账，累计spent+reserved≤12。当前V4只有离线入口，收费集成尚待接入/封存，未实现或声称账户级跨机器限额。

## 版本与Hash

协议版本：judge-evaluation-contract-v4；文件SHA256：`e99251778bc302c78fcdfed953bfa978272439ea88e340b4e523047e26cd205a`。
V4 Prompt摘要：`6bace48f0794ede9fefb436ba817a764ed4b1da8bd150e343f194afba71055a1`；文本文件SHA：`bbfed114cc91b35412f64e971d3d0791aa38ad63409f66372986f2f6dc2e171d`。
新对照SHA：`82c3e51a279ccdacb41abc1dbdbcbe66b5a3af4cc568ad9b5c1cb1f9e9800f7c`。原V3校准SHA：`0c3ca2749d095bacf8a44af97c0802b15280b2fbbbf5646c9914b6d634b410c4`。

## 模型建议及下一步

保留DeepSeek-V3.2作为一次预注册V4校准候选，不将其视为已经合格的正式Judge。若再次出现关键错误，停止，不反复微调/重试。
若单Prompt继续混淆，可改为不接收参考答案的Support-only Judge加独立Accuracy判定，或人工复核。当前仅提供无参考信息的support_only_view设计输入，不收费调用，也不宣称两阶段更准确。新增调用与合并标准须另行确认。
详见docs/EVAL_JUDGE_CONTRACT_V4.md、docs/JUDGE_V4_CONTROLS_REVIEW.md。当前新对照人工审核及V4执行方案均等待确认。
