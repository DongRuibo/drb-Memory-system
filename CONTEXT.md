# LazyTT Memory Context

本上下文描述 Agent 长期记忆治理所使用的统一语言。这里的记忆是可追溯的长期信息，不是运行时状态、权限来源或检索索引。

## Evidence and proposals

**Source Event**:
一次可能产生记忆的、具有来源标识和发生时间的观察，例如用户消息、已验证工具结果或任务结果。它是证据输入，不是 Memory。
_Avoid_: Raw Memory, Memory Record

**Memory Candidate**:
从 Source Event 提出的待决声明，尚未获得正式记忆的地位，也不能进入运行时 Context。
_Avoid_: Memory, Stored Memory

**Decision**:
确定性 Policy 或获授权审核者对 Memory Candidate 作出的 Accept、Reject 或 Needs Review 结论。
_Avoid_: Model Decision

## Canonical memory

**Memory Type**:
Memory 内容语义、治理策略和允许用途的稳定分类，与 Memory Scope 相互独立。正式记忆只使用 Preference、Profile Fact、Semantic Fact 和 Episodic Summary 四个治理类型；它们不是四个同级的认知类别。
_Avoid_: Retrieval Channel, Scope, Semantic Search Type

**Memory Family**:
Memory Type 的认知语义上卷维度。Preference、Profile Fact、Semantic Fact 属于 Semantic Family，Episodic Summary 属于 Episodic Family。Working Memory 属于 Runtime State，Procedure/Skill 属于独立的受治理能力系统，因此都不扩充正式 Memory Type 枚举。
_Avoid_: Memory Scope, Storage Tier, Procedure Authority

**Preference**:
主体明确表达的、可长期复用的回答或工作方式偏好；它只能影响表达和非权限型协作方式。
_Avoid_: Policy, Permission, Project Requirement

**Profile Fact**:
关于用户身份、背景或稳定特征的来源支持事实；它不构成身份认证、成员关系或授权依据。
_Avoid_: User Permission, Account State

**Semantic Fact**:
关于项目、领域或环境的来源支持稳定命题；这里的 Semantic 表示事实性知识，不表示向量相似度，也不替代 Canonical State。
_Avoid_: Project Fact, Semantic Search Result, Canonical State

**Episodic Summary**:
从明确发生的历史经历中提炼出的简洁、来源支持且可复用的经验摘要，而不是原始对话、运行日志或当前 Task 状态。
_Avoid_: Chat History, Runtime State, Unverified Lesson

**Memory Record**:
同一个长期记忆命题的逻辑身份；它关联多个不可变 Memory Version，但本身不代表当前内容。
_Avoid_: Vector, Provider Memory

**Memory Version**:
被接受的、不可变的记忆内容及其来源、有效期、分类和创建依据。
_Avoid_: Mutable Memory Row

**Memory Head**:
Memory Record 当前可见状态和当前版本的权威读取投影。它可由 Memory Ledger 重建。
_Avoid_: Latest Provider Result

**Memory Ledger**:
由不可变 Memory Version 与追加式 Memory Event 构成的权威历史，记录接受、修正、争议、过期、遗忘和擦除等事实。
_Avoid_: Chat History, Vector Store

**Memory Event**:
追加到 Memory Ledger 的领域事实，用于说明 Memory Record 为什么以及何时发生状态变化。
_Avoid_: Source Event, Outbox Event

## Access and use

**Memory Scope**:
一条 Memory Record 所属的单一、带类型的访问边界，由 Tenant 和一个 Scope Kind/Scope ID 确定。
_Avoid_: Arbitrary Metadata Filter

**Authority Decision**:
根据当前身份、成员关系、Task Purpose、Consent、Policy 和数据分类得出的允许或拒绝结论。它不能从 Memory 推断或授予。
_Avoid_: Retrieval Filter

**Memory Context Pack**:
为一次具体 Attempt 编译的、不可变且有过期时间的记忆片段集合；它携带来源与版本，但不携带权限或安全规则。
_Avoid_: Prompt, Canonical State, Working Memory

**Usage Receipt**:
记录某个 Memory Version 是否进入某个 Memory Context Pack、进入原因以及排序信息的审计事实。
_Avoid_: Retrieval Log

## Projections and lifecycle

**Retrieval Projection**:
由已获授权的当前 Memory Version 建立、可删除和可重建的 FTS、向量或 Provider 索引。
_Avoid_: Source of Truth, Memory Ledger

**Projection Receipt**:
Provider 对某次幂等索引操作的结果记录，用于判断投影是否 Ready、Failed、Stale 或 Deleted。
_Avoid_: Provider Memory Record

**Tombstone**:
阻止被遗忘或已擦除的 Memory Record 因重试、回放或重建而重新进入投影的持久标记。
_Avoid_: Soft Delete Flag

**Erasure Request**:
要求个人数据从权威内容、来源摘录、索引、缓存和可控副本中删除的可跟踪工作项。
_Avoid_: Delete API Call

## Explicit boundaries

**Canonical State**:
Agent 产品中决定真实业务事实、权限或执行约束的权威状态；Memory 可以引用它，但不能替代它。
_Avoid_: Memory

**Runtime State**:
当前 Task 或 Attempt 的执行进度和临时数据，不跨任务自动晋升为长期记忆。
_Avoid_: Episodic Memory

**Procedural Candidate**:
从历史经验提出的潜在流程建议，永远不直接获得执行权、工具权限或自动发布资格。
_Avoid_: Procedure, Skill
