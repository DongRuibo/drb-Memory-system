# 通用偏好、场景偏好与普通事实

## 分类与默认行为

保持 `preference / profile_fact / semantic_fact / episodic_summary` 四种顶层类型。
偏好额外使用已有 `structured_content` JSON 中的 `payload.applicability`，不新增顶层类型，
也不批量重写历史记录。

| 信息 | 类型和适用范围 | 检索行为 |
| --- | --- | --- |
| 回答时先给结论、使用中文、简洁表达 | preference + global | 通过权限与有效期检查后，独立于话题召回；不因低语义分数被丢弃 |
| 代码示例用 Rust、不吃香菜、技术回答先给结论 | preference + contextual | 当前任务相关时才使用，不能仅凭类型绕过相关性阈值 |
| 用户的姓名、工作地点、职业 | profile_fact | 作为用户资料检索，不自动成为每轮指令 |
| 项目数据库、接口地址、交付日期、系统错误码 | semantic_fact | 作为项目事实检索，必须写入明确授权的 project scope |
| 用户报告的某次已完成事件 | episodic_summary | 保留事件时间；不能把未知的历史事件时间替换成消息发送时间 |

旧偏好没有 `applicability` 时默认 contextual。global 不允许同时带 `qualifier`，避免
“仅在技术问题中适用”却标成通用偏好的矛盾。这里的 global 仅表示跨话题适用，
不表示跨用户或跨租户共享，也不能授予工具、数据读取或执行权限。

## 显式写入契约

`POST /v1/memory/events` 现在要求 `memory_type`；不再把没有类型声明的普通文本默认为偏好。
缺少类型会被拒绝，需要调用方补充类型，或使用单独的自动抽取入口。
SDK `SourceEventCommand`、Demo Agent、Demo v0 和固定检索评测器已同步。
`semantic_key` 可显式提供；省略时以正文 hash 为稳定身份，不尝试猜测事实属于哪个属性。

通用偏好的结构化示例：

```json
{
  "memory_type": "preference",
  "semantic_key": "user.response_style.conclusion_first",
  "structured_content": {
    "schema_name": "lazytt.preference",
    "schema_version": 1,
    "payload": {
      "subject": "user",
      "value": "先给结论再讲细节",
      "applicability": "global"
    }
  }
}
```

场景偏好改为 `applicability: contextual`，可增加来源明确支持的 `qualifier`，
例如“代码示例”或“饮食”。普通事实则声明相应事实类型，不能为了提升召回率标成偏好。
上述片段需与原有 event_id、source_ref、content、occurred_at、scope_selector 等字段一起提交。
项目事实必须提供授权项目的 scope_selector，个人偏好和资料仍使用本人 user scope。

这是显式开发接口的类型声明，不是要求最终用户在聊天里填写类型。
自动抽取提示已区分事实与偏好，并要求只有无条件的表达方式偏好才能标成 global。
本轮没有调用云 API，提示效果只有解析器与模拟输出测试证据，尚无真实抽取质量成绩。

Repository 原有 `ingest_explicit(IngestCommand)` 保留为内部显式偏好便捷方法，未自动升级历史行为；
没有附加标记的此类记录也按 contextual 处理。外部显式入口使用带类型的 Candidate 写入。

## 检索与审计

新增的 `global_preference` 通道直接从权威版本读取本人通用偏好，不依赖向量模型碰巧召回，
也不受查询中“昨天／明天”等事件时间窗口限制。仍检查当前 Head、权限、来源、有效期、
保留期、tombstone，并在排序后再次回查权威版本。

通用偏好在有限候选和最终返回数量中优先保留，但不伪造或抬高语义分数；仍受请求的
memory_types、scope、数量和 Context Pack token 预算约束。显式要求只读事实时不会注入偏好。
通道会出现在 Recall Explain 回执中。Migration `0033` 只扩展回执 channel 约束，
不修改旧 migration checksum 或批量更新记忆正文。

仅修改 `applicability` 也可通过 Correction 生成新版本，沿用 revision CAS、Outbox、
旧 Context Pack 失效与来源审计。正文和结构化数据都没变化时仍拒绝无意义修正。
同一个摄入幂等键不能改写记忆类型、正文、范围或结构化数据。

排序器版本已递增，避免新策略复用旧排序身份；抽取器版本同步递增。
历史 Context Pack 保留为历史证据，本轮未对旧存量环境做迁移或全局缓存失效操作。

## 验证与评测边界

本地真实链路单独验证 global 跨话题生效、contextual 按相关性筛选、事实不变成偏好、
适用范围修正生成新版本、幂等重试与遗忘后的立即拒读。

固定集升级为 v2.2.0：7 条场景偏好、2 条用户资料、9 条项目事实、2 条原生事件记忆。
沿用 v2.1.1 的 20 条正文、19 个正例、5 个负例及全部阈值，只补充正确类型、偏好适用范围
和明确事件时间。评测现在同时检索授权 user/project scope，并记录负例返回详情与失败原因。
分类变化会影响排序器输入与适用规则，因此新指标不能当作旧版完全同条件复现。

固定集没有通用表达偏好，不用“忽略通用偏好命中”的方式放宽负例门槛；通用偏好另行测试。
若高分难负例仍命中，应继续记录，不通过特定查询黑名单或放宽阈值制造通过结果。


最新实现及验证结果见 [2026-10-20 回执](../poc/evidence/preference-policy-2026-10-20.md)。
固定集仍有 1 条高分难负例，额外诊断也暴露场景偏好低分漏召回，因此不能声称语义政策已
全面达到产品效果标准。这里完成了分类、存储与检索契约，不等于模型已稳定理解所有适用场景。

后续真实聊天发现并修复旧 canonicalization 将 global 改写为技术场景的错误；
最新真实抽取、适用性复核结果和候选精准性限制见
[DeepSeek 验收](../poc/evidence/deepseek-recovery-2026-10-20.md)。
