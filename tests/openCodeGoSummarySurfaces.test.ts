import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

async function source(path: string): Promise<string> {
  return readFile(new URL(`../${path}`, import.meta.url), 'utf8');
}

test('dashboard renders OpenCode Go from the isolated connection store', async () => {
  const dashboard = await source('src/pages/DashboardPage.tsx');
  const goCard = dashboard
    .split("if (platformId === 'opencode_go')")[1]
    ?.split("if (!isAccountPlatform(platformId))")[0];

  assert.ok(dashboard.includes("useOpenCodeGoAccountStore"));
  assert.ok(dashboard.includes("buildOpenCodeGoSummary(openCodeGoAccounts)"));
  assert.ok(goCard?.includes('openCodeGoSummary.connection'));
  assert.ok(!goCard?.includes('codexCurrentAccount'));
  assert.ok(!goCard?.includes('codexApiUsageMap'));
});

test('floating card renders OpenCode Go without adapting a Codex account', async () => {
  const floatingCard = await source('src/pages/FloatingCardWindow.tsx');
  const goCard = floatingCard
    .split("selectedPlatform === 'opencode_go' ? (")[1]
    ?.split(') : viewedAccount && presentation ? (')[0];

  assert.ok(floatingCard.includes("useOpenCodeGoAccountStore"));
  assert.ok(floatingCard.includes("await fetchOpenCodeGoAccounts()"));
  assert.ok(goCard?.includes('openCodeGoQuotaItems'));
  assert.ok(!goCard?.includes('buildCodexAccountPresentation'));
});
