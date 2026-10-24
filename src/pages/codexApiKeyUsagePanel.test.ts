import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createInstance } from "i18next";
import type { CodexAccount } from "../types/codex";
import type { ModelProviderUsageSummary } from "../services/modelProviderUsageService";
import { useCodexApiKeyUsageFormatting } from "./codexApiKeyUsageFormatting";
import { renderCodexApiKeyUsagePanel } from "./codexApiKeyUsagePanel";

const i18n = createInstance();
await i18n.init({ lng: "en", resources: {}, initImmediate: false });

const account: CodexAccount = {
  id: "quota-test",
  email: "test@example.com",
  auth_mode: "apikey",
  openai_api_key: "synthetic-test-key",
  api_base_url: "https://example.com/v1",
  tokens: { id_token: "", access_token: "", refresh_token: "" },
  created_at: 0,
  last_used: 0,
};

function renderQuota(partial: Partial<ModelProviderUsageSummary>): string {
  const summary: ModelProviderUsageSummary = {
    mode: "new_api",
    modelStatsCount: 0,
    latencyMs: 0,
    ...partial,
  };
  function Panel() {
    const formatting = useCodexApiKeyUsageFormatting(i18n.t, String);
    return renderCodexApiKeyUsagePanel({
      account,
      provider: null,
      usageState: { loading: false, summary },
      t: i18n.t,
      ...formatting,
    });
  }
  return renderToStaticMarkup(createElement(Panel));
}

test("New API card bar matches the remaining percentage printed next to it", () => {
  const html = renderQuota({
    quotaLimit: 100,
    quotaUsed: 6,
    quotaRemaining: 94,
    unit: "%",
  });
  assert.match(html, /94% \/ 100%/);
  assert.match(html, /style="width:94%"/);
});

test("New API card uses the same allocation details for its text and bar", () => {
  const html = renderQuota({
    quotaLimit: 100,
    quotaUsed: 6,
    quotaRemaining: 94,
    unit: "CNY",
    details: [
      { key: "totalGranted", label: "Granted", value: "200" },
      { key: "totalAvailable", label: "Available", value: "50" },
    ],
  });
  assert.match(html, /¥50\.00 \/ ¥200/);
  assert.match(html, /style="width:25%"/);
});

for (const [name, quota, width] of [
  ["unused", { quotaLimit: 100, quotaUsed: 0, quotaRemaining: 100 }, 100],
  ["exhausted", { quotaLimit: 100, quotaUsed: 100, quotaRemaining: 0 }, 0],
  ["derived remaining", { quotaLimit: 100, quotaUsed: 75 }, 25],
  ["over total", { quotaLimit: 100, quotaRemaining: 120 }, 100],
  ["negative remaining", { quotaLimit: 100, quotaRemaining: -5 }, 0],
  ["zero total", { quotaLimit: 0, quotaRemaining: 94 }, 0],
  ["missing total", { quotaRemaining: 94 }, 0],
  ["missing remaining", { quotaLimit: 100 }, 0],
  ["unlimited", { quotaUnlimited: true }, 100],
] as const) {
  test(`New API card handles ${name} quota without inventing remaining capacity`, () => {
    assert.match(renderQuota(quota), new RegExp(`style="width:${width}%"`));
  });
}
