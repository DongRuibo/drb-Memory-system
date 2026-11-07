# Judge V4新增对照人工复核清单

14例均为自建、未写入V4 Prompt示例；由同一开发者编写，不是独立盲测。预期标签、事实原子为人工注释，仍待用户复核。**本轮无真实V4模型输出，不宣称语义稳定。**

A=semantic_accuracy；S=supported。事实原子只用于解释预期标签，不送入联合Judge输入。

| ID / 类别 | 问题 | 参考 / 证据 | 待评回答 | 预期 A/S | 已陈述事实原子 |
|---|---|---|---|---|---|
| c01 / incomplete_supported | Which two languages does Nora speak? | 参考：Japanese and Spanish<br>证据：Nora speaks Japanese and Spanish. | Japanese | false/true | Nora speaks Japanese. |
| c02 / incomplete_supported | Which two tools did Orion approve? | 参考：hammer and wrench<br>证据：Orion approved a hammer and a wrench. | A wrench. | false/true | Orion approved a wrench. |
| c03 / incomplete_unsupported | Which three checkpoints did Tess visit? | 参考：Harbor, Meadow, and Ridge<br>证据：Tess visited Harbor, Meadow, and Ridge. | Harbor and Desert. | false/false | Tess visited Harbor.<br>Tess visited Desert. |
| c04 / incomplete_unsupported | Which three programming languages does Milo use? | 参考：Rust, Julia, and Ruby<br>证据：Milo uses Rust, Julia, and Ruby. | Rust and Swift. | false/false | Milo uses Rust.<br>Milo uses Swift. |
| c05 / complete_supported | Which two materials did Kei select? | 参考：oak and cork<br>证据：Kei selected oak and cork. | Kei selected oak and cork. | true/true | Kei selected oak.<br>Kei selected cork. |
| c06 / complete_supported | Which two rooms did Imani reserve? | 参考：North and East<br>证据：Imani reserved the North room and the East room. | The East and North rooms. | true/true | Imani reserved the East room.<br>Imani reserved the North room. |
| c07 / complete_partly_false | Which two plants did Jules buy? | 参考：basil and mint<br>证据：Jules bought basil and mint. | Basil and thyme. | false/false | Jules bought basil.<br>Jules bought thyme. |
| c08 / complete_partly_false | What is the build label? | 参考：Nebula<br>证据：The build label is Nebula. The build was created on 2025-03-11. | Nebula, created on 2025-03-09. | false/false | The build label is Nebula.<br>The build was created on 2025-03-09. |
| c09 / entity_binding | Which instrument does Uma play? | 参考：cello<br>证据：Uma plays the cello. Vik plays the violin. | The violin. | false/false | Uma plays the violin. |
| c10 / entity_binding | Which workspace belongs to Kiran? | 参考：Cobalt<br>证据：The Cobalt workspace belongs to Kiran. The Amber workspace belongs to Leena. | Amber. | false/false | The Amber workspace belongs to Kiran. |
| c11 / temporal_update | What is Ivo's current office? | 参考：Birch<br>证据：On 2025-01-01 Ivo worked in the Aspen office. Effective 2025-06-01, Ivo moved to the Birch office and no longer worked in Aspen. | Aspen. | false/false | As of 2025-08-01, Ivo currently works in Aspen. |
| c12 / temporal_update | What is Ivo's current office, and where did he work in January? | 参考：Currently Birch; Aspen in January<br>证据：On 2025-01-01 Ivo worked in the Aspen office. Effective 2025-06-01, Ivo moved to the Birch office and no longer worked in Aspen. | Ivo worked in Aspen in January; he now works in Birch. | true/true | Ivo worked in Aspen in January 2025.<br>As of 2025-08-01, Ivo currently works in Birch. |
| c13 / abstention | Which team does Sora support? | 参考：unknown<br>证据：Sora reads fiction. | Insufficient evidence.<br>abstained=true | true/null | 无（拒答） |
| c14 / abstention | Which team does Talia support? | 参考：Falcons<br>证据：Talia supports the Falcons. | Insufficient evidence.<br>abstained=true | false/null | 无（拒答） |

当前只验证数据结构、信息流、输入上界及对给定标签的门禁；上述14个答案并未由模型评分。
c07中的“完整”指回答了两个槽位，但其中一个事实错误；不是指覆盖了全部正确参考事实。c11的短答案绑定当前时间，旧Aspen事实不能证明当前办公室；c12显式区分过去与当前，所以两个事实都可获支持。
拟在下一轮单独报告14例对照匹配，不把它们并入原16例的分母。建议新增14/14、关键错误0的补充闸门须用户明确确认。
