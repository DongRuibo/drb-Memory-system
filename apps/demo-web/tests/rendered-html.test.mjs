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
  assert.match(html, /一条自然演示路线/);
  assert.match(html, /自然认识你/);
  assert.match(html, /跨对话使用/);
  assert.match(html, /自动更新事实/);
  assert.match(html, /自然忘记/);
  assert.match(html, /稳定信息自动记，事实变化自动更新并提示/);
  assert.doesNotMatch(html, /采用新信息|保留原记忆|需要你确认/);
  assert.ok(html.includes("个人记忆 + 项目记忆"));
  assert.match(html, /它记住了什么/);
  assert.match(html, /和普通 AI 一样输入消息/);
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
  assert.match(page, /<MarkdownMessage content=\{item\.content\}/);
  assert.match(markdown, /markdown-code-block/);
  assert.match(markdown, /markdown-table-wrap/);
  assert.doesNotMatch(markdown, /dangerouslySetInnerHTML/);
  assert.match(css, /\.markdown-content\s*\{/);
  assert.match(css, /\.markdown-content ul\s*\{\s*list-style:disc outside;/);
  assert.match(css, /\.markdown-content ol\s*\{\s*list-style:decimal outside;/);
});
