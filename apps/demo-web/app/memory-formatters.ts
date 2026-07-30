import type {
  EventTime,
  MemoryRecord,
  MemoryScopeKind,
  MemoryType,
  StructuredContent,
} from "./types";

export const memoryAllowedEffect: Record<MemoryType, string> = {
  preference: "调整回答方式",
  profile_fact: "按用户背景个性化",
  semantic_fact: "使用当前事实知识",
  episodic_summary: "参考相关历史经验",
};

export const scopeLabel: Record<MemoryScopeKind, string> = {
  user: "个人范围",
  project: "项目共享",
  tenant: "租户共享",
  agent: "Agent 范围",
};

export function memoryDisplayLabel(
  memoryType: MemoryType,
  scopeKind: MemoryScopeKind,
): string {
  if (memoryType === "semantic_fact" && scopeKind === "project") return "项目事实";
  if (memoryType === "preference") return "用户偏好";
  if (memoryType === "profile_fact") return "用户资料";
  if (memoryType === "semantic_fact") return "事实知识";
  return "经历摘要";
}

const precisionLabels: Record<EventTime["precision"], string> = {
  instant: "精确时刻",
  minute: "分钟",
  hour: "小时",
  part_of_day: "时段",
  day: "日期",
  week: "周",
  month: "月份",
  quarter: "季度",
  year: "年份",
};

function formatDateTime(value: string, timezone: string): string {
  try {
    return new Intl.DateTimeFormat("zh-CN", {
      dateStyle: "medium",
      timeStyle: "short",
      timeZone: timezone,
      hour12: false,
    }).format(new Date(value));
  } catch {
    return value;
  }
}

export function formatEventTime(eventTime: EventTime): string {
  const start = formatDateTime(eventTime.start, eventTime.timezone);
  const end = formatDateTime(eventTime.end, eventTime.timezone);
  return `${start} — ${end}（${precisionLabels[eventTime.precision]}，${eventTime.timezone}）`;
}

const structuredKeyLabels: Record<string, string> = {
  subject: "主体",
  value: "取值",
  qualifier: "限定",
  attribute: "属性",
  predicate: "关系",
  object: "对象",
  event_kind: "事件类型",
  outcome: "结果",
  participants: "参与者",
};

export function structuredEntries(
  structured: StructuredContent,
): Array<{ key: string; label: string; value: string }> {
  return Object.entries(structured.payload).map(([key, value]) => ({
    key,
    label: structuredKeyLabels[key] ?? key,
    value: typeof value === "string" ? value : JSON.stringify(value),
  }));
}

export function currentVersion(memory: MemoryRecord) {
  return memory.versions.find(
    (version) => version.memory_version_id === memory.current_version_id,
  );
}
