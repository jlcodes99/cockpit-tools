// Run against npm run dev on localhost. Uses an installed Playwright module;
// COCKPIT_PLAYWRIGHT_MODULE can select a shared toolchain without changing app dependencies.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright');
const base = process.env.COCKPIT_HANDOFF_TEST_URL || 'http://127.0.0.1:1438';
const output = path.resolve(process.env.COCKPIT_HANDOFF_EVIDENCE || '.tmp/claude-handoff-ui');
await fs.mkdir(output, { recursive: true });
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const results = [], pageErrors = [];
let page;
async function fresh(scenario = 'normal', lang = 'en') {
  if (page) await page.close();
  page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.route('**/*', route => {
    const u = new URL(route.request().url());
    return u.origin === new URL(base).origin ? route.continue() : route.abort();
  });
  await page.goto(`${base}/tests/claude-handoff/?scenario=${scenario}&lang=${lang}`);
  await page.getByTestId('claude-handoff-open').click();
  await page.locator('.claude-handoff-overlay').evaluate(async overlay => {
    await Promise.all(overlay.getAnimations({ subtree: true }).map(animation => animation.finished.catch(() => {})));
  });
  const target = page.locator('[data-handoff-account="target"] button');
  if (scenario.startsWith('privacy') || scenario.startsWith('saved-index') || scenario === 'empty-parent') await page.locator('[data-handoff-account="target"] button:not([disabled])').waitFor();
  else await target.filter({ hasText: 'account-2@example.test' }).waitFor();
}
async function source(n = 1) {
  await page.locator('[data-handoff-account="source"] button').click();
  await page.getByRole('option').filter({ hasText: `account-${n}@example.test` }).click();
}
async function preview() {
  await page.getByTestId('claude-handoff-preview-button').click();
  await page.getByTestId('claude-handoff-preview').waitFor();
}
async function count(command) {
  return page.evaluate(command => window.__handoffCalls.filter(c => c.command === command).length, command);
}
async function record(name, fn) { await fn(); results.push({ name, passed: true }); }
async function capture(name) { await page.screenshot({ path: path.join(output, `${name}.png`), fullPage: true }); }
try {
  await fresh('saved-index');
  await record('Saved index supplies choices with an empty parent list and never implies the current target', async () => {
    await page.getByTestId('claude-handoff-saved-index').waitFor();
    const target = page.locator('[data-handoff-account="target"] button');
    assert(!(await target.innerText()).includes('account-2@example.test'));
    assert((await page.getByTestId('claude-handoff-dialog').innerText()).includes('Select the target manually'));
    await source();
    assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
    await target.click();
    await page.getByRole('option').first().waitFor();
    assert.equal(await page.getByRole('option').count(), 2);
    await page.getByRole('option').filter({ hasText: 'account-2@example.test' }).click();
    await preview();
    assert.equal(await count('claude_handoff_apply'), 0);
    const calls = await page.evaluate(() => window.__handoffCalls.filter(c => c.command === 'claude_handoff_preview'));
    assert.deepEqual(calls[0].args, { sourceAccountId: 'account-1', targetAccountId: 'account-2' });
  });
  await capture('saved-index-desktop');
  await page.setViewportSize({ width: 390, height: 844 });
  await record('Saved index instructions fit a narrow viewport', async () => {
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const box = await page.getByTestId('claude-handoff-dialog').boundingBox();
    assert(box && box.x >= 0 && box.x + box.width <= 390);
  });
  await capture('saved-index-mobile');
  for (const scenario of ['saved-index', 'saved-index-privacy']) {
    await fresh(scenario);
    await page.locator('[data-handoff-account="source"] button').click();
    await page.getByRole('option').first().click();
    await page.locator('[data-handoff-account="target"] button').click();
    await page.getByRole('option').nth(1).click();
    const sourceLabel = await page.locator('[data-handoff-account="source"] .single-select-dropdown-value').innerText();
    const targetLabel = await page.locator('[data-handoff-account="target"] .single-select-dropdown-value').innerText();
    const expectedPair = `${sourceLabel} → ${targetLabel}`;
    await preview();
    await page.getByTestId('claude-handoff-apply').click();
    await page.getByTestId('claude-handoff-result').waitFor();
    await page.waitForFunction(() => !window.__handoffBusy);
    await record(`Saved index resolves result, persisted history and rollback confirmation identities (${scenario})`, async () => {
      const result = await page.getByTestId('claude-handoff-result').innerText();
      assert(result.includes(expectedPair));
      if (scenario.endsWith('privacy')) {
        assert(!result.includes('account-1@example.test') && !result.includes('account-2@example.test'));
      }
      await page.keyboard.press('Escape');
      await page.getByTestId('claude-handoff-open').click();
      await page.getByTestId('claude-handoff-run').waitFor();
      const history = await page.getByTestId('claude-handoff-run').innerText();
      assert(history.includes(expectedPair));
      await page.getByTestId('claude-handoff-rollback').click();
      await page.locator('[data-testid="claude-handoff-run"][data-state="rolled_back"]').waitFor();
      const confirmation = await page.evaluate(() => window.__handoffCalls.filter(c => c.command === 'plugin:dialog|message').at(-1).args);
      assert(confirmation.message.startsWith(expectedPair));
      if (scenario.endsWith('privacy')) {
        assert(!JSON.stringify({ history, confirmation }).includes('@example.test'));
      }
      assert((await page.getByTestId('claude-handoff-result').innerText()).includes(expectedPair));
    });
  }
  await fresh('saved-index-pending');
  await record('Saved index resolves an existing interrupted run in recovery confirmation', async () => {
    await page.getByTestId('claude-handoff-recover').click();
    await page.locator('[data-testid="claude-handoff-run"][data-state="rolled_back"]').waitFor();
    const confirmation = await page.evaluate(() => window.__handoffCalls.filter(c => c.command === 'plugin:dialog|message').at(-1).args);
    assert(confirmation.message.startsWith('account-1@example.test → account-2@example.test'));
  });
  await fresh('empty-parent');
  await record('Normal mode still filters against the parent account list', async () => {
    await page.locator('[data-handoff-account="source"] button').click();
    await page.getByRole('listbox').waitFor();
    assert.equal(await page.getByRole('option').count(), 0);
    assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
  });
  await fresh('saved-index-unavailable', 'zh-tw');
  await record('Unavailable saved index shows a translated sanitized error and blocks preview', async () => {
    await page.getByRole('alert').waitFor();
    const text = await page.getByRole('alert').innerText();
    assert(text.includes('無法讀取原版 Cockpit'));
    assert(!text.includes('/private') && !text.includes('account-private') && !text.includes('ACCOUNT_INDEX_UNAVAILABLE'));
    assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
  });
  await fresh('saved-index-running', 'zh-tw');
  await source();
  await page.locator('[data-handoff-account="target"] button').click();
  await page.getByRole('option').filter({ hasText: 'account-2@example.test' }).click();
  await preview();
  await page.getByTestId('claude-handoff-apply').click();
  await page.waitForFunction(() => !window.__handoffBusy && window.__handoffCalls.some(c => c.command === 'claude_handoff_apply'));
  await record('Original Cockpit running error clears approval and supports a new read-only preview', async () => {
    const text = await page.getByTestId('claude-handoff-error').innerText();
    assert(text.includes('套用前請先結束原版 Cockpit'));
    assert(!text.includes('/private') && !text.includes('account-private') && !text.includes('EXTERNAL_COCKPIT_RUNNING'));
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
    assert.equal(await page.getByTestId('claude-handoff-preview').count(), 0);
    assert.equal(await page.getByTestId('claude-handoff-result').count(), 0);
    assert.equal(await count('claude_handoff_apply'), 1);
    await preview();
    assert.equal(await page.getByTestId('claude-handoff-error').count(), 0);
  });
  await capture('saved-index-zh-tw');
  await fresh();
  await record('No mutation or preview without a distinct explicit source', async () => {
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
    assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
    assert.equal(await count('claude_handoff_apply'), 0);
    assert(await page.evaluate(() => !!document.activeElement?.closest('[role="dialog"]')));
  });
  await source(); await preview();
  await record('Read-only preview shows the confirmed pair and counts', async () => {
    assert.equal(await count('claude_handoff_apply'), 0);
    assert((await page.getByTestId('claude-handoff-preview').innerText()).includes('account-1@example.test'));
    assert(!(await page.getByTestId('claude-handoff-apply').isDisabled()));
  });
  await capture('preview-light');
  await fresh('conflict'); await source(); await preview();
  await record('Conflicting conversations are identifiable but cannot be applied', async () => {
    await page.getByTestId('claude-handoff-preview').locator('summary').click();
    assert((await page.getByTestId('claude-handoff-preview').innerText()).includes('Synthetic planning conversation'));
    await page.getByTestId('claude-handoff-preview').locator('summary').click();
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
  });
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await capture('preview-dark');
  await record('Desktop dialog has no horizontal overflow', async () => {
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const box = await page.getByTestId('claude-handoff-dialog').boundingBox();
    assert(box && box.x >= 0 && box.x + box.width <= 1280);
  });
  await fresh(); await source(); await preview();
  await source(2);
  await record('Changing the pair invalidates the old plan', async () => {
    assert.equal(await page.getByTestId('claude-handoff-preview').count(), 0);
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
  });
  await source(); await preview();
  await page.evaluate(() => { window.__handoffConfirm = false; });
  await page.getByTestId('claude-handoff-apply').click();
  await page.waitForFunction(() => !window.__handoffBusy);
  await record('Cancelling confirmation does not mutate', async () => assert.equal(await count('claude_handoff_apply'), 0));
  await page.evaluate(() => { window.__handoffConfirm = true; });
  await page.getByTestId('claude-handoff-apply').dblclick();
  await page.keyboard.press('Escape');
  await record('Busy operation cannot be dismissed or submitted twice', async () => {
    assert.equal(await page.getByTestId('claude-handoff-dialog').count(), 1);
    await page.getByTestId('claude-handoff-result').waitFor();
    assert.equal(await count('claude_handoff_apply'), 1);
  });
  await capture('applied');
  await page.getByTestId('claude-handoff-rollback').click();
  await page.waitForFunction(() => window.__handoffCalls.some(c => c.command === 'claude_handoff_rollback'));
  await record('Recovery uses the run ID and keeps Desktop stopped', async () => {
    await page.locator('[data-testid="claude-handoff-run"][data-state="rolled_back"]').waitFor();
    assert.equal(await count('claude_handoff_rollback'), 1);
    assert.equal(await page.getByTestId('claude-handoff-rollback').count(), 0);
  });
  await page.keyboard.press('Escape');
  await record('Escape restores focus to the toolbar trigger', async () => {
    assert.equal(await page.getByTestId('claude-handoff-dialog').count(), 0);
    assert(await page.getByTestId('claude-handoff-open').evaluate(el => el === document.activeElement));
  });

  await fresh('stale'); await source(); await preview();
  await page.getByTestId('claude-handoff-apply').click();
  await page.waitForFunction(() => !window.__handoffBusy && window.__handoffCalls.some(c => c.command === 'claude_handoff_apply'));
  await record('Stale snapshot removes apply capability and allows a new preview', async () => {
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
    assert.equal(await page.getByTestId('claude-handoff-preview').count(), 0);
    assert(!(await page.getByTestId('claude-handoff-preview-button').isDisabled()));
    assert((await page.getByRole('alert').allTextContents()).some(t => /preview/i.test(t)));
  });
  await fresh('slow-preview'); await source();
  await page.getByTestId('claude-handoff-preview-button').click(); await source(2);
  await page.waitForTimeout(1000);
  await record('Late async preview cannot restore a plan for an old pair', async () => {
    assert.equal(await page.getByTestId('claude-handoff-preview').count(), 0);
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
  });
  await fresh('refresh-error'); await source(); await preview();
  await page.getByTestId('claude-handoff-apply').click();
  await page.getByTestId('claude-handoff-result').waitFor();
  await page.waitForFunction(() => !window.__handoffBusy);
  await record('A status refresh failure does not erase committed success', async () => {
    assert.equal(await count('claude_handoff_apply'), 1);
    assert.equal(await page.getByTestId('claude-handoff-result').count(), 1);
    assert(await page.getByRole('alert').count() > 0);
  });
  await fresh('unsupported'); await source();
  await record('Unknown Desktop version prevents new handoff', async () => {
    assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
    assert(await page.getByTestId('claude-handoff-apply').isDisabled());
  });
  await capture('unsupported-version');
  for (const scenario of ['partial-rollback', 'partial-rollback-refresh-error']) {
    await fresh(scenario); await source(); await preview();
    await page.getByTestId('claude-handoff-apply').click();
    await page.getByTestId('claude-handoff-result').waitFor();
    await page.waitForFunction(() => !window.__handoffBusy);
    await page.getByTestId('claude-handoff-rollback').click();
    await page.waitForFunction(() => !window.__handoffBusy && window.__handoffCalls.some(c => c.command === 'claude_handoff_rollback'));
    await record(`Partial recovery cannot retain stale success (${scenario})`, async () => {
      assert.equal(await page.getByTestId('claude-handoff-result').count(), 0);
      assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
      assert.equal(await page.locator('[data-testid="claude-handoff-run"][data-state="applied"]').count(), 0);
      if (scenario === 'partial-rollback') {
        assert.equal(await page.locator('[data-testid="claude-handoff-run"][data-state="rolling_back"]').count(), 1);
        await page.getByTestId('claude-handoff-recover').click();
        await page.locator('[data-testid="claude-handoff-run"][data-state="rolled_back"]').waitFor();
      }
    });
  }
  await fresh('pending-unsupported'); await source();
  await record('Pending transaction blocks new work but remains recoverable after a version update', async () => {
    assert(await page.getByTestId('claude-handoff-preview-button').isDisabled());
    assert(!(await page.getByTestId('claude-handoff-recover').isDisabled()));
    await page.getByTestId('claude-handoff-recover').click();
    await page.locator('[data-testid="claude-handoff-run"][data-state="rolled_back"]').waitFor();
  });
  await fresh('normal', 'zh-tw'); await source(); await preview();
  await record('Traditional Chinese copy is loaded', async () => {
    const text = await page.getByTestId('claude-handoff-dialog').innerText();
    assert(!text.includes('claude.handoff.')); assert(/交接/.test(text));
  });
  await capture('preview-zh-tw');
  await fresh('privacy-conflict');
  await page.locator('[data-handoff-account="source"] button').click();
  await page.getByRole('option').first().click();
  await preview();
  await page.getByTestId('claude-handoff-preview').locator('summary').click();
  await record('Privacy mode masks account emails and omits conversation titles from the dialog', async () => {
    const text = await page.getByTestId('claude-handoff-dialog').textContent();
    assert(!text.includes('Synthetic planning conversation'));
    assert(!text.includes('account-1@example.test'));
    assert(!text.includes('account-2@example.test'));
    assert(!text.includes('claude.handoff.'));
  });
  await capture('preview-privacy');
  assert.deepEqual(pageErrors, []);
  await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ status: 'passed', scope: 'Actual React component with isolated synthetic Tauri IPC; not a real Desktop account test', results, pageErrors }, null, 2));
  console.log(JSON.stringify({ status: 'passed', checks: results.length, output }));
} catch (error) {
  if (page) await capture('failure').catch(() => {});
  await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ status: 'failed', results, error: String(error), pageErrors }, null, 2));
  throw error;
} finally { await browser.close(); }
