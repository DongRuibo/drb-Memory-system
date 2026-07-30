---
status: accepted
---

# Unify canonical memory into four types independent of scope

两个项目使用相近但不等价的四类记忆。目标模型将正式 Memory Type 统一为 `preference`、`profile_fact`、`semantic_fact` 和 `episodic_summary`，并让 Tenant/User/Project/Agent Scope 独立表达访问边界；现有 `project_fact` 规范化为保留原 typed Scope 的 `semantic_fact`，不能从旧类型名推导 Scope。四个 Type 是面向 API、Policy、Retention 和 Prompt Effect 的治理分类，并向上归入 Semantic、Episodic 两个认知 Family；它们不是四个同级的认知类别。这保留 Profile 和 Preference 的差异化隐私与用途策略，也避免用项目归属替代事实语义，或把 Semantic Fact 与语义向量检索混为一谈。

## Considered Options

- 保留 `project_fact`：迁移最少，但类型继续混入 Scope 维度，无法自然表达 Tenant、User 或 Agent Scope 的一般事实。
- 只使用 `semantic_fact`、`episodic_summary` 和 `procedural`：分类更接近认知模型，但会失去 Preference 与 Profile 的独立权限、排序和 Prompt Effect 控制。
- 采用四类统一模型：增加一次受控 API/数据迁移，换取两个项目共享稳定协议和后续扩展空间。

## Consequences

- `memory_type` 表达内容语义，`scope` 表达访问边界；两者不得互相推导。
- `preference`、`profile_fact`、`semantic_fact` 向上归入 Semantic Family，
  `episodic_summary` 归入 Episodic Family；Family 当前由 Type 确定性派生，不新增数据库
  真相源或 HTTP 请求维度。
- `project_fact` 只作为要求显式 Project Scope 的迁移期请求别名存在；响应和新的权威记录
  始终使用 `semantic_fact`。历史 Tenant Scope 调用方必须改用 Canonical Type，不能让旧
  别名扩大查询到所有 Tenant Scope Semantic Fact。
- `procedural_candidate` 仍是待审核的流程建议，不属于四类正式记忆，也不能自动成为 Tool 或 Skill。
- Working Memory 继续属于 Runtime State；Graph、Vector、FTS/BM25 和 Temporal 继续属于
  可重建 Retrieval Projection 或查询维度。
- `0022_unified_memory_taxonomy.sql` 在无 identity 碰撞的前提下完成重分类，并用不含正文的
  `memory_type_reclassifications` 保存原类型、目标类型和 reason code。
