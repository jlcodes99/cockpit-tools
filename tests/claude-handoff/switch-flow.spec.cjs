// Isolated browser acceptance for the switch-time handoff UI.
// Start Vite with: npm run dev -- --host 127.0.0.1 --port 1489 --strictPort
// Run with NODE_PATH pointing to a local Playwright installation.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright');

const base = process.env.HANDOFF_FIXTURE_URL || 'http://127.0.0.1:1489/tests/claude-handoff/index.html';
const evidence = process.env.HANDOFF_SWITCH_EVIDENCE_DIR || path.join(__dirname, 'evidence-switch-flow');
fs.mkdirSync(evidence, { recursive: true });

async function waitForSettledDialog(page) {
  await page.locator('.claude-handoff-overlay').evaluate(async overlay => {
    await Promise.all(overlay.getAnimations({ subtree: true }).map(animation => animation.finished.catch(() => {})));
  });
}

async function run() {
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}),
  });
  const results = [];
  try {
    for (const scenario of ['switch', 'switch-conflict', 'switch-missing', 'switch-unsupported', 'switch-no-source', 'switch-after-copy-fail', 'switch-race', 'switch-rejected', 'switch-pending']) {
      const mobile = scenario === 'switch-no-source';
      const page = await browser.newPage({ viewport: mobile ? { width: 390, height: 844 } : { width: 1280, height: 900 } });
      const pageErrors = [];
      const screenshots = [];
      const screenshot = async name => {
        await page.screenshot({ path: path.join(evidence, name), fullPage: true });
        screenshots.push(name);
      };
      page.on('pageerror', error => pageErrors.push(error.message));
      await page.goto(`${base}?scenario=${scenario}&lang=zh-tw`);
      const dialog = page.locator('[data-testid="claude-handoff-dialog"]');
      await dialog.waitFor();
      await page.locator('.claude-handoff-status-row button').waitFor();
      await page.waitForFunction(() => window.__handoffCalls.some(call => call.command === 'claude_handoff_status'));
      await page.locator('.claude-handoff-status-row').getByText(/預覽為唯讀/).waitFor();
      await waitForSettledDialog(page);
      assert.match(await dialog.locator('h2').innerText(), /切換 Claude Desktop 帳號/);
      const previewButton = page.locator('[data-testid="claude-handoff-preview-button"]');
      const copyButton = page.locator('[data-testid="claude-handoff-apply"]');
      assert.equal(await previewButton.count(), 0, 'switching should not require a separate preview click');
      assert.equal(await copyButton.isDisabled(), true, 'copy must wait for an automatic preview');
      assert.match(await page.locator('[data-handoff-account="source"]').innerText(), /選擇來源帳號/,
        'the current account marker must not select a source for the user');
      assert.equal(await page.evaluate(() => window.__handoffCalls.filter(call => call.command === 'claude_handoff_preview').length), 0,
        'a manual source selection must precede every switch preview');
      await screenshot(`${scenario}-initial.png`);

      if (scenario === 'switch' || scenario === 'switch-after-copy-fail' || scenario === 'switch-rejected') {
        await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
        await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-1@example.test' }).click();
        await page.locator('[data-testid="claude-handoff-preview"]').waitFor();
        await waitForSettledDialog(page);
        assert.match(await dialog.innerText(), /新增 1 項|將新增/);
        if (scenario === 'switch') await screenshot('switch-preview.png');
        assert.equal(await copyButton.isEnabled(), true);
        await copyButton.click();
        await page.waitForFunction(() => window.__handoffCalls.some(call => call.command === 'claude_handoff_apply_and_switch') && window.__handoffBusy === false);
        await waitForSettledDialog(page);
        const calls = await page.evaluate(() => window.__handoffCalls);
        const apply = calls.find(call => call.command === 'claude_handoff_apply_and_switch');
        assert.equal(calls.filter(call => call.command === 'claude_handoff_preview').length, 1,
          'selecting one source should request one automatic preview');
        assert.equal(calls.some(call => call.command === 'claude_handoff_apply'), false);
        assert.equal(calls.some(call => call.command === 'plugin:dialog|message'), false, 'primary switch action is the final confirmation');
        assert.deepEqual(await page.evaluate(() => window.__switchCalls), [], 'frontend must not perform a second switch');
        assert.equal(apply.args.sourceAccountId, 'account-1');
        assert.equal(apply.args.targetAccountId, 'account-2');
        if (scenario === 'switch-rejected') {
          await page.locator('[data-testid="claude-handoff-error"]').getByText(/Claude Desktop 無法正常結束/).waitFor();
          assert.equal(calls.filter(call => call.command === 'claude_handoff_preview').length, 1,
            'failed apply must remain visible rather than silently rerun preview');
          assert.equal(await page.locator('[data-testid="claude-handoff-preview"]').count(), 0);
          assert.equal(await copyButton.isDisabled(), true);
          await screenshot('switch-rejected-error.png');
          await page.locator('.claude-handoff-status-row button').click();
          await page.waitForFunction(() => window.__handoffCalls.filter(call => call.command === 'claude_handoff_preview').length === 2);
          await page.locator('[data-testid="claude-handoff-preview"]').waitFor();
          assert.equal(await copyButton.isEnabled(), true);
        } else if (scenario === 'switch') {
          assert.match(await dialog.innerText(), /已載入目標帳號的登入資料並啟動 Claude Desktop/);
        } else {
          assert.match(await dialog.innerText(), /對話已交接，但目標帳號登入狀態未確認/);
          assert.equal(await page.locator('[data-testid="claude-switch-only"]').isEnabled(), true);
          assert.equal(await page.locator('[data-testid="claude-handoff-rollback"]').count(), 1);
        }
        await screenshot(`${scenario}-done.png`);
      } else if (scenario === 'switch-conflict') {
        await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
        await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-1@example.test' }).click();
        await page.locator('[data-testid="claude-handoff-preview"]').waitFor();
        await waitForSettledDialog(page);
        assert.match(await page.locator('[data-testid="claude-handoff-missing"]').innerText(), /1 個項目尚無法完整交接/);
        assert.match(await page.locator('[data-testid="claude-handoff-stale"]').innerText(), /1 筆對話使用不同的目前分支/);
        assert.match(await page.locator('[data-testid="claude-handoff-replaced"]').innerText(), /有 2 筆對話在來源與目標使用不同分支/);
        assert.equal(await copyButton.isDisabled(), true);
        assert.equal(await page.evaluate(() => window.__handoffCalls.some(call => call.command === 'claude_handoff_apply_and_switch')), false);
        await screenshot('switch-conflict-blocked.png');
      } else if (scenario === 'switch-missing') {
        await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
        await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-1@example.test' }).click();
        await page.locator('[data-testid="claude-handoff-preview"]').waitFor();
        assert.match(await page.locator('[data-testid="claude-handoff-missing-blocked"]').innerText(), /交接已停用/);
        assert.equal(await copyButton.isDisabled(), true);
        assert.equal(await page.evaluate(() => window.__handoffCalls.some(call => call.command === 'claude_handoff_apply_and_switch')), false);
        await screenshot('switch-missing-blocked.png');
      } else if (scenario === 'switch-race') {
        await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
        await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-1@example.test' }).click();
        await page.waitForFunction(() => window.__handoffCalls.some(call => call.command === 'claude_handoff_preview' && call.args.sourceAccountId === 'account-1'));
        await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
        await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-3@example.test' }).click();
        await page.waitForFunction(() => window.__handoffCalls.some(call => call.command === 'claude_handoff_preview' && call.args.sourceAccountId === 'account-3'));
        const preview = page.locator('[data-testid="claude-handoff-preview"]');
        await preview.waitFor();
        assert.match(await preview.innerText(), /account-3@example\.test → account-2@example\.test/);
        assert.equal(await preview.locator('dd').first().innerText(), '7');
        await page.waitForTimeout(900);
        assert.equal(await preview.locator('dd').first().innerText(), '7', 'old preview must not replace a newer selection');
        assert.equal(await copyButton.isEnabled(), true);
        await screenshot('switch-race-preview.png');
      } else if (scenario === 'switch-pending') {
        await page.locator('[data-testid="claude-handoff-recover"]').waitFor();
        assert.match(await page.locator('[data-testid="claude-handoff-run"]').innerText(), /交接期間偵測到 Claude 程序仍在執行/);
        assert.equal(await page.locator('[data-testid="claude-switch-only"]').isDisabled(), true,
          'a direct switch must not consume partially copied records');
        assert.equal(await copyButton.isDisabled(), true);
        assert.equal(await page.evaluate(() => window.__switchCalls.length), 0);
      } else if (scenario === 'switch-unsupported') {
        assert.match(await dialog.innerText(), /尚未通過交接審查/);
        await page.locator('[data-testid="claude-switch-only"]').click();
        await dialog.waitFor({ state: 'detached' });
        assert.deepEqual(await page.evaluate(() => window.__switchCalls), ['account-2']);
      } else {
        assert.match(await dialog.innerText(), /請親自選擇 Claude Desktop 目前實際使用的來源帳號/);
        assert.equal(await page.locator('[data-testid="claude-switch-only"]').isEnabled(), true);
      }

      const overflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
      assert.equal(overflow, false, 'dialog must not cause horizontal viewport overflow');
      assert.deepEqual(pageErrors, [], 'no uncaught browser errors');
      results.push({ scenario, viewport: mobile ? '390x844' : '1280x900', overflow, pageErrors, screenshots });
      await page.close();
    }
  } finally {
    await browser.close();
  }
  fs.writeFileSync(path.join(evidence, 'results.json'), `${JSON.stringify(results, null, 2)}\n`);
  process.stdout.write(`${JSON.stringify(results, null, 2)}\n`);
}

run().catch(error => { console.error(error); process.exitCode = 1; });
