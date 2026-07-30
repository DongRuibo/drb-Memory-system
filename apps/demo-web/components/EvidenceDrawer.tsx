import { formatEventTime } from "../app/memory-formatters";
import type { PromptMemoryContext, Turn } from "../app/types";

const sectionLabels: Record<PromptMemoryContext["section_kind"], string> = {
  user_preference: "用户偏好",
  current_fact: "当前事实",
  relevant_experience: "经历摘要",
};

const allowedEffectLabels: Record<PromptMemoryContext["allowed_effect"], string> = {
  response_style_only: "仅调整回答方式",
  factual_personalization_only: "仅用于事实个性化",
  contextual_example_only: "仅作为相关经验背景",
};

export function EvidenceDrawer({ turn }: { turn: Turn }) {
  const snapshot = turn.prompt_context;
  const used = snapshot?.memory_context ?? [];

  return (
    <div className="evidence-drawer">
      <div className="evidence-summary">
        <span>模型实际收到的上下文快照</span>
        <strong>
          {!snapshot
            ? "本轮没有可审计的 Context Pack"
            : used.length === 0
              ? "没有使用长期记忆"
              : `${used.length} 条已授权记忆`}
        </strong>
      </div>
      {snapshot && (
        <section className="evidence-runtime">
          <span>服务器运行时上下文</span>
          <strong>{formatRuntimeTime(snapshot.runtime_context)}</strong>
          <p>
            本地日期 {snapshot.runtime_context.local_date} · 时区{` `}
            {snapshot.runtime_context.timezone}
          </p>
        </section>
      )}
      {!snapshot && (
        <p className="evidence-unavailable">
          Context Pack 编译失败或该轮次来自旧版本，因此页面不会用当前记忆反推当时的模型输入。
        </p>
      )}
      {used.map((memory) => (
        <article className="evidence-memory" key={memory.memory_version_id}>
          <div>
            <span className="memory-type">
              {sectionLabels[memory.section_kind]}
            </span>
            <small>{memory.trust}</small>
          </div>
          <strong>{memory.memory_data ?? "快照内容已按清除请求销毁"}</strong>
          {memory.event_time && <time>{formatEventTime(memory.event_time)}</time>}
          <p>
            用于：{allowedEffectLabels[memory.allowed_effect]} · 来源{" "}
            {memory.source_refs.length} 个 · 版本{` `}
            {shortId(memory.memory_version_id)}
          </p>
        </article>
      ))}
      <div className="evidence-meta">
        <div>
          <span>Context Pack</span>
          <code>{turn.context_pack_id ? shortId(turn.context_pack_id) : "fallback"}</code>
        </div>
        <div>
          <span>Outcome Receipt</span>
          <code>
            {turn.outcome_receipt_id ? shortId(turn.outcome_receipt_id) : "pending"}
          </code>
        </div>
        <div>
          <span>模型</span>
          <code>{turn.model_id ?? "k3"}</code>
        </div>
      </div>
    </div>
  );
}

function formatRuntimeTime(runtime: {
  current_time_utc: string;
  timezone: string;
}) {
  try {
    return new Intl.DateTimeFormat("zh-CN", {
      dateStyle: "long",
      timeStyle: "medium",
      timeZone: runtime.timezone,
      hour12: false,
    }).format(new Date(runtime.current_time_utc));
  } catch {
    return runtime.current_time_utc;
  }
}

function shortId(value: string) {
  return `${value.slice(0, 8)}…${value.slice(-4)}`;
}
