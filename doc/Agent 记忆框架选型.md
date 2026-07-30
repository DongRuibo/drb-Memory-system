# Agent 记忆框架选型：为什么当前阶段选择 Mem0

> **状态：框架调研输入。** Mem0 选择尚需通过 POC gate；当前决策记录见 [`ADR-0001`](../docs/adr/0001-rust-memory-service-with-mem0.md)，开发规范见 [`docs/README.md`](../docs/README.md)。

## 1. 最终结论

结合当前项目情况：

- 主系统使用 Rust；
- 已计划自研 Agent Runtime；
- 不使用 LangGraph；
- 团队规模较小；
- 需要优先完成 POC 和 MVP；
- 记忆模块需要独立部署、可替换、可治理；

第一阶段建议选择：

> **Mem0 OSS 自部署作为主要记忆能力提供者。**

同时保留：

- **Hindsight**：作为效果对照和后续升级候选；
- **Letta**：作为未来是否整体采用 Agent Runtime 的架构级候选，而不是当前的独立记忆框架。

这个决定并不是因为 Mem0 在所有能力上最强，而是因为：

> **Mem0 能以最低的接入和维护成本，满足当前 MVP 的核心需求，同时不会破坏现有 Rust Agent 架构。**

---

## 2. 三个框架的定位不同

在比较三个框架之前，必须先明确它们不是完全相同的产品。

### Mem0

Mem0 更像一个独立的记忆中间件。

它主要负责：

- 从对话中提取记忆；
- 保存用户偏好和事实；
- 基础去重与更新；
- 语义检索；
- 记忆增删改查；
- 为现有 Agent 增加跨会话记忆。

可以简单理解为：

> **给已经存在的 Agent 加一个独立记忆服务。**

---

### Hindsight

Hindsight 更像一个完整的长期记忆引擎。

除了基础保存与检索，它更强调：

- 时间检索；
- 关键词、语义、图关系等混合检索；
- 多次经历的自动整合；
- Observation；
- Reflection；
- 来源与证据追踪；
- 从历史经历中形成更高层认识。

可以简单理解为：

> **不仅让 Agent 记住事情，还让它从过去的经历中形成总结和认识。**

---

### Letta

Letta 更像一套完整的有状态 Agent Runtime。

它不仅管理记忆，还管理：

- Agent 身份；
- Agent 状态；
- 消息和对话；
- Memory Blocks；
- Archival Memory；
- 工具调用；
- Context；
- Agent Loop；
- 长期 Agent 生命周期。

可以简单理解为：

> **它不是单独给 Agent 增加记忆，而是直接提供一个拥有记忆的长期 Agent。**

---

## 3. 核心对比

| 维度 | Mem0 OSS | Hindsight OSS | Letta |
|---|---|---|---|
| 产品定位 | 独立记忆中间件 | 完整长期记忆引擎 | 有状态 Agent Runtime |
| 是否适合接入现有 Rust Runtime | 很适合 | 适合 | 容易产生重叠 |
| REST 接入 | 简单 | 简单 | 可以，但对象模型更复杂 |
| 自动记忆提取 | 强 | 强 | 主要依赖 Agent Loop |
| 用户偏好和稳定事实 | 很适合 | 适合 | 很适合 |
| 显式增删改查 | 清晰 | 较清晰 | 可以，但围绕 Agent/Block/Archive |
| 时间变化处理 | OSS 较弱 | 强 | 不是核心优势 |
| 混合检索 | 有 | 强 | 相对基础 |
| 自动经验整合 | 基础 | 强 | 依赖 Agent 自己整理 |
| Reflection | 不突出 | 原生支持 | 依赖 Agent Loop 或后台 Agent |
| 来源与证据追踪 | 需要补充 | 强 | Block 重写后证据链较弱 |
| 与自研 Context Assembler 重叠 | 低 | 低 | 高 |
| 部署和理解成本 | 低到中 | 中到高 | 高 |
| 两人团队快速落地 | 最适合 | 偏重 | 不适合只拿记忆 |
| 未来替换成本 | 低 | 低到中 | 高 |
| 当前阶段推荐度 | 最高 | 第二 | 较低 |

---

## 4. 为什么选择 Mem0

## 4.1 它正好位于我们需要的系统层级

当前项目已经计划自己实现：

- Rust Agent Runtime；
- Task 和 Attempt；
- Tool Registry；
- Tool Executor；
- Policy；
- 权限；
- Context Assembler；
- 状态管理；
- 审计与可观测。

因此现在真正缺少的是一个独立的记忆服务，负责：

- 记忆提取；
- 基础去重；
- 向量化；
- 搜索；
- 更新；
- 删除。

Mem0 的系统边界正好符合这个需求。

推荐结构：

```text
Rust Agent Runtime
        ↓
Rust Memory Service
        ↓
Mem0 Adapter
        ↓
Mem0 OSS
        ↓
PostgreSQL + pgvector
```

Mem0 不会要求我们采用它的 Agent Loop，也不会接管 Task、Attempt、Tool 和 Context。

---

## 4.2 它已经覆盖 MVP 的核心需求

第一版真正需要解决的是：

- 用户偏好；
- 稳定用户画像；
- 项目事实；
- 跨会话记忆；
- 基础去重；
- 语义召回；
- 记忆更新；
- 记忆删除；
- 用户和项目隔离；
- Context 注入。

这些能力 Mem0 OSS 已经基本覆盖。

例如：

```text
用户：以后技术问题先讲原理，再讲代码。
```

Mem0 可以负责：

```text
对话
→ 提取偏好
→ 保存
→ 后续查询时召回
```

我们的 Rust 层再负责：

```text
是否允许写入
属于哪个用户和项目
当前是否有效
是否可以进入 Context
```

---

## 4.3 接入成本最低

Mem0 的核心使用方式很容易理解：

```text
add
search
update
delete
```

Rust 只需要通过 REST API 调用，然后封装一个 Adapter。

相比之下：

Hindsight 需要理解：

- Retain；
- Recall；
- Reflect；
- World Fact；
- Experience Fact；
- Observation；
- Mental Model；
- Consolidation；
- 时间、图、语义和关键词融合。

Letta 需要理解：

- Agent；
- Memory Block；
- Archive；
- Passage；
- Conversation；
- Context；
- Agent Loop；
- MemFS；
- Dreaming。

对小团队来说，简单并不只是“学习成本低”，也意味着：

- 更容易定位问题；
- 更容易写测试；
- 更容易控制调用成本；
- 更容易上线；
- 更容易替换。

---

## 4.4 与 Rust 架构的侵入最小

Mem0 可以被封装成一个普通的 Memory Provider：

```rust
#[async_trait]
pub trait MemoryProvider {
    async fn remember(
        &self,
        request: RememberRequest,
    ) -> Result<RememberResult, MemoryError>;

    async fn recall(
        &self,
        request: RecallRequest,
    ) -> Result<Vec<MemoryHit>, MemoryError>;

    async fn correct(
        &self,
        request: CorrectMemoryRequest,
    ) -> Result<(), MemoryError>;

    async fn forget(
        &self,
        request: ForgetRequest,
    ) -> Result<(), MemoryError>;
}
```

业务层只认识自己的领域模型：

```text
MemoryCandidate
MemoryRecord
MemoryScope
MemoryHit
ContextPack
```

业务层不直接认识：

```text
Mem0 Memory ID
Hindsight Bank
Letta Block
Letta Archive
```

这样未来更换框架时，不需要重写 Agent Runtime。

---

## 4.5 适合采用渐进式建设

第一阶段可以先让 Mem0 解决：

- 提取；
- 基础去重；
- Embedding；
- 搜索；
- 可选 Reranker。

后续再由 Rust Memory Service 补充：

- 版本；
- 来源；
- Scope；
- 有效期；
- 冲突；
- 使用记录；
- Context Assembler；
- 权限；
- 审计。

这比一开始引入一个能力非常完整但复杂的系统，更适合当前团队。

---

## 5. 为什么不选择 Letta

## 5.1 Letta 解决的问题太大

我们现在只缺一个记忆模块，但 Letta 提供的是：

```text
记忆
+ Agent 状态
+ 对话
+ 工具
+ Context
+ Agent Loop
+ 长期 Agent Runtime
```

可以用一个类比解释：

> Mem0 像给现有电脑增加一块硬盘。  
> Letta 像为了使用硬盘，换了一整台电脑和操作系统。

Letta 不是不好，而是它的产品边界超过了当前需求。

---

## 5.2 与自研 Runtime 重复

我们准备自己管理：

| 自研 Rust 系统 | Letta 也会管理 |
|---|---|
| Agent 状态 | Stateful Agent |
| Context | Agent Context |
| Working Memory | Memory Blocks / MemFS |
| 工具执行 | Agent Tools |
| 对话 | Conversations |
| Agent Loop | Letta Runtime |
| 长期状态 | Persistent Agent |

如果同时使用两套系统，就会出现：

```text
谁决定 Context？
谁维护用户画像？
谁触发历史检索？
谁控制工具调用？
谁管理 Agent 状态？
发生冲突时听谁的？
```

这会增加架构复杂度。

---

## 5.3 单独拿记忆模块无法发挥 Letta 的核心优势

Letta 最强的地方不是 Block 或 Archive 本身，而是：

```text
Agent 观察当前对话
→ 判断什么值得记
→ 选择写入哪个位置
→ 主动修改 Memory Block
→ 需要时搜索 Archive
→ 将记忆用于后续行动
```

这套智能依赖 Letta Agent Loop。

可以用一个办公室类比：

```text
Memory Block = 桌面便签
Archive = 档案室
Agent Loop = 主动整理和查资料的秘书
```

如果只使用 Letta 的记忆 API，相当于：

> 保留了便签和档案室，却没有使用负责整理资料的秘书。

那么我们仍需自己实现：

- 什么时候记；
- 记到哪里；
- 如何去重；
- 如何更新用户画像；
- 什么时候搜索；
- 如何进入 Context。

此时 Letta 的投入产出比不如 Mem0。

---

## 5.4 架构绑定更深

采用 Letta 后，业务会逐渐依赖：

- Letta Agent；
- Block；
- Archive；
- Conversation；
- MemFS；
- Letta Context；
- Letta Tool Loop。

以后迁移时，可能不仅要迁移记忆，还要迁移：

- Agent 状态；
- 消息历史；
- Context 结构；
- 工具定义；
- 执行方式。

Mem0 只作为独立 Memory Provider，退出成本更低。

---

## 5.5 什么情况下应该选择 Letta

Letta 更适合以下情况：

- 不准备自研 Agent Runtime；
- 每个用户拥有一个长期 Agent；
- 希望 Agent 自己维护记忆；
- 希望记忆、工具、消息和 Context 统一管理；
- 接受 Letta 的 Agent 抽象；
- 产品形态是个人助手、AI 员工或长期人格化 Agent。

因此我们的结论不是 Letta 不好，而是：

> **Letta 更适合整体采用，不适合在已经自研 Runtime 的情况下只拿它的记忆模块。**

---

## 6. 为什么不选择 Hindsight

## 6.1 Hindsight 的纯记忆能力更完整

Hindsight 的优势必须如实承认。

它更擅长：

- 时间检索；
- 关键词、语义、图和时间混合检索；
- Rerank；
- 多次经历整合；
- Observation；
- Reflection；
- 证据追踪；
- 从历史中总结规律。

例如：

```text
项目 4 月使用 MySQL。
项目 7 月迁移到 PostgreSQL。
```

用户问：

```text
项目现在使用什么数据库？
项目 5 月使用什么数据库？
```

Hindsight 在这种时间变化场景中可能比 Mem0 OSS 更合适。

又例如：

```text
连续三次部署失败都与 Node 版本不一致有关。
```

Hindsight 更有机会形成：

```text
该项目的部署问题经常由 Node 版本不一致导致。
```

---

## 6.2 当前阶段存在能力过剩

当前 MVP 的核心目标是：

```text
用户偏好
稳定事实
跨会话记忆
基础检索
Context 注入
```

Hindsight 同时带来：

```text
时间检索
图遍历
Observation
Mental Model
Reflect
自动整合
证据链
后台维护
```

这些能力有价值，但当前并不全部是必需项。

选择框架的原则不应该是：

> 谁功能最多就选谁。

而应该是：

> 谁能以最低复杂度满足当前核心需求，就先选谁。

---

## 6.3 部署和调试复杂度更高

Hindsight 内部链路更长：

```text
原始事件
→ 事实抽取
→ 时间解析
→ 实体解析
→ 图关系
→ Observation 整合
→ 多路检索
→ 融合
→ Rerank
→ Reflect
```

功能越多，意味着：

- 更多后台流程；
- 更多模型调用；
- 更高的 Token 成本；
- 更多参数；
- 更复杂的排障；
- 更多需要评测的行为。

这并不代表 Hindsight 不成熟，而是对两人团队的第一版来说偏重。

---

## 6.4 与自研治理层存在边界问题

我们计划自己掌握：

- Memory Candidate；
- 正式 Memory Record；
- 来源；
- 版本；
- 冲突；
- 状态；
- 有效期；
- Consolidation；
- Reflection；
- Context Assembler。

而 Hindsight 自己也会生成：

- Observation；
- Mental Model；
- Reflect 结果；
- 当前事实判断。

这会产生一些边界问题：

```text
Observation 是正式记忆还是候选记忆？
Reflect 结果能否直接进入 Context？
Hindsight 判断的事实与 Rust Ledger 冲突时听谁的？
自动整合是否允许修改正式状态？
```

这些问题可以解决，但会增加设计成本。

---

## 6.5 什么情况下应该改选 Hindsight

以下能力被证明是产品核心时，应重新评估 Hindsight：

- “以前、现在、某个时间段”的问题很多；
- 用户和业务状态频繁变化；
- 需要关系和图检索；
- 需要从多次经历自动总结规律；
- 需要回答“为什么系统这么认为”；
- Reflection 能明显提升任务成功率；
- 程序记忆是产品核心卖点。

因此 Hindsight 不是被淘汰，而是：

> **作为第二阶段升级候选和当前 PoC 的对照方案。**

---

## 7. Mem0 的不足和对应方案

选择 Mem0 不代表忽略它的缺点。

| Mem0 OSS 的不足 | 当前处理方案 |
|---|---|
| 时间推理较弱 | 自有数据增加 `event_time`、`valid_from`、`valid_to` |
| 历史版本不够严格 | Rust Ledger 保存 `version`、`supersedes_id` |
| Memory Decay 不完整 | 后续在检索评分中增加新鲜度和使用频率 |
| 多租户治理不足 | Rust 层做 tenant/user/project 双重校验 |
| 图能力不是核心优势 | 第一版暂不引入图系统 |
| Reflection 不突出 | 后续根据任务效果单独增加 |
| Export 和审计需自建 | 权威数据保存在自有 PostgreSQL |
| 高可用需要自建 | MVP 单机，生产环境拆分应用和数据库 |
| 框架可能自动修改记忆 | Mem0 输出视为候选或 Provider 结果，由 Rust 再校验 |

关键原则：

> **Mem0 负责基础记忆智能，Rust 负责正式状态和治理。**

---

## 8. 推荐架构

```text
用户对话 / 工具轨迹 / 任务结果
                ↓
        Rust Memory Service
        ├── 写入策略
        ├── Scope 与权限
        ├── Memory Candidate
        ├── 版本与状态
        ├── 来源与审计
        └── Context Assembler
                ↓
           Mem0 Adapter
                ↓
            Mem0 OSS
        ├── Memory Extraction
        ├── 基础去重
        ├── Embedding
        ├── Search
        └── Optional Reranker
                ↓
       PostgreSQL + pgvector
```

其中建议区分两个逻辑区域：

```text
memory_core
├── 正式 Memory Record
├── Memory Candidate
├── 版本
├── 来源
├── Scope
├── 状态
└── Usage Receipt

mem0
├── Mem0 内部数据
├── 向量
└── 可重建检索索引
```

可以使用同一个 PostgreSQL 实例，但最好分 Schema 管理。

---

## 9. 项目推进建议

### 阶段一：POC

同时测试：

```text
Mem0 OSS
vs
Hindsight OSS
```

重点测试：

1. 稳定用户偏好；
2. 临时信息是否被错误保存；
3. 重复记忆；
4. 偏好变化；
5. 项目事实变化；
6. 精确关键词；
7. 时间查询；
8. 用户和项目隔离；
9. 单条修正和删除；
10. 中文效果。

记录指标：

- 写入准确率；
- 错误记忆率；
- 重复率；
- 时间问题正确率；
- Recall@K；
- 无关注入率；
- 写入 P95；
- 查询 P95；
- Token 成本；
- 部署与排障时间。

---

### 阶段二：MVP

采用：

```text
Rust Memory Service
+
Mem0 OSS
+
PostgreSQL + pgvector
```

先实现：

- remember；
- recall；
- correct；
- forget；
- get_context；
- inspect；
- 用户和项目隔离；
- 基础版本；
- Context Pack；
- 审计。

暂时不做：

- 复杂图关系；
- 完整 Temporal Reasoning；
- 自动 Reflection；
- 高级 Memory Decay；
- 程序记忆自动晋升。

---

### 阶段三：根据实际需求升级

如果发现时间和经验整合成为核心问题，可以：

```text
方案一：增加 Hindsight Adapter
方案二：自研 Temporal Layer
方案三：增加独立 Reflection Worker
方案四：引入 Graphiti 等时序图系统
```

不需要重写 Agent Runtime。

---

## 10. 会议汇报版本

可以在会议上直接这样讲：

> 我们第一阶段选择 Mem0 OSS 自部署。
>
> 这个选择不是因为 Mem0 在所有能力上最强，而是因为它最符合我们当前的架构边界和开发阶段。
>
> 我们已经决定自研 Rust Agent Runtime，包括任务状态、工具系统、权限和上下文组装。因此现在需要的是一个可以独立接入的记忆服务，而不是另一套 Agent Runtime。
>
> Mem0 主要提供记忆提取、去重、搜索、更新和删除，通过 REST 就可以接入 Rust。第一版需要的用户偏好、项目事实和跨会话记忆，它已经能够基本覆盖，而且接入和维护成本最低。
>
> 没有选择 Letta，是因为 Letta 本质上是一整套有状态 Agent Runtime。它不仅管理记忆，还管理 Agent、消息、工具和 Context。只拿它的记忆模块，无法发挥 Agent 自主管理记忆的优势；整体采用又会与我们的 Rust Runtime 重叠。
>
> 没有选择 Hindsight，不是因为它能力弱。Hindsight 在时间检索、关系检索、经验整合和 Reflection 方面更强，但这些能力会带来更复杂的处理流程和运维成本。对当前 MVP 来说存在一定能力过剩。
>
> 所以我们的原则不是选择功能最多的框架，而是选择能够以最低复杂度满足当前核心需求，并且未来容易替换和升级的框架。
>
> Mem0 负责基础记忆智能，Rust Memory Service 负责权限、版本、来源、状态和 Context，PostgreSQL 保存权威业务数据。未来如果时间推理和经验整合成为核心需求，再通过统一 Adapter 引入 Hindsight。

---

## 11. 一句话总结

> **选择 Mem0，是因为它“够用、简单、边界清楚、容易替换”；不选 Letta，是因为它与自研 Runtime 重叠；不选 Hindsight，是因为它当前能力偏重，复杂度超过 MVP 的必要需求。**
