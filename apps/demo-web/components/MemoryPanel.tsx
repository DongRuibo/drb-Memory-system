"use client";

import { useMemo, useState } from "react";

import {
  currentVersion,
  formatEventTime,
  memoryAllowedEffect,
  memoryDisplayLabel,
  scopeLabel,
  structuredEntries,
} from "../app/memory-formatters";
import type {
  MemoryRecord,
  MemoryScopeKind,
  MemoryType,
  StructuredContent,
} from "../app/types";

const allTypes: Array<{ value: "all" | MemoryType; label: string }> = [
  { value: "all", label: "全部类型" },
  { value: "preference", label: "用户偏好" },
  { value: "profile_fact", label: "用户资料" },
  { value: "semantic_fact", label: "事实知识" },
  { value: "episodic_summary", label: "经历摘要" },
];

const allScopes: Array<{ value: "all" | MemoryScopeKind; label: string }> = [
  { value: "all", label: "全部范围" },
  { value: "user", label: "个人" },
  { value: "project", label: "项目" },
  { value: "tenant", label: "租户" },
  { value: "agent", label: "Agent" },
];

export function MemoryPanel({
  className,
  memories,
  onClose,
  onErase,
  onForget,
}: {
  className: string;
  memories: MemoryRecord[];
  onClose: () => void;
  onErase: (id: string) => void;
  onForget: (id: string) => void;
}) {
  const [typeFilter, setTypeFilter] = useState<"all" | MemoryType>("all");
  const [scopeFilter, setScopeFilter] = useState<"all" | MemoryScopeKind>("all");
  const active = useMemo(
    () =>
      memories.filter(
        (memory) =>
          memory.head_state === "active" &&
          memory.current_version_id &&
          (typeFilter === "all" || memory.memory_type === typeFilter) &&
          (scopeFilter === "all" || memory.scope.kind === scopeFilter),
      ),
    [memories, scopeFilter, typeFilter],
  );
  const activeTotal = memories.filter(
    (memory) => memory.head_state === "active" && memory.current_version_id,
  ).length;
  const inactive = memories.filter((memory) => memory.head_state !== "active");

  return (
    <aside className={className}>
      <header>
        <div>
          <span className="memory-spark">✦</span>
          <div>
            <h2>记忆档案</h2>
            <p>当前版本与历史证据</p>
          </div>
        </div>
        <button onClick={onClose} type="button" aria-label="关闭">
          ×
        </button>
      </header>
      <div className="memory-explainer">
        <strong>{activeTotal}</strong>
        <span>条当前可用记忆</span>
        <p>类型说明“是什么”，范围说明“谁可见”。只有相关且仍获授权的版本才进入回答。</p>
      </div>
      <div className="memory-filters">
        <select
          aria-label="按记忆类型筛选"
          onChange={(event) => setTypeFilter(event.target.value as "all" | MemoryType)}
          value={typeFilter}
        >
          {allTypes.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
        <select
          aria-label="按记忆范围筛选"
          onChange={(event) =>
            setScopeFilter(event.target.value as "all" | MemoryScopeKind)
          }
          value={scopeFilter}
        >
          {allScopes.map((item) => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </div>
      <div className="memory-list">
        {active.length === 0 && (
          <div className="empty-memory">
            <span>◇</span>
            <strong>{activeTotal ? "当前筛选没有结果" : "这里还是空的"}</strong>
            <p>
              {activeTotal
                ? "调整上方的类型或范围筛选。"
                : "像平常聊天一样介绍自己、项目或经历，系统会自动判断是否值得保存。"}
            </p>
          </div>
        )}
        {active.map((memory) => {
          const current = currentVersion(memory);
          return (
            <details className="memory-card" key={memory.memory_record_id}>
              <summary>
                <div className="memory-card-labels">
                  <span className={`memory-type ${memory.memory_type}`}>
                    {memoryDisplayLabel(memory.memory_type, memory.scope.kind)}
                  </span>
                  <span className={`scope-badge ${memory.scope.kind}`}>
                    {scopeLabel[memory.scope.kind]}
                  </span>
                </div>
                <p>{current?.content ?? "内容不可用"}</p>
                <div className="memory-card-meta">
                  <span>当前 v{current?.version_no ?? memory.versions.length}</span>
                  <span>{current?.source_refs.length ?? 0} 个来源</span>
                </div>
              </summary>
              <div className="memory-detail">
                <div className="memory-policy-line">
                  <span>可影响</span>
                  <strong>{memoryAllowedEffect[memory.memory_type]}</strong>
                </div>
                {current?.event_time && (
                  <section className="event-time-view">
                    <span>事件时间</span>
                    <strong>{formatEventTime(current.event_time)}</strong>
                  </section>
                )}
                {current?.structured_content && (
                  <StructuredContentView structured={current.structured_content} />
                )}
                <section className="version-section">
                  <h3>版本时间线</h3>
                  <div className="version-line">
                    {[...memory.versions]
                      .sort((left, right) => right.version_no - left.version_no)
                      .map((version) => (
                        <div
                          className={
                            version.memory_version_id === memory.current_version_id
                              ? "current"
                              : ""
                          }
                          key={version.memory_version_id}
                        >
                          <span>v{version.version_no}</span>
                          <p>
                            {version.content_erased_at
                              ? "内容已物理清除"
                              : version.content ?? "内容不可用"}
                            <small>{formatAuditTime(version.created_at)}</small>
                          </p>
                        </div>
                      ))}
                  </div>
                </section>
                <div className="memory-actions">
                  <button onClick={() => onForget(memory.memory_record_id)} type="button">
                    忘记
                  </button>
                  <button
                    className="danger"
                    onClick={() => onErase(memory.memory_record_id)}
                    type="button"
                  >
                    物理清除
                  </button>
                </div>
              </div>
            </details>
          );
        })}
        {inactive.length > 0 && (
          <div className="inactive-count">
            另有 {inactive.length} 条争议中、已忘记或已清除记录
          </div>
        )}
      </div>
      <footer>
        <span>权威档案</span>
        <strong>PostgreSQL</strong>
        <span>搜索投影</span>
        <strong>Mem0</strong>
      </footer>
    </aside>
  );
}

function StructuredContentView({ structured }: { structured: StructuredContent }) {
  return (
    <section className="structured-view">
      <div className="structured-heading">
        <span>结构化内容</span>
        <small>
          {structured.schema_name} · v{structured.schema_version}
        </small>
      </div>
      <dl>
        {structuredEntries(structured).map((entry) => (
          <div key={entry.key}>
            <dt>{entry.label}</dt>
            <dd>{entry.value}</dd>
          </div>
        ))}
      </dl>
      <details>
        <summary>查看原始 JSON</summary>
        <pre>{JSON.stringify(structured.payload, null, 2)}</pre>
      </details>
    </section>
  );
}

function formatAuditTime(value: string): string {
  try {
    return new Intl.DateTimeFormat("zh-CN", {
      dateStyle: "short",
      timeStyle: "short",
      hour12: false,
    }).format(new Date(value));
  } catch {
    return value;
  }
}
