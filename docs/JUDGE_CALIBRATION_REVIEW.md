# Judge v2校准案例最终确认清单

此页保留v2人工清单。当前v3仅补充正例室内偏好事实和关键门禁，参考答案与标签不变，见 [v3修正](EVAL_JUDGE_CALIBRATION_V3.md)。

已按用户逐项复核意见整理。新版状态为 **pending_final_confirmation**；用户已确认的原则已应用，新版尚待最终确认，真实Judge尚未运行，收费授权仍为否。

数据：`evals/datasets/judge_calibration_v2.json`；SHA256：`e6ba50bd599e149c2258c7aeb5055ab60607e6b598c1d71f23959d237aa03e0d`。
v1 JSON原文不变；上一版人工清单见 [v1清单快照](JUDGE_CALIBRATION_REVIEW_V1.md)。规则和门禁见 [v2规则说明](EVAL_JUDGE_CALIBRATION_V2.md)。

“正确/错误”指完整回答的semantic_accuracy；“有据/无据/不适用”指supported。以下全部是预期标签，不是模型实测结果。

| Case ID | 原始问题 | 参考及可见事实 | 待评回答 | 最终预期标签 | 修改/保留理由 |
|---|---|---|---|---|---|
| preference_rubric_positive | What could Ada do this evening? | 参考：Recommend a quiet indoor activity reflecting an interest in chess; avoid traffic and crowds.<br>事实：Ada enjoys chess and wants to avoid traffic noise and crowds. | Try a quiet indoor chess evening.<br>abstained=false；citations=["s"] | 正确；有据 | 保留正确、有据；支持性指建议符合用户偏好及已有约束，不是证明某个活动真实存在。 |
| full_sentence | Where does Ada live? | 参考：Paris<br>事实：Ada lives in Paris. | Ada lives in Paris.<br>abstained=false；citations=["s"] | 正确；有据 | 保留正确、有据；完整句与短答案语义等价。 |
| synonym | What does Ada drink with dinner? | 参考：sparkling water<br>事实：Ada drinks plain sparkling water with dinner. | carbonated water<br>abstained=false；citations=["s"] | 正确；有据 | 用sparkling water/carbonated water替换有歧义的bike/bicycle；标签不变。 |
| number_format | How many files? | 参考：1000<br>事实：There are 1000 files. | 1,000<br>abstained=false；citations=["s"] | 正确；有据 | 保留正确、有据；英文千位分隔不改变数值。 |
| date_format | When did it start? | 参考：2024-02-01<br>事实：It started on 2024-02-01. | February 1, 2024<br>abstained=false；citations=["s"] | 正确；有据 | 保留正确、有据；两种格式表示同一日期。 |
| supported_explanation | Where does Ada live? | 参考：Paris<br>事实：Ada has lived in Paris since 2020. | Paris, her home since 2020.<br>abstained=false；citations=["s"] | 正确；有据 | 保留正确、有据；附加2020年事实也得到证据支持。 |
| invented_explanation | Where does Ada live? | 参考：Paris<br>事实：Ada has lived in Paris continuously since 2020 and did not live there before 2020. | Paris, her home since 2019.<br>abstained=false；citations=["s"] | 错误；无据 | 保留错误、无据；明确2020年才开始且此前未居住，使附加2019年事实与证据直接矛盾，测试核心答案正确但完整回答错误。 |
| decimal_error | What is the dose in mg? | 参考：1.5<br>事实：The dose is 1.5 mg. | 15<br>abstained=false；citations=["s"] | 错误；无据 | 保留错误、无据；1.5与15相差十倍。 |
| entity_swap | Where does Ada live? | 参考：Paris<br>事实：Ada lives in Paris. Bea lives in Lyon. | Lyon<br>abstained=false；citations=["s"] | 错误；无据 | 保留错误、无据；Lyon属于Bea，不能移到Ada。 |
| correct_abstention | Where does Ada live? | 参考：unknown<br>事实：Ada likes chess. | Insufficient evidence.<br>abstained=true；citations=[] | 正确拒答；支持性不适用 | 保留正确拒答、supported=null；null为不适用，不代表无幻觉。 |
| unsupported_guess | Where does Ada live? | 参考：unknown<br>事实：Ada likes chess. | Paris<br>abstained=false；citations=["s"] | 错误；无据 | 保留错误、无据；无答案题不能凭空猜测城市。 |
| lucky_guess | Where does Ada live? | 参考：Paris<br>事实：Ada likes chess. | Paris<br>abstained=false；citations=["s"] | 正确；无据 | 保留正确、无据；准确性与证据支持分离。 |
| arithmetic | How many days in total? | 参考：17 days<br>事实：The two breaks did not overlap: the first lasted 10 days and the second lasted 7 days. | 17 days<br>abstained=false；citations=["s"] | 正确；有据 | 明确两段时间不重叠，排除重复计算歧义；保留正确、有据。 |
| partial_answer | Which two cities did Ada visit? | 参考：Paris and Lyon<br>事实：Ada visited Paris and Lyon. | Paris<br>abstained=false；citations=["s"] | 错误；有据 | 保留错误、有据；已陈述Paris属实，但遗漏必须回答的Lyon。 |
| preference_rubric_negative | What could Ada do this evening? | 参考：Recommend a quiet indoor activity reflecting an interest in chess; avoid traffic and crowds.<br>事实：Ada enjoys chess and wants to avoid traffic noise and crowds. | Attend the busy Red Square parade tonight.<br>abstained=false；citations=["s"] | 错误；无据 | 保留错误、无据；违背避开人群的偏好并暗示未提供依据的具体地点与今晚活动。 |
| unnecessary_abstention | Where does Ada live? | 参考：Paris<br>事实：Ada lives in Paris. | Insufficient evidence.<br>abstained=true；citations=[] | 错误拒答；支持性不适用 | 保留错误拒答、supported=null；证据明确却拒答属于未完成问题。 |

## 最终标签矩阵

T/F为true/false；—为不适用。拒答正确性仅在本表已声明拒答的案例中由accurate派生，不另算一次投票。strict/normalized由确定性评分器计算，不由LLM Judge投票。

| Case ID | semantic_accuracy | supported | 拒答正确性 | strict | normalized | 关键错误检查 |
|---|---|---|---|---|---|---|
| preference_rubric_positive | T | T | — | F | F | 无 |
| full_sentence | T | T | — | F | F | 无 |
| synonym | T | T | — | F | F | 无 |
| number_format | T | T | — | F | F | 无 |
| date_format | T | T | — | F | F | 无 |
| supported_explanation | T | T | — | F | F | 无 |
| invented_explanation | F | F | — | F | F | material_false_addition_marked_accurate |
| decimal_error | F | F | — | F | F | 无 |
| entity_swap | F | F | — | F | F | wrong_entity_marked_accurate |
| correct_abstention | T | — | T | T | T | correct_abstention_marked_incorrect |
| unsupported_guess | F | F | — | F | F | unsupported_guess_marked_supported |
| lucky_guess | T | F | — | T | T | lucky_guess_marked_supported |
| arithmetic | T | T | — | T | T | 无 |
| partial_answer | F | T | — | F | F | 无 |
| preference_rubric_negative | F | F | — | F | F | 无 |
| unnecessary_abstention | F | — | F | F | F | unnecessary_abstention_marked_correct |

## 与v1的差异

- synonym：替换问题、参考、上下文和回答，使用sparkling water与carbonated water，去除bike歧义。
- arithmetic：仅补充事实，明确两段时间不重叠。
- invented_explanation：仅补强事实为2020年起持续居住、此前未居住，使2019成为明确错误的附加事实，而非仅仅缺乏支持。
- 其余13例问题、参考、上下文和回答不变；16例原有expected标签全部不变。逐字段before/after及逐例理由保存在v2 JSON的changes中。
- preference_rubric_positive仅明确评分解释：泛化建议的支持性指符合记忆中的偏好及约束，不等于已证明某具体活动存在。

没有修改锁定14题、Reader Prompt、检索权重或上下文策略。本轮无收费API调用；请最终确认v2内容和门禁，未自动生成人工通过或消费批准。
