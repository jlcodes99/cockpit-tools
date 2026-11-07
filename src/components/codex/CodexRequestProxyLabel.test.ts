import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createInstance } from "i18next";
import { CodexRequestProxyLabel } from "./CodexRequestProxyLabel";
import type { CodexLocalAccessProxyRoute } from "../../types/codexLocalAccess";
import zhCN from "../../locales/zh-CN.json";
import en from "../../locales/en.json";

async function render(route?: CodexLocalAccessProxyRoute | null, lng = "zh-CN") {
  const i18n = createInstance();
  await i18n.init({
    lng,
    resources: { "zh-CN": { translation: zhCN }, en: { translation: en } },
    interpolation: { escapeValue: false },
  });
  return renderToStaticMarkup(createElement(CodexRequestProxyLabel, { route, t: i18n.t }));
}

test("named nodes retain the full recorded name in the title and escape markup", async () => {
  const name = 'Tokyo <01> & "出口" ' + "Long node name ".repeat(20);
  const output = await render({ kind: "node", name });
  const escaped = name.trim().replace(/&/g, "&amp;").replace(/</g, "&lt;")
    .replace(/>/g, "&gt;").replace(/"/g, "&quot;");
  assert.ok(output.includes(`title="代理：${escaped}"`));
  assert.ok(output.includes(`>${"代理：" + escaped}</span>`));
  assert.ok(output.includes('class="codex-request-proxy-label"'));
});

test("direct connections ignore stale names and translate the route", async () => {
  assert.equal(await render({ kind: "direct", name: "old proxy" }, "en"),
    '<span class="codex-request-proxy-label" title="Proxy: Direct">Proxy: Direct</span>');
});

test("missing, unknown and incomplete snapshots do not imply direct connections", async () => {
  for (const route of [undefined, null, { kind: "unknown", name: "stale" },
    { kind: "node", name: "   " }, { kind: "proxy", name: "" }] as const) {
    const output = await render(route);
    assert.ok(output.includes(">代理：未记录</span>"));
    assert.ok(!output.includes("stale"));
    assert.ok(!output.includes("直连"));
  }
});

test("proxy entries explain the missing final node only in the tooltip", async () => {
  const output = await render({ kind: "proxy", name: "127.0.0.1:7890" });
  assert.ok(output.includes("代理：127.0.0.1:7890\n仅记录代理入口，最终节点未记录。"));
  assert.ok(output.endsWith(">代理：127.0.0.1:7890</span>"));
  const node = await render({ kind: "node", name: "Singapore" });
  assert.ok(!node.includes("最终节点未记录"));
});
