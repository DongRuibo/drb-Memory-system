"use client";

import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { MarkdownMessage } from "./markdown";

const API_BASE = process.env.NEXT_PUBLIC_DEMO_API_URL ?? "http://127.0.0.1:8090";

type Health = { ready: boolean; memory_service: string; chat_store: string; model_configuration: string; detail: string | null };
type SessionSummary = { id: string; title: string; mode: "guided" | "free"; guided_step: string; created_at: string; updated_at: string };
type Message = { id: string; session_id: string; turn_id: string | null; role: "user" | "assistant" | "system"; content: string; status: "pending" | "streaming" | "completed" | "failed" | "cancelled"; created_at: string };
type MemoryEffect = { kind: "remembered" | "source_merged" | "corrected" | "forgotten" | "needs_review"; memory_record_id?: string; memory_version_id?: string | null; content?: string; revision?: number; resolution?: "automatic_supersede" };
type Turn = { id: string; session_id: string; assistant_message_id: string; status: "accepted" | "processing" | "streaming" | "completed" | "failed" | "cancelled"; context_pack_id: string | null; outcome_receipt_id: string | null; used_memory_ids: string[]; used_memory_version_ids: string[]; memory_effects: MemoryEffect[]; model_id: string | null; model_request_id_hash: string | null; total_tokens: number | null; error_detail: string | null };
type SessionDetail = SessionSummary & { messages: Message[]; turns: Turn[] };
type MemoryVersion = { memory_version_id: string; version_no: number; content: string | null; source_refs: string[]; created_at: string; content_erased_at: string | null };
type MemoryRecord = { memory_record_id: string; memory_type: string; semantic_key: string; scope: { kind: string; id: string | null }; head_state: string; revision: number; current_version_id: string | null; versions: MemoryVersion[]; events: Array<{ sequence_no: number; event_type: string; reason_code: string; occurred_at: string }> };
type ApiError = { code?: string; detail?: string };

const memoryTypeLabel: Record<string, string> = {
  preference: "回答偏好",
  profile_fact: "个人信息",
  semantic_fact: "事实知识",
  episodic_summary: "相关经历",
};

const guidedSteps = [
  { number: "1", title: "自然认识你", hint: "一句话识别偏好、身份、项目与经历。", message: "我是 Agent 研发工程师，主要使用 Rust。我们正在开发企业级 Agent 记忆系统，当前项目使用 PostgreSQL 作为权威数据库。上次做 Outbox 时遇到过重复消费，后来用 event_id 去重解决了。我喜欢回答技术问题时先给一句结论，再列出要点。请根据这些情况，简要说说上线前最需要关注什么。", action: undefined },
  { number: "2", title: "跨对话使用", hint: "打开新对话，自动带回真正相关的记忆。", message: "请根据你记得的我的工作角色、常用编程语言、LazyTT Memory 项目的权威数据库，以及我处理 Outbox 重复消费的经验，给我一份上线风险备忘；按我喜欢的回答格式组织。", action: "new_chat" as const },
  { number: "3", title: "自动更新事实", hint: "发现明确变化，生成新版本并提示。", message: "项目的权威数据库现在改用 CockroachDB，不再使用 PostgreSQL。请按新情况更新建议。", action: "project_update" as const },
  { number: "4", title: "自然忘记", hint: "指定经历立即停用，其他记忆不受影响。", message: "上次 Outbox 重复消费的经历不用再保留了。现在再给我一份简短的上线提醒。", action: "forget_experience" as const },
];

const memoryAllowedEffect: Record<string, string> = {
  preference: "调整回答方式",
  profile_fact: "按你的背景个性化",
  semantic_fact: "使用当前事实知识",
  episodic_summary: "参考相关历史经验",
};

export default function Home() {
  const [health, setHealth] = useState<Health | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [session, setSession] = useState<SessionDetail | null>(null);
  const [memories, setMemories] = useState<MemoryRecord[]>([]);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [evidenceTurnId, setEvidenceTurnId] = useState<string | null>(null);
  const [mobileMemoryOpen, setMobileMemoryOpen] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);
  const messageScrollerRef = useRef<HTMLDivElement | null>(null);

  function scrollConversationToBottom(force = false) {
    requestAnimationFrame(() => {
      const scroller = messageScrollerRef.current;
      if (!scroller) return;
      const distanceFromBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
      if (force || distanceFromBottom < 180) {
        scroller.scrollTo({ top: scroller.scrollHeight, behavior: force ? "auto" : "smooth" });
      }
    });
  }

  const fetchJson = useCallback(async <T,>(path: string, init?: RequestInit): Promise<T> => {
    const response = await fetch(`${API_BASE}${path}`, init);
    const payload = (await response.json().catch(() => ({}))) as T & ApiError;
    if (!response.ok) throw new Error(payload.detail ?? `请求失败 (${response.status})`);
    return payload;
  }, []);

  const refreshMemories = useCallback(async () => {
    const payload = await fetchJson<{ memories: { records: MemoryRecord[] } }>("/api/memories");
    setMemories(payload.memories.records);
  }, [fetchJson]);
  const refreshSession = useCallback(async (sessionId: string) => {
    const payload = await fetchJson<{ session: SessionDetail }>(`/api/chat/sessions/${sessionId}`);
    setSession(payload.session);
    return payload.session;
  }, [fetchJson]);
  const refreshSessions = useCallback(async () => {
    const payload = await fetchJson<{ sessions: SessionSummary[] }>("/api/chat/sessions");
    setSessions(payload.sessions);
    return payload.sessions;
  }, [fetchJson]);
  const createSession = useCallback(async (mode: "guided" | "free" = "guided") => {
    const payload = await fetchJson<{ session: SessionSummary }>("/api/chat/sessions", {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ title: mode === "guided" ? "记忆演示" : "新的对话", mode }),
    });
    setSelectedSessionId(payload.session.id);
    await Promise.all([refreshSessions(), refreshSession(payload.session.id)]);
    return payload.session;
  }, [fetchJson, refreshSession, refreshSessions]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const currentHealth = await fetchJson<Health>("/api/health");
        if (cancelled) return;
        setHealth(currentHealth);
        const currentSessions = await refreshSessions();
        await refreshMemories();
        if (cancelled) return;
        if (currentSessions.length === 0) await createSession("guided");
        else {
          setSelectedSessionId(currentSessions[0].id);
          await refreshSession(currentSessions[0].id);
        }
      } catch (cause) {
        if (!cancelled) {
          const detail = cause instanceof Error ? cause.message : "演示服务不可用";
          setHealth({ ready: false, memory_service: "unavailable", chat_store: "unavailable", model_configuration: "check_required", detail });
          setError(detail);
        }
      }
    })();
    return () => { cancelled = true; eventSourceRef.current?.close(); };
  }, [createSession, fetchJson, refreshMemories, refreshSession, refreshSessions]);

  async function selectSession(sessionId: string) {
    setSelectedSessionId(sessionId);
    try { await refreshSession(sessionId); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "无法加载对话"); }
  }

  const activeMemories = useMemo(() => memories.filter((memory) => memory.head_state === "active" && memory.current_version_id), [memories]);
  const projectMemory = activeMemories.find((memory) => memory.memory_type === "semantic_fact" && memory.scope.kind === "project");
  const experienceMemory = activeMemories.find((memory) => memory.memory_type === "episodic_summary");
  const activeTurn = session?.turns.find((turn) => ["accepted", "processing", "streaming"].includes(turn.status));

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const scroller = messageScrollerRef.current;
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
  }, [session?.id]);

  function connectToTurn(turnId: string) {
    eventSourceRef.current?.close();
    let lastSequence = 0;
    const source = new EventSource(`${API_BASE}/api/chat/turns/${turnId}/events?after=0`);
    eventSourceRef.current = source;
    const eventTypes = ["turn.accepted", "turn.processing", "memory.processing", "memory.effects", "memory.warning", "context.ready", "answer.delta", "evidence.warning", "turn.cancel_requested", "turn.completed", "turn.failed", "turn.cancelled"];
    const onEvent = (raw: Event) => {
      const parsed = JSON.parse((raw as MessageEvent<string>).data) as { sequence_no: number; type: string; payload: Record<string, unknown> };
      if (parsed.sequence_no <= lastSequence) return;
      lastSequence = parsed.sequence_no;
      if (parsed.type === "answer.delta") {
        const delta = String(parsed.payload.delta ?? "");
        setSession((current) => current ? { ...current, messages: current.messages.map((item) => item.turn_id === turnId && item.role === "assistant" ? { ...item, content: item.content + delta, status: "streaming" } : item) } : current);
        scrollConversationToBottom();
      }
      if (parsed.type === "memory.effects") {
        const effects = (parsed.payload.effects ?? []) as MemoryEffect[];
        const remembered = effects.filter((effect) => effect.kind === "remembered").length;
        const merged = effects.filter((effect) => effect.kind === "source_merged").length;
        const corrected = effects.filter((effect) => effect.kind === "corrected").length;
        const correctedEffect = effects.find((effect) => effect.kind === "corrected");
        const forgotten = effects.filter((effect) => effect.kind === "forgotten").length;
        if (remembered) setNotice(`已自动记住 ${remembered} 条信息，可随时撤销`);
        else if (merged) setNotice(`没有新增重复记忆，已为 ${merged} 条记忆补充来源`);
        else if (corrected) setNotice(correctedEffect?.content ? `已自动更新记忆：${correctedEffect.content}` : "已自动创建新版本，并在本轮生效");
        else if (forgotten) setNotice("已立即停止使用这条记忆");
      }
      if (parsed.type === "memory.warning" || parsed.type === "evidence.warning") setNotice(String(parsed.payload.detail ?? "回答已继续，但部分记忆证据暂不可用"));
      if (["turn.completed", "turn.failed", "turn.cancelled"].includes(parsed.type)) {
        source.close(); eventSourceRef.current = null; setBusy(false);
        Promise.all([selectedSessionId ? refreshSession(selectedSessionId) : Promise.resolve(), refreshSessions(), refreshMemories()]).catch(() => undefined);
      }
    };
    eventTypes.forEach((type) => source.addEventListener(type, onEvent));
    source.onerror = () => { if (eventSourceRef.current === source) setNotice("连接正在自动恢复，已生成内容不会丢失"); };
  }

  useEffect(() => {
    if (!session || !activeTurn || eventSourceRef.current) return;
    setSession((current) => current ? { ...current, messages: current.messages.map((item) => item.id === activeTurn.assistant_message_id ? { ...item, content: "" } : item) } : current);
    connectToTurn(activeTurn.id);
    // The stream function only mutates state through stable setters and callbacks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTurn?.id, session?.id]);

  async function sendMessage(content: string, memoryAction?: { kind: "correct"; memory_record_id: string; new_content: string } | { kind: "forget"; memory_record_id: string }, targetSessionId?: string) {
    const sessionId = targetSessionId ?? session?.id;
    if (!sessionId || !content.trim() || busy || activeTurn) return;
    setBusy(true); setError(null); setNotice(null);
    const id = crypto.randomUUID();
    try {
      const payload = await fetchJson<{ turn: Turn }>("/api/chat/turns", {
        method: "POST", headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_id: sessionId, message: content.trim(), client_message_id: `web-${id}`, idempotency_key: `web-turn-${id}`, memory_action: memoryAction }),
      });
      await refreshSession(sessionId);
      scrollConversationToBottom(true);
      connectToTurn(payload.turn.id);
      setMessage("");
    } catch (cause) {
      setBusy(false); setError(cause instanceof Error ? cause.message : "消息发送失败");
    }
  }
  async function submit(event: FormEvent) { event.preventDefault(); await sendMessage(message); }
  async function runGuidedStep(index: number) {
    const step = guidedSteps[index];
    if (step.action === "new_chat") {
      if (activeMemories.length === 0) return setError("请先完成第 1 步，让系统自然认识你。");
      const created = await createSession("guided");
      return sendMessage(step.message, undefined, created.id);
    }
    if (step.action === "project_update" && !projectMemory) {
      return setError("当前还没有项目事实，请先完成第 1 步。");
    }
    if (step.action === "forget_experience") {
      if (!experienceMemory) return setError("当前还没有可忘记的历史经历，请先完成第 1 步。");
      return sendMessage(step.message, { kind: "forget", memory_record_id: experienceMemory.memory_record_id });
    }
    return sendMessage(step.message);
  }
  async function stopGeneration() {
    if (!activeTurn) return;
    await fetchJson(`/api/chat/turns/${activeTurn.id}/cancel`, { method: "POST" });
    setNotice("已请求停止；此前已保存的记忆仍可单独撤销。");
  }
  async function forgetMemory(memoryId: string) {
    try { await fetchJson(`/api/memories/${memoryId}/forget`, { method: "POST" }); setNotice("已立即停止使用这条记忆"); await refreshMemories(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "忘记失败"); }
  }
  async function eraseMemory(memoryId: string) {
    if (!window.confirm("物理 Erase 会清除内容与索引，且不可恢复。确认继续？")) return;
    try { await fetchJson(`/api/memories/${memoryId}/erase`, { method: "POST" }); setNotice("物理清除请求已受理，后台正在完成索引删除"); await refreshMemories(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "清除失败"); }
  }
  async function resetDemo() {
    if (!window.confirm("这会物理清除演示用户的记忆和全部演示对话。确认重置？")) return;
    setBusy(true);
    try {
      const payload = await fetchJson<{ session: SessionSummary }>("/api/demo/reset", { method: "POST" });
      setSelectedSessionId(payload.session.id);
      await Promise.all([refreshSessions(), refreshSession(payload.session.id), refreshMemories()]);
      setNotice("演示环境已恢复为空白状态");
    } catch (cause) { setError(cause instanceof Error ? cause.message : "重置失败"); }
    finally { setBusy(false); }
  }

  return (
    <main className="app-shell">
      <aside className="conversation-sidebar">
        <div className="brand-lockup"><div className="brand-mark">L</div><div><strong>LazyTT Memory</strong><span>会记忆的 AI</span></div></div>
        <button className="new-chat" onClick={() => createSession("free")} type="button"><span>＋</span> 新建对话</button>
        <div className="conversation-label">对话</div>
        <div className="conversation-list">
          {sessions.map((item) => <button className={item.id === selectedSessionId ? "conversation active" : "conversation"} key={item.id} onClick={() => selectSession(item.id)} type="button"><span className="conversation-icon">{item.mode === "guided" ? "✦" : "○"}</span><span><strong>{item.title}</strong><small>{item.mode === "guided" ? "引导演示" : "自由对话"}</small></span></button>)}
        </div>
        <div className="sidebar-bottom"><div className={`service-status ${health?.ready ? "ready" : ""}`}><span />{health?.ready ? "真实链路已就绪" : "链路需要检查"}</div><button className="text-button" onClick={resetDemo} type="button">重置演示数据</button></div>
      </aside>

      <section className="chat-column">
        <header className="chat-header"><div><h1>{session?.title ?? "记忆演示"}</h1><p>{session?.mode === "free" ? "像普通大模型一样聊，系统会自动判断什么值得记住" : "一条自然对话，验证它了解你、项目和过去经验"}</p></div><div className="header-actions"><span className="privacy-badge">个人记忆 + 项目记忆</span><button className="memory-mobile" onClick={() => setMobileMemoryOpen(true)} type="button">记忆 {activeMemories.length}</button></div></header>

        {(session?.mode ?? "guided") === "guided" && <section className="guided-strip" aria-label="统一演示流程"><div className="guided-heading"><span>一条自然演示路线</span><small>稳定信息自动记，事实变化自动更新并提示</small></div><div className="guided-steps">{guidedSteps.map((step, index) => <button key={step.number} onClick={() => runGuidedStep(index)} disabled={busy || Boolean(activeTurn)} type="button"><span className="step-number">{step.number}</span><span><strong>{step.title}</strong><small>{step.hint}</small></span><span className="step-arrow">→</span></button>)}</div></section>}

        <div className="message-scroller" ref={messageScrollerRef}>
          {(!session || session.messages.length === 0) && <div className="welcome-state"><div className="welcome-orb">✦</div><h2>你好，我会逐渐了解你。</h2><p>像平常聊天一样说，系统会自动区分偏好、事实、项目和经历。</p><div className="starter-grid"><button onClick={() => runGuidedStep(0)} type="button"><span>01</span><strong>开始完整演示</strong><small>一次自然介绍，形成多类记忆</small></button><button onClick={() => sendMessage("我是 Agent 研发工程师，主要使用 Rust。结合这个背景给我介绍事务型 Outbox。")} type="button"><span>02</span><strong>自由聊聊</strong><small>看看系统会自动记住什么</small></button></div></div>}
          <div className="messages">
            {session?.messages.map((item) => {
              const turn = item.turn_id ? session.turns.find((candidate) => candidate.id === item.turn_id) : undefined;
              const updatedEffect = turn?.memory_effects?.find((effect) => effect.kind === "corrected");
              return <article className={`message-row ${item.role}`} key={item.id}>{item.role === "assistant" && <div className="assistant-avatar">L</div>}<div className="message-content"><div className="message-meta">{item.role === "user" ? "你" : "LazyTT"}</div><div className="message-bubble">{item.content ? (item.role === "assistant" ? <MarkdownMessage content={item.content} /> : item.content) : (item.status === "pending" || item.status === "streaming" ? <Typing /> : "")}</div>{item.role === "assistant" && turn && <>{updatedEffect && <div className="memory-update-note"><span>✓</span>{updatedEffect.content ? `已自动更新记忆：${updatedEffect.content}` : "已自动更新记忆，并在本轮生效"}</div>}<button className="memory-usage" onClick={() => setEvidenceTurnId((open) => open === turn.id ? null : turn.id)} type="button"><span className="usage-dot">✓</span>本次使用了 {turn.used_memory_ids?.length ?? 0} 条记忆<span>{evidenceTurnId === turn.id ? "收起" : "查看来源"}</span></button>{evidenceTurnId === turn.id && <Evidence turn={turn} memories={memories} />}</>}</div></article>;
            })}
          </div>
        </div>

        <div className="composer-area">
          {notice && <div className="notice-toast"><span>✓</span>{notice}<button onClick={() => setNotice(null)} type="button">×</button></div>}
          {error && <div className="error-toast"><span>!</span>{error}<button onClick={() => setError(null)} type="button">×</button></div>}
          <form className="composer" onSubmit={submit}><textarea aria-label="发送消息" disabled={!health?.ready || busy} onChange={(event) => setMessage(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} placeholder="和普通 AI 一样输入消息；值得长期使用的信息会自动保存…" rows={1} value={message} />{activeTurn ? <button className="stop-button" onClick={stopGeneration} type="button" aria-label="停止生成">■</button> : <button className="send-button" disabled={!message.trim() || busy || !health?.ready} type="submit" aria-label="发送">↑</button>}</form>
          <p className="composer-note">无需固定口令；稳定偏好、用户事实、项目事实和相关经历会按权限自动处理，密码与临时指令不会保存。</p>
        </div>
      </section>

      <MemoryPanel className={mobileMemoryOpen ? "memory-panel mobile-open" : "memory-panel"} memories={memories} onClose={() => setMobileMemoryOpen(false)} onErase={eraseMemory} onForget={forgetMemory} />
      {mobileMemoryOpen && <button className="mobile-backdrop" onClick={() => setMobileMemoryOpen(false)} aria-label="关闭记忆面板" type="button" />}
    </main>
  );
}

function MemoryPanel({ className, memories, onClose, onErase, onForget }: { className: string; memories: MemoryRecord[]; onClose: () => void; onErase: (id: string) => void; onForget: (id: string) => void }) {
  const active = memories.filter((memory) => memory.head_state === "active" && memory.current_version_id);
  const inactive = memories.filter((memory) => memory.head_state !== "active");
  return <aside className={className}><header><div><span className="memory-spark">✦</span><div><h2>它记住了什么</h2><p>自动判断，始终可撤销</p></div></div><button onClick={onClose} type="button" aria-label="关闭">×</button></header><div className="memory-explainer"><strong>{active.length}</strong><span>条当前可用记忆</span><p>系统自动区分个人与项目信息，只有相关且仍获授权的记忆才进入回答。</p></div><div className="memory-list">{active.length === 0 && <div className="empty-memory"><span>◇</span><strong>这里还是空的</strong><p>像平常聊天一样介绍自己、项目或经历，系统会自动判断是否值得保存。</p></div>}{active.map((memory) => {
    const current = memory.versions.find((version) => version.memory_version_id === memory.current_version_id);
    return <details className="memory-card" key={memory.memory_record_id}><summary><div className="memory-card-labels"><span className={`memory-type ${memory.memory_type}`}>{memoryTypeLabel[memory.memory_type] ?? memory.memory_type}</span><span className={`scope-badge ${memory.scope.kind}`}>{memory.scope.kind === "project" ? "项目共享" : "仅你可见"}</span></div><p>{current?.content ?? "内容不可用"}</p><div className="memory-card-meta"><span>版本 {current?.version_no ?? memory.versions.length}</span><span>{current?.source_refs.length ?? 0} 个来源</span></div></summary><div className="memory-detail"><div className="memory-policy-line"><span>可影响</span><strong>{memoryAllowedEffect[memory.memory_type] ?? "仅限当前用途"}</strong></div><div className="version-line">{memory.versions.filter((version) => !version.content_erased_at).map((version) => <div key={version.memory_version_id}><span>v{version.version_no}</span><p>{version.content}</p></div>)}</div><div className="memory-actions"><button onClick={() => onForget(memory.memory_record_id)} type="button">忘记</button><button className="danger" onClick={() => onErase(memory.memory_record_id)} type="button">物理清除</button></div></div></details>;
  })}{inactive.length > 0 && <div className="inactive-count">另有 {inactive.length} 条争议中、已忘记或已清除记录</div>}</div><footer><span>权威档案</span><strong>PostgreSQL</strong><span>搜索目录</span><strong>Mem0</strong></footer></aside>;
}

function Evidence({ turn, memories }: { turn: Turn; memories: MemoryRecord[] }) {
  const used = turn.used_memory_ids.map((id, index) => {
    const memory = memories.find((candidate) => candidate.memory_record_id === id);
    const versionId = turn.used_memory_version_ids?.[index];
    const version = memory?.versions.find((candidate) => candidate.memory_version_id === versionId);
    return memory && version ? { memory, version } : null;
  }).filter((entry): entry is { memory: MemoryRecord; version: MemoryVersion } => Boolean(entry));
  return <div className="evidence-drawer"><div className="evidence-summary"><span>本轮上下文</span><strong>{used.length === 0 ? "没有使用长期记忆" : `${used.length} 条已授权记忆`}</strong></div>{used.map(({ memory, version }) => {
    return <article className="evidence-memory" key={version.memory_version_id}><div><span className={`memory-type ${memory.memory_type}`}>{memoryTypeLabel[memory.memory_type] ?? memory.memory_type}</span><small>{memory.scope.kind === "project" ? "项目范围" : "个人范围"}</small></div><strong>{version.content ?? "内容不可用"}</strong><p>用于：{memoryAllowedEffect[memory.memory_type] ?? "回答个性化"} · 来源 {version.source_refs.length} 个 · v{version.version_no}</p></article>;
  })}<div className="evidence-meta"><div><span>Context Pack</span><code>{turn.context_pack_id ? shortId(turn.context_pack_id) : "fallback"}</code></div><div><span>Outcome Receipt</span><code>{turn.outcome_receipt_id ? shortId(turn.outcome_receipt_id) : "pending"}</code></div><div><span>模型</span><code>{turn.model_id ?? "k3"}</code></div></div></div>;
}
function Typing() { return <span className="typing"><i /><i /><i /></span>; }
function shortId(value: string) { return `${value.slice(0, 8)}…${value.slice(-4)}`; }
