# Judge Evaluation Contract V4：离线修复与待确认提案

本轮收费调用0，未启动模型服务、未读取Key、未创建批准或收费claim。V3 Prompt、16例校准标签、首次真实报告与账本均保留。V4只提供独立Prompt适配、完整输入准备、离线回放和下一轮费用提案；没有收费执行模式，不把离线通过称为模型稳定。

## 1. 真实故障与可证实的根因

证据是首次真实调用的 [partial_answer Trace](../evals/reports/phase3-real-models-20261107/traces/calibration/partial_answer.json)，请求16，输入SHA为510056df250016428631d62fbf953b3e990779c15defb92fa9a3da1fc05a1258。

问题要求两个城市；证据“Ada visited Paris and Lyon.”；待评回答“Paris”。真实响应是accurate=false、supported=false，理由仅为答案不完整、参考要求两个城市。

可以确认：

- 实际输入正确包含证据和待评回答，原始模型JSON已经把supported判false；不是Adapter丢了Lyon、标签颠倒或评分器重新推断了Support。
- 不完整足以使Accuracy失败，但Paris这一已陈述事实有直接证据，故Support不应失败。可观察的错误就是完整性判断污染了支持性判断。
- 解析器保持模型布尔值，整例门禁也正确拒绝这次关键错误，不能通过修改参考标签或门禁来修复它。

尚不能确认模型内部因果。V3虽然多次要求两维独立，但先讲Accuracy、先输出accurate，引用参考并共用一个reason；这些都可能形成锚定或标签关联。真实理由直接提到reference，支持“参考完整性影响Support”的假设，但单例不证明移除参考就一定改好，也不证明输出顺序是唯一原因。

原16例的15/16、Accuracy标签16/16、Support标签15/16仍是原成绩，不因本次Prompt修订改变。

## 2. V3到V4的最小修改

独立实现位于evals/memory_eval/judge4.py；没有改protocol3.py的原Prompt、解析器、门禁，也没有改Reader、Memory、RRF或正式Benchmark。

| V3 | V4 |
|---|---|
| 先讲完整性/Accuracy，再笼统要求Support独立 | 先识别实际断言，再判断Support，最后独立判断Accuracy |
| accurate字段在输出示例中靠前 | supported靠前，JSON字段与类型不变，解析仍不依赖字段顺序 |
| 未明确排除“遗漏的参考事实”进入Support断言集合 | 遗漏不是事实断言；不能因缺少必要答案直接判已述事实无据 |
| 一个brief explanation | reason给简短、分开的Support/Accuracy依据 |
| 定义性说明为主 | 补用户指定的Paris/Lyon、Paris/Rome、猜中无据、错误附加事实及正确拒答对照 |

其余V3规则保留，包括偏好、时间更新、错实体、重要错误附加事实、拒答null与指令隔离。完整逐行差异见evals/reports/judge-contract-v4-locked/prompt.diff。

Support步骤只使用回答实际陈述的实质事实及证据，不把reference_answers、unanswerable标记、答案长度或完整性当证据。短答案仍须绑定问题的主体与时间：对“当前办公室”的裸答构成当前事实，不能用旧记录证明；显式标注过去时间的事实则可以有据。

Accuracy仍要求完整正确回答问题且没有重要错误附加事实。答对核心不能抵消错误附加信息。支持性不是任务完成度，也不是“引用ID存在”。拒答supported=null不等于无幻觉。

此修复是**评分合同和步骤的修订**，真实V4是否有效仍未知。仅声明先提取断言不保证模型按步骤执行；当前保持原输出schema，不自动用关键词提取/匹配事实来冒充语义评分器。

## 3. 对照测试及实际结果的含义

新增14例，七类各2例：[完整人工复核表](JUDGE_V4_CONTROLS_REVIEW.md)。使用与Prompt示例不同的问题、主体、实体和值；包括不完整但有据、不完整且无据、完整有据、完整但部分错误、实体归属、时间更新、合理/不必要拒答。

这些案例由同一开发者手工标注，**待人工复核，不是盲测或官方Benchmark**。标注的事实原子、expected及critical_checks不进入Judge输入。

本轮实际运行的是结构和程序回归：94项离线测试通过、0跳过，其中16项新增。检查输入只替换Judge系统消息、30条完整输入上界、标签/注释不泄漏、旧门禁、缺失/重复/无效结果、费用累计和无收费入口。没有任何真实V4自然语言评分结果；14例语义一致率为null。

30条输入中最大UTF-8保守Token界4413，小于拟议8192上限；没有删证据或截断。首轮真实V3输出经原评分器回放仍为15/16、partial_answer关键错误1、passed=false。此结果证明未绕过门禁，不证明V4已经把false/false改成false/true。

## 4. 门禁及防止过拟合

主校准继续使用原V3数据及SHA，16个有效唯一结果、至少15/16整例完全匹配、关键错误0。partial_answer保留，原有18条关键条件不降级。缺失结果不缩分母，拒答必须显式null。

原16例已经用于诊断，尤其partial_answer现已进入Prompt示例，不能再当独立泛化测试。新14例不进入Prompt示例，下一轮若出现关键错误即停止，不能持续改Prompt/重复到碰巧通过。

费用提案建议先过16例原门禁，再跑14例新对照，并增加14/14整例匹配、关键错误0的**待确认补充门禁**；原16分母不变，不将两组拼成一个更高分数。这项更严格门禁及新增14次调用均不是旧授权内容，需用户确认。

## 5. 下一轮费用和跨轮次预算（尚未授权）

实时官方价格页与公告再次核实输入4元、输出6元每百万Token，不假设缓存折扣。所有Judge输出仍512可见Token，另留128思维Token保险；实际thinking仍禁止。Reader保持768+128。

| 阶段/角色 | 次数 | 单次输入上限 | 单次计费输出预留 | 最坏元 |
|---|---:|---:|---:|---:|
| 可用性检查 | 2 | 16384 | 896 | 0.141824 |
| 原16校准 | 16 | 8192 | 640 | 0.585728 |
| 新对照 | 14 | 8192 | 640 | 0.512512 |
| 正式Reader | 56 | 16384 | 896 | 3.971072 |
| 正式Judge | 56 | 24576 | 640 | 5.720064 |
| 新轮总计 | **144** | 累计2572288 | 累计107008 | **10.931200** |

原校准/新对照是短输入，只有逐条完整字节保守界通过才采用8192预留；正式Judge仍24576，Reader、Context和检索预算不变。费用减少来自更紧的短输入上限，不是改答案、少留证据或放宽评分。

- 首次已结算usage估算：0.035568元（18次）；本轮0元。
- 下次先校准及对照：32次，最坏1.240064元，含首次累计最坏1.275632元。
- 只有校准/对照通过且授权覆盖正式阶段，才有后续112次，最坏9.691136元。
- 新轮合计144次；连首次累计162次，最坏累计**10.966768元**，仍低于12元。

144比旧130多的14次就是新对照，必须新批准；不是自动使用旧剩余授权。报价基于usage及公示价，不是已核对的账户账单，真实执行前须再次核价。

旧预算key、claim、ledger不动。新提案使用judge-contract-v4-trial-01与独立分阶段claim key；key不随输出目录、Prompt或代码hash变化而重置。配置固定上轮receipt SHA和0.035568费用；若有未结算或重复记录，必须先对账，不能清零。新阶段死亡/超时保留最坏预留，已有claim拒绝新目录重开，不自动恢复或重试。正式阶段须核对上轮及校准阶段已花费/未决费用，并限制累计spent+reserved≤12元。

现有Ledger/claim模块可复用；V4付费集成尚未接入/封存，不能拿V3 models-run冒充V4。接入时应使用同一阶段账本，短输入client固定8192，正式client保持原上限，按新角色次数限制及累计余额设置硬界。授权32次不包含112次正式阶段；若用户明确批准全部144次，也必须先通过两个门禁。此处是执行方案，不宣称跨机器账户级限额已经实现。

## 6. 模型选择建议

不再将V3的DeepSeek-V3.2 Judge当作已合格评分器；但也不凭一次失败断言更换模型一定更好。建议保留它作为**一次预注册V4校准候选**，以控制模型变量；不建议直接用于正式14题计分。若仍出现关键错误，停止并报告，不继续微调到通过。

备选是两阶段Accuracy/Support评分，或独立Support-only Judge。后者只接收问题/时间、实际回答及证据，不接收参考答案与unanswerable标记；本轮已提供support_only_view用于信息流审查。它能在输入层隔离参考锚定，但不能保证事实提取或模型判断必然正确，会增加调用与合并规则，需要另行封存预算及人工校准，当前未调用。

小规模也可用人工复核替代未合格Judge；不能在正式结果出来后仅人工挑好结果改分。单次V4通过也不能证明长期稳定；托管权重不可固定、同模型Reader/Judge偏差及小样本局限保留。

## 7. 协议、Hash、文件和复现

当前配置：evals/configs/judge_contract_v4.json，版本judge-evaluation-contract-v4。

文件SHA256：`671ce34a2c1ca6aa7beb8235bc5852dd651238d96bdaa5d3da886ab95ca29179`。

V4 Prompt规范化摘要：`ba7ab1acc3aec7b379313b7b20ca61a2226c05feb40418c6fd43022f4eed6d05`（沿用项目digest规则）；Prompt文本文件SHA：`1bcaf1e55e149a8ac12fadd66b83b9cca91f3761c2e859c26fd602845a59d74e`。

新对照文件SHA：`82c3e51a279ccdacb41abc1dbdbcbe66b5a3af4cc568ad9b5c1cb1f9e9800f7c`。原V3校准SHA仍为`0c3ca2749d095bacf8a44af97c0802b15280b2fbbbf5646c9914b6d634b410c4`。

| 文件 | 目的 |
|---|---|
| evals/memory_eval/judge4.py（新增） | 独立V4 Prompt/输入适配、离线回放和费用提案；没有收费模式 |
| evals/configs/judge_contract_v4.json（新增） | Prompt/数据/代码Hash、门禁和待授权预算 |
| evals/datasets/judge_contract_v4_controls.json（新增） | 14个新对照及待复核标签/事实原子 |
| evals/tests/test_judge_contract_v4.py（新增） | 16项输入、门禁、费用与零网络回归 |
| 本文及JUDGE_V4_CONTROLS_REVIEW.md（新增） | 根因、最小差异、模型建议与逐项复核 |
| evals/README.md | 增加当前V4离线入口及状态 |
| evals/reports/judge-contract-v4-locked（新增） | 最终Prompt、diff、输入、协议、回放、测试和保护校验 |

先前同轮judge-contract-v4、judge-contract-v4-final目录为封存前离线草案，保留但不作为当前协议；无真实模型调用。

```bash
PYTHONPATH=/tmp/drb-phase3-deps python3 -m unittest discover -s evals/tests -v
python3 -m evals.memory_eval.judge4 prepare --output evals/runs/judge-v4-offline-001
python3 -m evals.memory_eval.judge4 evaluate \
  --dataset evals/datasets/judge_calibration_v3.json \
  --responses evals/reports/phase3-real-models-20261107/judge-calibration.json \
  --output evals/runs/judge-v4-v3-replay-001.json
```

evaluate读取已有结果，不发模型请求；V3回放预期仍失败。原有32文件、V1/V2/V3、锁定14题、旧报告及canonical账本的保护检查见最终报告目录validation.json。

**本轮结束在待确认状态。收费调用0；不自动续用旧授权，不改正式测试，不执行模型。**

价格依据：[官方价格页](https://www.siliconflow.cn/pricing)、[官方公告](https://api-docs.siliconflow.cn/docs/release-notes/overview)。
