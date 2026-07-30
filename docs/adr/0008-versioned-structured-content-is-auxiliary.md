---
status: accepted
---

# Keep versioned structured content auxiliary to canonical readable memory

LazyTT 为四个 Canonical Memory Type 增加可选、版本化的 Structured Content，但可读
`MemoryVersion.content` 仍是权威内容。每个类型只接受自己的 v1 Schema：
`lazytt.preference`、`lazytt.profile_fact`、`lazytt.semantic_fact` 和
`lazytt.episodic_summary`；未知版本、未知字段、类型错配、超限或不安全字段不得进入
Ledger。模型生成的结构无效时只丢弃结构，不阻断已经满足原有 Policy 的 canonical
正文。

## Considered Options

- 保存任意 JSON metadata：接入最快，但会让模型自行扩展字段、藏入指令或形成无法演进的
  隐式协议。
- 只保存强类型列：数据库约束最强，但四类内容差异大，每次 Schema 演进都需要扩宽主表，
  也难以保留稳定的版本边界。
- 保存版本化、白名单 JSON envelope：应用层执行完整类型校验，数据库校验 envelope、
  类型映射和大小，兼顾演进、审计与 fail-closed。

## Consequences

- Structured Content 绑定 Candidate，并在 Accept 时复制到不可变 Memory Version；Record
  和 Provider 不拥有独立结构化真相。
- v1 payload 总大小不超过 4096 bytes、无未知字段，所有字符串受长度和安全流水线约束。
- Correction 不复制可能与新正文不一致的旧 Structured Content；后续重新提取必须形成受
  治理的新版本契约。
- Forget 通过 Head 阻断读取；Erase 必须清空 Candidate/Version 的正文、Event Time 和
  Structured Content。
- 图、时间、过滤和 Prompt 表示只能从当前获授权 Version 重建，不能直接信任模型输出。
