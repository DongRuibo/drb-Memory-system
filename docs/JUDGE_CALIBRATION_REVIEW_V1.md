# Judge校准案例人工复核清单

源文件：`evals/datasets/judge_calibration_v1.json`；SHA256：`93ab2e268dfa40a86a4fc38dee1f2c84789b34bed51b14b61bce598e97d560a1`。
以下逐字保留问题、参考答案、上下文和待评回答；预期语义/支持标签照录。理由及歧义是本次解释，不是新标签。
**16项均待人工复核。没有修改源JSON，没有宣布复核通过。**

表中“正确/错误”指现有semantic_accuracy；“有据/无据/不适用”指supported=true/false/null。严格与保守规范化匹配另报：完整句、同义、数字/日期改写即使语义正确，也可能不匹配。无答案题的strict/normalized使用拒答声明诊断，不是原串相等；正式拒答正确性仍需Judge确认。

## 语义等价与格式（4例）

| Case ID | 用户原始问题 | 标准事实或参考答案（含可见证据） | 待评测回答 | 现有预期判断 | 预期理由 | 评分歧义/待确认 |
|---|---|---|---|---|---|---|
| full_sentence | Where does Ada live? | 参考：Paris<br>证据：Ada lives in Paris. | Ada lives in Paris.<br>abstained=false；citations=["s"] | 正确；有据 | 完整句表达同一城市，没有增加实质事实。 | 无明显语义歧义；字符串EM不匹配是预期行为。 |
| synonym | What does Ada ride? | 参考：bicycle<br>证据：Ada rides a bike. | a bike<br>abstained=false；citations=["s"] | 正确；有据 | 现标签把bike解释为bicycle，视为同义表达。 | bike也可指摩托；需要确认本例按自行车解释。 |
| number_format | How many files? | 参考：1000<br>证据：There are 1000 files. | 1,000<br>abstained=false；citations=["s"] | 正确；有据 | 英文计数中的千位逗号不改变1000这个数。 | 按英文千位分隔解释，不采用其他地区的小数逗号习惯。 |
| date_format | When did it start? | 参考：2024-02-01<br>证据：It started on 2024-02-01. | February 1, 2024<br>abstained=false；citations=["s"] | 正确；有据 | 两个格式均指2024年2月1日。 | 无明显歧义；不涉及时区转换或含糊的月/日格式。 |

## 解释与事实错误（4例）

| Case ID | 用户原始问题 | 标准事实或参考答案（含可见证据） | 待评测回答 | 现有预期判断 | 预期理由 | 评分歧义/待确认 |
|---|---|---|---|---|---|---|
| supported_explanation | Where does Ada live? | 参考：Paris<br>证据：Ada has lived in Paris since 2020. | Paris, her home since 2020.<br>abstained=false；citations=["s"] | 正确；有据 | 城市和新增的2020起居住时间均有证据。 | 请确认允许有证据的多余解释，不要求只答一个词。 |
| invented_explanation | Where does Ada live? | 参考：Paris<br>证据：Ada moved to Paris in 2020. | Paris, her home since 2019.<br>abstained=false；citations=["s"] | 错误；无据 | 城市正确，但新增2019起居住的说法未获证据支持。 | “迁入2020”不必逻辑排除所有早期居住；当前标签按整答包含无依据细节判错。 |
| decimal_error | What is the dose in mg? | 参考：1.5<br>证据：The dose is 1.5 mg. | 15<br>abstained=false；citations=["s"] | 错误；无据 | 15与1.5相差十倍，不能因文字相似判对。 | 无明显歧义；规范化必须保留小数点。 |
| entity_swap | Where does Ada live? | 参考：Paris<br>证据：Ada lives in Paris. Bea lives in Lyon. | Lyon<br>abstained=false；citations=["s"] | 错误；无据 | Lyon属于Bea，不是问题中的Ada。 | 不能仅因城市在上下文出现，就认定支持Ada住在那里。 |

## 拒答、支持性与完整性（6例）

| Case ID | 用户原始问题 | 标准事实或参考答案（含可见证据） | 待评测回答 | 现有预期判断 | 预期理由 | 评分歧义/待确认 |
|---|---|---|---|---|---|---|
| correct_abstention | Where does Ada live? | 参考：unknown<br>证据：Ada likes chess. | Insufficient evidence.<br>abstained=true；citations=[] | 正确拒答；支持性不适用 | 上下文只有兴趣，没有居住城市；明确拒答符合要求。 | supported=null表示不适用，不是“证据支持为假”。 |
| unsupported_guess | Where does Ada live? | 参考：unknown<br>证据：Ada likes chess. | Paris<br>abstained=false；citations=["s"] | 错误；无据 | 没有城市证据，却给出Paris，且题目被标为无答案。 | 引用s不代表s支持城市结论。 |
| lucky_guess | Where does Ada live? | 参考：Paris<br>证据：Ada likes chess. | Paris<br>abstained=false；citations=["s"] | 正确；无据 | 碰巧命中参考Paris，但可见证据没有城市信息。 | 当前标签为准确但无依据；若要评估Memory可靠性，可另报二者联合，不应混成一个分数。 |
| arithmetic | How many days in total? | 参考：17 days<br>证据：Two breaks lasted 10 days and 7 days. | 17 days<br>abstained=false；citations=["s"] | 正确；有据 | 现标签按10+7得到17天，并允许从事实推导。 | 两段是否不重叠未明示；请确认本例允许直接相加。 |
| partial_answer | Which two cities did Ada visit? | 参考：Paris and Lyon<br>证据：Ada visited Paris and Lyon. | Paris<br>abstained=false；citations=["s"] | 错误；有据 | Paris这部分有证据，但遗漏第二个必要城市Lyon。 | 准确性按必须答全判错；支持性只评价已经说出的事实，故为真。 |
| unnecessary_abstention | Where does Ada live? | 参考：Paris<br>证据：Ada lives in Paris. | Insufficient evidence.<br>abstained=true；citations=[] | 错误拒答；支持性不适用 | 城市事实已经明确，仍拒答，未完成任务。 | 错误拒答不等于产生无依据事实；supported保留null。 |

## 偏好rubric（2例）

| Case ID | 用户原始问题 | 标准事实或参考答案（含可见证据） | 待评测回答 | 现有预期判断 | 预期理由 | 评分歧义/待确认 |
|---|---|---|---|---|---|---|
| preference_rubric_positive | What could Ada do this evening? | 参考：Recommend a quiet indoor activity reflecting an interest in chess; avoid traffic and crowds.<br>证据：Ada enjoys chess and wants to avoid traffic noise and crowds. | Try a quiet indoor chess evening.<br>abstained=false；citations=["s"] | 正确；有据 | 建议围绕安静、室内及棋类兴趣，现标签将其判为满足rubric。 | “quiet indoor”是否足以隐含避开人群？泛化建议的支持性标准需你确认。 |
| preference_rubric_negative | What could Ada do this evening? | 参考：Recommend a quiet indoor activity reflecting an interest in chess; avoid traffic and crowds.<br>证据：Ada enjoys chess and wants to avoid traffic noise and crowds. | Attend the busy Red Square parade tonight.<br>abstained=false；citations=["s"] | 错误；无据 | 建议拥挤活动违背偏好，并无条件给出未被记忆支持的今晚活动。 | 建议与事实断言的边界需确认；当前将具体地点/时间活动视为存在性暗示。 |

## 需要你作出的主要判断

1. 是否接受“主要答案正确但新增无依据细节，则整答错误”（invented_explanation）？
2. 是否保持准确性与证据支持分开：lucky_guess为准确/无据，partial_answer为不完整/有据？
3. bike是否在本例固定解释为bicycle；两段break是否按不重叠时长相加？
4. 偏好题是否允许基于兴趣的泛化建议；quiet indoor是否足以满足避开人群；具体地点/今晚活动是否属于事实存在性断言？

本轮不据此自动改标签。若你决定修改，应形成明确的修订记录并重封计划，仍不得依据新14题测试结果修改。

## 覆盖边界

16例主要校准基础语义、支持性和结构化拒答；不是16个独立真实用户问题。尚无完整的知识更新、跨会话时间排序、冲突处理、提示注入等模型Judge校准案例；代码单元测试不能替代这些语义校准。正式Judge仍待人工复核和授权后的16次一致性检查（至少15/16双标签一致）。

人工结论请按Case ID逐项记录“认可/需要修改及理由”。当前源JSON的human_review_status继续为pending。
