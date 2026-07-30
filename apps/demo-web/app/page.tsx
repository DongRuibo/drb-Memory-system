"use client";

import {
  FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { EvidenceDrawer } from "../components/EvidenceDrawer";
import {
  GuidedDemo,
  guidedSteps,
  type GuidedAction,
} from "../components/GuidedDemo";
import { MemoryPanel } from "../components/MemoryPanel";
import { UserSwitcher } from "../components/UserSwitcher";
import { MarkdownMessage } from "./markdown";
import type {
  ApiError,
  DemoBootstrap,
  DemoUserProfile,
  Health,
  MemoryEffect,
  MemoryRecord,
  SessionDetail,
  SessionSummary,
  Turn,
} from "./types";

const API_BASE = process.env.NEXT_PUBLIC_DEMO_API_URL ?? "http://localhost:8090";

export default function Home() {
  const [health, setHealth] = useState<Health | null>(null);
  const [currentUser, setCurrentUser] = useState<DemoUserProfile | null>(null);
  const [availableUsers, setAvailableUsers] = useState<DemoUserProfile[]>([]);
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
  const identityRevisionRef = useRef(0);
  const identityEpochRef = useRef(0);

  function scrollConversationToBottom(force = false) {
    requestAnimationFrame(() => {
      const scroller = messageScrollerRef.current;
      if (!scroller) return;
      const distanceFromBottom =
        scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
      if (force || distanceFromBottom < 180) {
        scroller.scrollTo({
          top: scroller.scrollHeight,
          behavior: force ? "auto" : "smooth",
        });
      }
    });
  }

  const fetchJson = useCallback(
    async <T,>(path: string, init?: RequestInit): Promise<T> => {
      const headers = new Headers(init?.headers);
      if (identityRevisionRef.current > 0) {
        headers.set(
          "x-demo-identity-revision",
          String(identityRevisionRef.current),
        );
      }
      const response = await fetch(`${API_BASE}${path}`, {
        ...init,
        credentials: "include",
        headers,
      });
      const payload = (await response.json().catch(() => ({}))) as T & ApiError;
      if (!response.ok) {
        throw new Error(payload.detail ?? `请求失败 (${response.status})`);
      }
      return payload;
    },
    [],
  );

  const applyBootstrap = useCallback((payload: DemoBootstrap) => {
    identityRevisionRef.current = payload.identity_revision;
    setCurrentUser(payload.current_user);
    setAvailableUsers(payload.available_users);
    setHealth(payload.health);
    setSessions(payload.sessions);
    setMemories(payload.memories.records);
    setSelectedSessionId(payload.sessions[0]?.id ?? null);
    setSession(null);
    setEvidenceTurnId(null);
  }, []);

  const refreshMemories = useCallback(async () => {
    const payload = await fetchJson<{ memories: { records: MemoryRecord[] } }>(
      "/api/memories",
    );
    setMemories(payload.memories.records);
  }, [fetchJson]);

  const refreshSession = useCallback(
    async (sessionId: string) => {
      const payload = await fetchJson<{ session: SessionDetail }>(
        `/api/chat/sessions/${sessionId}`,
      );
      setSession(payload.session);
      return payload.session;
    },
    [fetchJson],
  );

  const refreshSessions = useCallback(async () => {
    const payload = await fetchJson<{ sessions: SessionSummary[] }>(
      "/api/chat/sessions",
    );
    setSessions(payload.sessions);
    return payload.sessions;
  }, [fetchJson]);

  const createSession = useCallback(
    async (mode: "guided" | "free" = "guided") => {
      const payload = await fetchJson<{ session: SessionSummary }>(
        "/api/chat/sessions",
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            title: mode === "guided" ? "记忆演示" : "新的对话",
            mode,
          }),
        },
      );
      setSelectedSessionId(payload.session.id);
      await Promise.all([
        refreshSessions(),
        refreshSession(payload.session.id),
      ]);
      return payload.session;
    },
    [fetchJson, refreshSession, refreshSessions],
  );

  useEffect(() => {
    let cancelled = false;
    const epoch = ++identityEpochRef.current;
    (async () => {
      try {
        const payload = await fetchJson<DemoBootstrap>("/api/demo/bootstrap");
        if (cancelled || epoch !== identityEpochRef.current) return;
        applyBootstrap(payload);
        if (payload.sessions.length === 0) {
          await createSession("guided");
        } else {
          await refreshSession(payload.sessions[0].id);
        }
      } catch (cause) {
        if (cancelled) return;
        const detail =
          cause instanceof Error ? cause.message : "演示服务不可用";
        setHealth({
          ready: false,
          memory_service: "unavailable",
          chat_store: "unavailable",
          model_configuration: "check_required",
          detail,
        });
        setError(detail);
      }
    })();
    return () => {
      cancelled = true;
      eventSourceRef.current?.close();
    };
  }, [applyBootstrap, createSession, fetchJson, refreshSession]);

  const activeMemories = useMemo(
    () =>
      memories.filter(
        (memory) =>
          memory.head_state === "active" && memory.current_version_id,
      ),
    [memories],
  );
  const projectMemory = activeMemories.find(
    (memory) =>
      memory.memory_type === "semantic_fact" &&
      memory.scope.kind === "project",
  );
  const experienceMemory = activeMemories.find(
    (memory) => memory.memory_type === "episodic_summary",
  );
  const activeTurn = session?.turns.find((turn) =>
    ["accepted", "processing", "streaming"].includes(turn.status),
  );

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const scroller = messageScrollerRef.current;
      if (scroller) scroller.scrollTop = scroller.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
  }, [session?.id]);

  async function selectSession(sessionId: string) {
    eventSourceRef.current?.close();
    eventSourceRef.current = null;
    setSelectedSessionId(sessionId);
    try {
      await refreshSession(sessionId);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "无法加载对话");
    }
  }

  function connectToTurn(turnId: string, sessionId: string) {
    eventSourceRef.current?.close();
    let lastSequence = 0;
    const epoch = identityEpochRef.current;
    const source = new EventSource(
      `${API_BASE}/api/chat/turns/${turnId}/events?after=0`,
      { withCredentials: true },
    );
    eventSourceRef.current = source;
    const eventTypes = [
      "turn.accepted",
      "turn.processing",
      "memory.processing",
      "memory.effects",
      "memory.warning",
      "context.ready",
      "answer.delta",
      "evidence.warning",
      "turn.cancel_requested",
      "turn.completed",
      "turn.failed",
      "turn.cancelled",
    ];
    const onEvent = (raw: Event) => {
      if (epoch !== identityEpochRef.current) {
        source.close();
        return;
      }
      const parsed = JSON.parse((raw as MessageEvent<string>).data) as {
        sequence_no: number;
        type: string;
        payload: Record<string, unknown>;
      };
      if (parsed.sequence_no <= lastSequence) return;
      lastSequence = parsed.sequence_no;
      if (parsed.type === "answer.delta") {
        const delta = String(parsed.payload.delta ?? "");
        setSession((current) =>
          current
            ? {
                ...current,
                messages: current.messages.map((item) =>
                  item.turn_id === turnId && item.role === "assistant"
                    ? {
                        ...item,
                        content: item.content + delta,
                        status: "streaming",
                      }
                    : item,
                ),
              }
            : current,
        );
        scrollConversationToBottom();
      }
      if (parsed.type === "memory.effects") {
        showMemoryEffectNotice(
          (parsed.payload.effects ?? []) as MemoryEffect[],
          setNotice,
        );
      }
      if (
        parsed.type === "memory.warning" ||
        parsed.type === "evidence.warning"
      ) {
        setNotice(
          String(
            parsed.payload.detail ??
              "回答已继续，但部分记忆证据暂不可用",
          ),
        );
      }
      if (
        ["turn.completed", "turn.failed", "turn.cancelled"].includes(parsed.type)
      ) {
        source.close();
        if (eventSourceRef.current === source) eventSourceRef.current = null;
        setBusy(false);
        Promise.all([
          refreshSession(sessionId),
          refreshSessions(),
          refreshMemories(),
        ]).catch(() => undefined);
      }
    };
    eventTypes.forEach((type) => source.addEventListener(type, onEvent));
    source.onerror = () => {
      if (
        eventSourceRef.current === source &&
        epoch === identityEpochRef.current
      ) {
        setNotice("连接正在自动恢复，已生成内容不会丢失");
      }
    };
  }

  useEffect(() => {
    if (!session || !activeTurn || eventSourceRef.current) return;
    setSession((current) =>
      current
        ? {
            ...current,
            messages: current.messages.map((item) =>
              item.id === activeTurn.assistant_message_id
                ? { ...item, content: "" }
                : item,
            ),
          }
        : current,
    );
    connectToTurn(activeTurn.id, session.id);
    // connectToTurn intentionally tracks the currently loaded turn only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTurn?.id, session?.id]);

  async function sendMessage(
    content: string,
    memoryAction?:
      | { kind: "correct"; memory_record_id: string; new_content: string }
      | { kind: "forget"; memory_record_id: string },
    targetSessionId?: string,
  ) {
    const sessionId = targetSessionId ?? session?.id;
    if (!sessionId || !content.trim() || busy || activeTurn) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    const id = crypto.randomUUID();
    try {
      const payload = await fetchJson<{ turn: Turn }>("/api/chat/turns", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          session_id: sessionId,
          message: content.trim(),
          client_message_id: `web-${id}`,
          idempotency_key: `web-turn-${id}`,
          memory_action: memoryAction,
        }),
      });
      await refreshSession(sessionId);
      scrollConversationToBottom(true);
      connectToTurn(payload.turn.id, sessionId);
      setMessage("");
    } catch (cause) {
      setBusy(false);
      setError(cause instanceof Error ? cause.message : "消息发送失败");
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    await sendMessage(message);
  }

  async function runGuidedStep(action: GuidedAction) {
    const step = guidedSteps.find((candidate) => candidate.action === action);
    if (!step) return;
    if (action === "inspect") {
      if (activeMemories.length === 0) {
        setError("请先完成第 1 步，让系统形成记忆。");
        return;
      }
      setMobileMemoryOpen(true);
      setNotice("展开右侧记忆，可检查类型、范围、事件时间、结构化内容和版本。");
      return;
    }
    if (action === "new_chat") {
      if (activeMemories.length === 0) {
        setError("请先完成第 1 步，让系统自然认识你。");
        return;
      }
      const created = await createSession("guided");
      await sendMessage(step.message ?? "", undefined, created.id);
      return;
    }
    if (action === "time_recall" && !experienceMemory) {
      setError("当前还没有带事件时间的经历摘要，请先完成第 1 步。");
      return;
    }
    if (action === "project_update" && !projectMemory) {
      setError("当前还没有项目事实，请先完成第 1 步。");
      return;
    }
    if (action === "forget_experience") {
      if (!experienceMemory) {
        setError("当前还没有可忘记的经历摘要，请先完成第 1 步。");
        return;
      }
      await sendMessage(step.message ?? "", {
        kind: "forget",
        memory_record_id: experienceMemory.memory_record_id,
      });
      return;
    }
    await sendMessage(step.message ?? "");
  }

  async function switchUser(userId: string) {
    if (!currentUser || userId === currentUser.user_id || busy || activeTurn) {
      return;
    }
    setBusy(true);
    setError(null);
    setNotice(null);
    eventSourceRef.current?.close();
    eventSourceRef.current = null;
    const epoch = ++identityEpochRef.current;
    setSession(null);
    setSessions([]);
    setMemories([]);
    setSelectedSessionId(null);
    try {
      const payload = await fetchJson<DemoBootstrap>("/api/demo/users/switch", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ user_id: userId }),
      });
      if (epoch !== identityEpochRef.current) return;
      applyBootstrap(payload);
      if (payload.sessions.length === 0) {
        await createSession("guided");
      } else {
        await refreshSession(payload.sessions[0].id);
      }
      setNotice(
        `已切换为 ${payload.current_user.display_name}：个人对话和个人记忆已隔离，项目记忆按成员权限共享。`,
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "切换用户失败");
      const recovered = await fetchJson<DemoBootstrap>(
        "/api/demo/bootstrap",
      ).catch(() => null);
      if (recovered) {
        applyBootstrap(recovered);
        if (recovered.sessions[0]) {
          await refreshSession(recovered.sessions[0].id);
        }
      }
    } finally {
      setBusy(false);
    }
  }

  async function stopGeneration() {
    if (!activeTurn) return;
    await fetchJson(`/api/chat/turns/${activeTurn.id}/cancel`, {
      method: "POST",
    });
    setNotice("已请求停止；此前已保存的记忆仍可单独撤销。");
  }

  async function forgetMemory(memoryId: string) {
    try {
      await fetchJson(`/api/memories/${memoryId}/forget`, { method: "POST" });
      setNotice("已立即停止使用这条记忆");
      await refreshMemories();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "忘记失败");
    }
  }

  async function eraseMemory(memoryId: string) {
    if (
      !window.confirm("物理清除会删除内容与索引，且不可恢复。确认继续？")
    ) {
      return;
    }
    try {
      await fetchJson(`/api/memories/${memoryId}/erase`, { method: "POST" });
      setNotice("物理清除请求已受理，后台正在完成索引删除");
      await refreshMemories();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "清除失败");
    }
  }

  async function resetCurrentUser() {
    if (
      !window.confirm(
        `这会物理清除 ${currentUser?.display_name ?? "当前用户"} 的个人记忆和全部个人对话，不影响共享项目记忆。确认重置？`,
      )
    ) {
      return;
    }
    setBusy(true);
    try {
      const payload = await fetchJson<{ session: SessionSummary }>(
        "/api/demo/reset",
        { method: "POST" },
      );
      setSelectedSessionId(payload.session.id);
      await Promise.all([
        refreshSessions(),
        refreshSession(payload.session.id),
        refreshMemories(),
      ]);
      setNotice("当前用户的个人记忆与对话已清空；共享项目记忆未改变。");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "重置失败");
    } finally {
      setBusy(false);
    }
  }

  async function resetProject() {
    if (
      !window.confirm(
        `这会物理清除 ${currentUser?.primary_project_label ?? "当前项目"} 的共享项目记忆，影响所有项目成员。确认重置？`,
      )
    ) {
      return;
    }
    setBusy(true);
    try {
      await fetchJson("/api/demo/reset-project", { method: "POST" });
      await refreshMemories();
      setNotice("共享项目记忆已清空；成员的个人记忆与对话未改变。");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "项目重置失败");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="app-shell">
      <aside className="conversation-sidebar">
        <div className="brand-lockup">
          <div className="brand-mark">L</div>
          <div>
            <strong>LazyTT Memory</strong>
            <span>会记忆的 AI</span>
          </div>
        </div>
        <UserSwitcher
          currentUser={currentUser}
          disabled={busy || Boolean(activeTurn)}
          onSwitch={switchUser}
          users={availableUsers}
        />
        <button
          className="new-chat"
          disabled={busy}
          onClick={() => createSession("free")}
          type="button"
        >
          <span>＋</span> 新建对话
        </button>
        <div className="conversation-label">当前用户的对话</div>
        <div className="conversation-list">
          {sessions.map((item) => (
            <button
              className={
                item.id === selectedSessionId
                  ? "conversation active"
                  : "conversation"
              }
              key={item.id}
              onClick={() => selectSession(item.id)}
              type="button"
            >
              <span className="conversation-icon">
                {item.mode === "guided" ? "✦" : "○"}
              </span>
              <span>
                <strong>{item.title}</strong>
                <small>
                  {item.mode === "guided" ? "引导演示" : "自由对话"}
                </small>
              </span>
            </button>
          ))}
        </div>
        <div className="sidebar-bottom">
          <div className={`service-status ${health?.ready ? "ready" : ""}`}>
            <span />
            {health?.ready ? "真实链路已就绪" : "链路需要检查"}
          </div>
          <button
            className="text-button"
            disabled={busy}
            onClick={resetCurrentUser}
            type="button"
          >
            重置当前用户
          </button>
          {currentUser?.can_reset_project && (
            <button
              className="text-button danger-text"
              disabled={busy}
              onClick={resetProject}
              type="button"
            >
              重置共享项目
            </button>
          )}
        </div>
      </aside>

      <section className="chat-column">
        <header className="chat-header">
          <div>
            <h1>{session?.title ?? "记忆演示"}</h1>
            <p>
              {session?.mode === "free"
                ? "自然聊天，系统自动判断什么值得长期记住"
                : "验证分类、范围、时间、结构、版本和遗忘"}
            </p>
          </div>
          <div className="header-actions">
            <span className="active-principal">
              <b>{currentUser?.avatar_initial ?? "?"}</b>
              {currentUser?.display_name ?? "正在加载"}
            </span>
            <span className="privacy-badge">个人隔离 · 项目共享</span>
            <button
              className="memory-mobile"
              onClick={() => setMobileMemoryOpen(true)}
              type="button"
            >
              记忆 {activeMemories.length}
            </button>
          </div>
        </header>

        {(session?.mode ?? "guided") === "guided" && (
          <GuidedDemo
            disabled={busy || Boolean(activeTurn)}
            onRun={runGuidedStep}
          />
        )}

        <div className="message-scroller" ref={messageScrollerRef}>
          {(!session || session.messages.length === 0) && (
            <div className="welcome-state">
              <div className="welcome-orb">✦</div>
              <h2>
                你好，{currentUser?.display_name ?? "我会逐渐了解你"}。
              </h2>
              <p>
                类型说明记忆是什么，范围决定谁能看到；事件时间与结构化字段均可审计。
              </p>
              <div className="starter-grid">
                <button
                  onClick={() => runGuidedStep("introduce")}
                  type="button"
                >
                  <span>01</span>
                  <strong>开始六步演示</strong>
                  <small>一次自然介绍，形成多类记忆</small>
                </button>
                <button
                  onClick={() =>
                    sendMessage(
                      "我是 Agent 研发工程师，主要使用 Rust。结合这个背景给我介绍事务型 Outbox。",
                    )
                  }
                  type="button"
                >
                  <span>02</span>
                  <strong>自由聊聊</strong>
                  <small>看看系统会自动记住什么</small>
                </button>
              </div>
            </div>
          )}
          <div className="messages">
            {session?.messages.map((item) => {
              const turn = item.turn_id
                ? session.turns.find(
                    (candidate) => candidate.id === item.turn_id,
                  )
                : undefined;
              const updatedEffect = turn?.memory_effects?.find(
                (effect) => effect.kind === "corrected",
              );
              const extractionSummary = turn?.memory_effects?.find(
                (effect) => effect.kind === "extraction_summary",
              );
              return (
                <article
                  className={`message-row ${item.role}`}
                  key={item.id}
                >
                  {item.role === "assistant" && (
                    <div className="assistant-avatar">L</div>
                  )}
                  <div className="message-content">
                    <div className="message-meta">
                      {item.role === "user"
                        ? currentUser?.display_name ?? "你"
                        : "LazyTT"}
                    </div>
                    <div className="message-bubble">
                      {item.content ? (
                        item.role === "assistant" ? (
                          <MarkdownMessage content={item.content} />
                        ) : (
                          item.content
                        )
                      ) : item.status === "pending" ||
                        item.status === "streaming" ? (
                        <Typing />
                      ) : (
                        ""
                      )}
                    </div>
                    {item.role === "assistant" && turn && (
                      <>
                        {updatedEffect && (
                          <div className="memory-update-note">
                            <span>✓</span>
                            {updatedEffect.content
                              ? `已自动更新记忆：${updatedEffect.content}`
                              : "已生成记忆新版本，并在本轮生效"}
                          </div>
                        )}
                        {extractionSummary && (
                          <div className="memory-extraction-note">
                            <strong>
                              本轮形成{" "}
                              {extractionSummary.candidate_count ?? 0}{" "}
                              条有效记忆候选
                            </strong>
                            <span>
                              新增 {extractionSummary.remembered_count ?? 0} ·
                              合并来源{" "}
                              {extractionSummary.source_merged_count ?? 0} ·
                              更新 {extractionSummary.corrected_count ?? 0}
                              {(extractionSummary.coverage_added_count ?? 0) >
                                0 &&
                                ` · 受限覆盖补齐 ${extractionSummary.coverage_added_count} 条`}
                              {filteredCandidateCount(extractionSummary) > 0 &&
                                ` · 过滤/降级 ${filteredCandidateCount(extractionSummary)} 项`}
                            </span>
                            {filteredCandidateCount(extractionSummary) > 0 && (
                              <small>
                                过滤详情：
                                {formatFilteredReasons(extractionSummary)}
                              </small>
                            )}
                          </div>
                        )}
                        <button
                          className="memory-usage"
                          onClick={() =>
                            setEvidenceTurnId((open) =>
                              open === turn.id ? null : turn.id,
                            )
                          }
                          type="button"
                        >
                          <span className="usage-dot">✓</span>
                          本次使用了 {turn.used_memory_ids?.length ?? 0} 条记忆
                          <span>
                            {evidenceTurnId === turn.id
                              ? "收起"
                              : "查看版本证据"}
                          </span>
                        </button>
                        {evidenceTurnId === turn.id && (
                          <EvidenceDrawer turn={turn} />
                        )}
                      </>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
        </div>

        <div className="composer-area">
          {notice && (
            <div className="notice-toast">
              <span>✓</span>
              {notice}
              <button onClick={() => setNotice(null)} type="button">
                ×
              </button>
            </div>
          )}
          {error && (
            <div className="error-toast">
              <span>!</span>
              {error}
              <button onClick={() => setError(null)} type="button">
                ×
              </button>
            </div>
          )}
          <form className="composer" onSubmit={submit}>
            <textarea
              aria-label="发送消息"
              disabled={!health?.ready || busy}
              onChange={(event) => setMessage(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  event.currentTarget.form?.requestSubmit();
                }
              }}
              placeholder="自然输入；系统按类型、范围和用途治理值得长期使用的信息…"
              rows={1}
              value={message}
            />
            {activeTurn ? (
              <button
                className="stop-button"
                onClick={stopGeneration}
                type="button"
                aria-label="停止生成"
              >
                ■
              </button>
            ) : (
              <button
                className="send-button"
                disabled={!message.trim() || busy || !health?.ready}
                type="submit"
                aria-label="发送"
              >
                ↑
              </button>
            )}
          </form>
          <p className="composer-note">
            用户偏好、用户资料、事实知识与经历摘要是类型；个人、项目、租户与 Agent
            是可见范围。密码和临时指令不会保存。
          </p>
        </div>
      </section>

      <MemoryPanel
        className={
          mobileMemoryOpen ? "memory-panel mobile-open" : "memory-panel"
        }
        memories={memories}
        onClose={() => setMobileMemoryOpen(false)}
        onErase={eraseMemory}
        onForget={forgetMemory}
      />
      {mobileMemoryOpen && (
        <button
          className="mobile-backdrop"
          onClick={() => setMobileMemoryOpen(false)}
          aria-label="关闭记忆面板"
          type="button"
        />
      )}
    </main>
  );
}

function showMemoryEffectNotice(
  effects: MemoryEffect[],
  setNotice: (message: string) => void,
) {
  const summary = effects.find(
    (effect) => effect.kind === "extraction_summary",
  );
  const remembered = effects.filter((effect) => effect.kind === "remembered");
  const merged = effects.filter((effect) => effect.kind === "source_merged");
  const corrected = effects.filter((effect) => effect.kind === "corrected");
  const forgotten = effects.filter((effect) => effect.kind === "forgotten");
  if (summary && (summary.candidate_count ?? 0) > 0) {
    const coverage =
      (summary.coverage_added_count ?? 0) > 0
        ? `，其中受限覆盖检查补齐 ${summary.coverage_added_count} 条`
        : "";
    setNotice(
      `本轮形成 ${summary.candidate_count} 条有效候选：新增 ${summary.remembered_count ?? 0}，合并来源 ${summary.source_merged_count ?? 0}${coverage}`,
    );
  } else if (remembered.length) {
    setNotice(`已自动记住 ${remembered.length} 条信息，可在右侧检查结构和范围`);
  } else if (merged.length) {
    setNotice(`没有新增重复记忆，已为 ${merged.length} 条记忆补充来源`);
  } else if (corrected.length) {
    setNotice(
      corrected[0].content
        ? `已自动生成新版本：${corrected[0].content}`
        : "已自动生成记忆新版本，并在本轮生效",
    );
  } else if (forgotten.length) {
    setNotice("已立即停止使用指定记忆，其他记忆不受影响");
  }
}

function filteredCandidateCount(effect: MemoryEffect) {
  return Object.values(effect.filtered_reason_counts ?? {}).reduce(
    (total, count) => total + count,
    0,
  );
}

function formatFilteredReasons(effect: MemoryEffect) {
  const labels: Record<string, string> = {
    unauthorized_memory_type: "类型未获授权",
    invalid_semantic_key: "记忆标识不规范",
    invalid_content: "内容不合规",
    invalid_score: "置信度或重要性不合格",
    invalid_event_time: "事件时间不合规",
    invalid_structured_content: "结构化字段降级",
    duplicate_identity: "重复候选",
    candidate_limit: "超过候选上限",
    coverage_candidate_limit: "受限覆盖超过上限",
  };
  return Object.entries(effect.filtered_reason_counts ?? {})
    .filter(([, count]) => count > 0)
    .map(([reason, count]) => `${labels[reason] ?? reason} ${count}`)
    .join("，");
}

function Typing() {
  return (
    <span className="typing">
      <i />
      <i />
      <i />
    </span>
  );
}
