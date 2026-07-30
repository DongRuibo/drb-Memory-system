import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

async function render() {
  const workerUrl = new URL("../dist/server/index.js", import.meta.url);
  workerUrl.searchParams.set("test", `${process.pid}-${Date.now()}`);
  const { default: worker } = await import(workerUrl.href);

  return worker.fetch(
    new Request("http://localhost/", {
      headers: { accept: "text/html" },
    }),
    {
      ASSETS: {
        fetch: async () => new Response("Not found", { status: 404 }),
      },
    },
    {
      waitUntil() {},
      passThroughOnException() {},
    },
  );
}

test("server-renders the interactive memory agent", async () => {
  const response = await render();
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /^text\/html\b/i);

  const html = await response.text();
  assert.match(html, /<title>LazyTT Memory · 会记忆的 AI<\/title>/i);
  assert.match(html, /会记忆的 AI/);
  assert.match(html, /当前演示身份/);
  assert.match(html, /个人记忆彼此隔离；同项目成员共享项目事实/);
  assert.match(html, /六步验证核心能力/);
  assert.match(html, /自然形成记忆/);
  assert.match(html, /查看记忆结构/);
  assert.match(html, /跨对话回忆/);
  assert.match(html, /按时间回忆/);
  assert.match(html, /事实更新/);
  assert.match(html, /忘记与清除/);
  assert.match(html, /分类、范围、时间、结构、版本、遗忘都可检查/);
  assert.doesNotMatch(html, /采用新信息|保留原记忆|需要你确认/);
  assert.ok(html.includes("个人隔离 · 项目共享"));
  assert.match(html, /记忆档案/);
  assert.match(html, /事件时间与结构化字段均可审计/);
  assert.match(html, /自然输入/);
  assert.doesNotMatch(html, /codex-preview|react-loading-skeleton/i);
});

test("keeps application chrome fixed while only the conversation scrolls", async () => {
  const css = await readFile(new URL("../app/globals.css", import.meta.url), "utf8");
  const page = await readFile(new URL("../app/page.tsx", import.meta.url), "utf8");
  const markdown = await readFile(new URL("../app/markdown.tsx", import.meta.url), "utf8");

  assert.match(css, /\.app-shell\s*\{[^}]*height:100dvh;[^}]*min-height:0;[^}]*overflow:hidden;/s);
  assert.match(css, /\.chat-column\s*\{[^}]*min-height:0;[^}]*overflow:hidden;/s);
  assert.match(css, /\.message-scroller\s*\{[^}]*min-height:0;[^}]*overflow-y:auto;/s);
  assert.match(css, /\.composer-area\s*\{[^}]*flex:0 0 auto;/s);
  assert.match(page, /messageScrollerRef/);
  assert.match(page, /scrollConversationToBottom/);
  assert.match(page, /credentials:\s*"include"/);
  assert.match(page, /withCredentials:\s*true/);
  assert.match(page, /x-demo-identity-revision/);
  assert.match(page, /\/api\/demo\/users\/switch/);
  assert.match(page, /\/api\/demo\/reset-project/);
  assert.match(page, /<MarkdownMessage content=\{item\.content\}/);
  assert.match(page, /effect\.kind === "extraction_summary"/);
  assert.match(page, /本轮形成/);
  assert.match(page, /受限覆盖补齐/);
  assert.match(page, /过滤详情/);
  assert.match(page, /事件时间不合规/);
  assert.match(page, /<EvidenceDrawer turn=\{turn\}/);
  const evidence = await readFile(
    new URL("../components/EvidenceDrawer.tsx", import.meta.url),
    "utf8",
  );
  assert.match(evidence, /turn\.prompt_context/);
  assert.match(evidence, /模型实际收到的上下文快照/);
  assert.match(evidence, /服务器运行时上下文/);
  assert.doesNotMatch(evidence, /memories\.find|currentVersion/);
  assert.match(markdown, /markdown-code-block/);
  assert.match(markdown, /markdown-table-wrap/);
  assert.doesNotMatch(markdown, /dangerouslySetInnerHTML/);
  assert.match(css, /\.markdown-content\s*\{/);
  assert.match(css, /\.markdown-content ul\s*\{\s*list-style:disc outside;/);
  assert.match(css, /\.markdown-content ol\s*\{\s*list-style:decimal outside;/);
  assert.match(css, /\.memory-extraction-note\s*\{/);
});
