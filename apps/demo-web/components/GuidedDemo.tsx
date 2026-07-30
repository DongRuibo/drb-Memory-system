"use client";

export type GuidedAction =
  | "introduce"
  | "inspect"
  | "new_chat"
  | "time_recall"
  | "project_update"
  | "forget_experience";

export const guidedSteps: Array<{
  number: string;
  title: string;
  hint: string;
  action: GuidedAction;
  message?: string;
}> = [
  {
    number: "1",
    title: "自然形成记忆",
    hint: "从一句自然介绍提取四类记忆。",
    action: "introduce",
    message:
      "我是 Agent 研发工程师，主要使用 Rust。我们正在开发企业级 Agent 记忆系统，当前项目使用 PostgreSQL 作为权威数据库。2026年7月15日下午做 Outbox 时遇到过重复消费，后来用 event_id 去重解决了。我喜欢回答技术问题时先给一句结论，再列出要点。请根据这些情况，简要说说上线前最需要关注什么。",
  },
  {
    number: "2",
    title: "查看记忆结构",
    hint: "检查类型、范围、时间与结构化字段。",
    action: "inspect",
  },
  {
    number: "3",
    title: "跨对话回忆",
    hint: "新对话只带回相关且获授权的记忆。",
    action: "new_chat",
    message:
      "请根据你记得的我的工作角色、常用编程语言、LazyTT Memory 项目的权威数据库，以及 Outbox 经验，给我一份上线风险备忘；按我喜欢的回答格式组织。",
  },
  {
    number: "4",
    title: "按时间回忆",
    hint: "用事件发生时间检索历史经历。",
    action: "time_recall",
    message: "我在2026年7月处理过什么工程问题？请说明时间与解决办法。",
  },
  {
    number: "5",
    title: "事实更新",
    hint: "明确变化生成新版本，旧版本仍可审计。",
    action: "project_update",
    message: "项目的权威数据库现在改用 CockroachDB，不再使用 PostgreSQL。请按新情况更新建议。",
  },
  {
    number: "6",
    title: "忘记与清除",
    hint: "停用指定经历，保留其他可用记忆。",
    action: "forget_experience",
    message: "2026年7月15日那次 Outbox 重复消费的经历不用再保留了。现在再给我一份简短的上线提醒。",
  },
];

export function GuidedDemo({
  disabled,
  onRun,
}: {
  disabled: boolean;
  onRun: (action: GuidedAction) => void;
}) {
  return (
    <section className="guided-strip" aria-label="六步核心演示">
      <div className="guided-heading">
        <span>六步验证核心能力</span>
        <small>分类、范围、时间、结构、版本、遗忘都可检查</small>
      </div>
      <div className="guided-steps">
        {guidedSteps.map((step) => (
          <button
            disabled={disabled}
            key={step.number}
            onClick={() => onRun(step.action)}
            type="button"
          >
            <span className="step-number">{step.number}</span>
            <span>
              <strong>{step.title}</strong>
              <small>{step.hint}</small>
            </span>
            <span className="step-arrow">→</span>
          </button>
        ))}
      </div>
    </section>
  );
}
