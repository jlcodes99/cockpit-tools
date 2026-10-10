// Synthetic IPC only. Uses existing installed Vite/Playwright and a disposable browser profile.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { chromium } = require(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright');
const { getAllKeys, getLeafStringMap } = require('../../scripts/check_locales.cjs');
const repo = path.resolve(__dirname, '../..');
const evidence = process.env.HANDOFF_UPDATER_EVIDENCE_DIR || path.join(__dirname, 'evidence-updater-flow');
fs.mkdirSync(path.join(evidence, 'runtime'), { recursive: true });
process.env.TMPDIR = path.join(evidence, 'runtime');
const report = { observedAt: new Date().toISOString(), isolation: 'synthetic IPC; localhost only; disposable profiles',
  machineVerdict: 'running', visualVerdict: 'pending', scenarios: [] };
const save = () => fs.writeFileSync(path.join(evidence, 'results.json'), JSON.stringify(report, null, 2) + '\n');
const resource = lang => JSON.parse(fs.readFileSync(path.join(repo, 'src/locales', `${lang}.json`)));
function checkLocales() {
  const files = fs.readdirSync(path.join(repo, 'src/locales')).filter(file => file.endsWith('.json'));
  assert.equal(files.length, 18);
  const base = resource('en-US'), keys = [...getAllKeys(base)].sort();
  const strings = getLeafStringMap(base.claude.handoff);
  for (const file of files) {
    const data = resource(file.slice(0, -5));
    assert.deepEqual([...getAllKeys(data)].sort(), keys, `${file}: complete key parity`);
    const actual = getLeafStringMap(data.claude.handoff);
    for (const [key, value] of strings) {
      assert.ok(actual.get(key)?.trim(), `${file}: ${key}`);
      const tokens = text => (text.match(/{{[^{}]+}}/g) || []).sort();
      assert.deepEqual(tokens(actual.get(key)), tokens(value), `${file}: placeholders ${key}`);
    }
    assert.notEqual(data.claude.handoff.errors.DESKTOP_UPDATE_TIMEOUT, data.claude.handoff.errors.UNKNOWN);
    if (!file.startsWith('en')) {
      assert.notEqual(data.claude.handoff.progress.updating, base.claude.handoff.progress.updating);
      assert.notEqual(data.claude.handoff.errors.DESKTOP_UPDATE_TIMEOUT, base.claude.handoff.errors.DESKTOP_UPDATE_TIMEOUT);
    }
  }
  return { files: files.length, keyParity: 'pass', handoffPlaceholders: 'pass', translatedUpdaterMessages: 'pass' };
}
async function exercise(browser, base, scenario, lang, viewport) {
  const item = { scenario, lang, verdict: 'running', screenshots: [], errors: [] };
  report.scenarios.push(item);
  const h = resource(lang).claude.handoff;
  const context = await browser.newContext({ viewport, reducedMotion: 'reduce' });
  await context.tracing.start({ screenshots: true, snapshots: true, sources: false });
  try {
    await context.route('**/*', route => {
      assert.equal(new URL(route.request().url()).origin, new URL(base).origin, 'localhost only');
      return route.continue();
    });
    const page = await context.newPage();
    page.on('pageerror', error => item.errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') item.errors.push(message.text()); });
    page.on('requestfailed', request => item.errors.push(request.failure()?.errorText));
    const screenshot = async label => {
      const file = `${scenario}-${lang}-${label}.png`;
      await page.screenshot({ path: path.join(evidence, file), fullPage: true });
      item.screenshots.push(file);
    };
    await page.goto(`${base}?scenario=${scenario}&lang=${lang}`);
    await page.locator('.claude-handoff-status-row').getByText(h.readOnly, { exact: true }).waitFor();
    await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
    await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-1@example.test' }).click();
    await page.locator('[data-testid="claude-handoff-preview"]').waitFor();
    await page.locator('[data-testid="claude-handoff-apply"]').click();
    const busy = page.locator('[data-testid="claude-handoff-busy"]');
    await page.waitForFunction(() => window.__handoffEvents.some(event => event.payload.stage === 'updating'));
    assert.ok((await busy.innerText()).includes(h.progress.updating));
    assert.equal(await page.locator('[data-testid="claude-handoff-apply"]').isDisabled(), true);
    assert.deepEqual(await page.evaluate(() => window.__completedSwitches), []);
    await screenshot('updating');
    if (scenario !== 'switch-updater-resume') {
      const notice = page.locator('[data-testid="claude-handoff-error"]');
      await notice.waitFor();
      await page.waitForFunction(() => window.__handoffBusy === false);
      const timeout = scenario === 'switch-updater-timeout';
      const late = scenario === 'switch-updater-late';
      assert.equal(await notice.innerText(), h.errors[late ? 'DESKTOP_CONTRACT_CHANGED' : timeout ? 'DESKTOP_UPDATE_TIMEOUT' : 'CLAUDE_WRITER_RUNNING']);
      assert.equal(await notice.getAttribute('role'), 'alert');
      const detail = await page.locator('[data-testid="claude-handoff-process-detail"]').innerText();
      assert.ok(detail.includes(late ? '4444' : timeout ? '4242' : '4343') && detail.includes(timeout || late ? 'ShipIt' : 'Claude Helper')
        && detail.includes(h.processRoles[timeout || late ? 'desktop-updater' : 'desktop-helper']));
      assert.equal(await page.locator('[data-testid="claude-handoff-success-title"], [data-testid="claude-handoff-result"], [data-testid="claude-handoff-run"]').count(), 0);
      assert.equal(await page.locator('[data-testid="claude-handoff-apply"]').isDisabled(), true);
      assert.deepEqual(await page.evaluate(() => window.__completedSwitches), []);
      assert.equal(await page.evaluate(() => window.__handoffEvents.some(event => ['writing', 'switching', 'complete'].includes(event.payload.stage))), false);
      await screenshot('timeout');
    } else {
      await page.waitForFunction(() => window.__handoffEvents.some(event => event.payload.stage === 'checking'));
      await page.locator('[data-testid="claude-handoff-success-title"]').waitFor();
      await page.waitForFunction(() => window.__handoffBusy === false);
      assert.deepEqual(await page.evaluate(() => window.__completedSwitches), ['account-2']);
      const stages = await page.evaluate(() => window.__handoffEvents.map(event => event.payload.stage));
      assert.deepEqual(stages, ['updating', 'checking', 'hashing', 'backup', 'writing', 'verifying', 'switching', 'confirming', 'complete']);
      await screenshot('success');
    }
    assert.deepEqual(await page.evaluate(() => window.__switchCalls), [], 'frontend must not initiate a separate switch');
    item.calls = await page.evaluate(() => window.__handoffCalls);
    assert.equal(item.calls.filter(call => call.command === 'claude_handoff_apply_and_switch').length, 1);
    assert.equal(item.calls.filter(call => call.command === 'claude_handoff_preview').length, 1);
    item.events = await page.evaluate(() => window.__handoffEvents);
    assert.ok(item.events.every(event => event.delivered === 1), 'one progress listener');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    assert.deepEqual(item.errors, []);
    item.verdict = 'pass';
  } catch (error) { item.verdict = 'fail'; item.failure = error.stack; throw error; }
  finally {
    await context.tracing.stop({ path: path.join(evidence, `${scenario}-${lang}-trace.zip`) });
    await context.close();
    save();
  }
}
async function run() {
  let server, browser;
  try {
    report.locales = checkLocales();
    const { createServer } = await import(pathToFileURL(require.resolve('vite')));
    server = await createServer({ root: repo, configFile: path.join(repo, 'vite.config.ts'), configLoader: 'runner',
      cacheDir: path.join(evidence, 'runtime/node_modules/.vite'),
      server: { host: '127.0.0.1', port: 0, strictPort: true, open: false,
        watch: { ignored: ['**/tests/claude-handoff/evidence-*/**', '**/src-tauri/**', '**/target/**'] } } });
    await server.listen();
    const address = server.httpServer.address();
    assert.equal(address.address, '127.0.0.1');
    const base = `http://127.0.0.1:${address.port}/tests/claude-handoff/index.html`;
    browser = await chromium.launch({ headless: true,
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}) });
    report.toolchain = { node: process.version, browser: browser.version() };
    const scenarios = process.env.HANDOFF_UPDATER_SCENARIOS?.split(',')
      || ['switch-updater-resume', 'switch-updater-timeout', 'switch-updater-writer-blocked', 'switch-updater-late'];
    for (const scenario of scenarios) {
      for (const lang of ['zh-tw', 'en']) await exercise(browser, base, scenario, lang,
        lang === 'zh-tw' ? { width: 390, height: 844 } : { width: 1280, height: 900 });
    }
    report.machineVerdict = 'pass';
  } catch (error) { report.machineVerdict = 'fail'; report.failure = error.stack; throw error; }
  finally { await browser?.close(); await server?.close(); report.completedAt = new Date().toISOString(); save(); }
  console.log(JSON.stringify({ verdict: report.machineVerdict, scenarios: report.scenarios.length, locales: report.locales.files, evidence }, null, 2));
}
run().catch(error => { console.error(error); process.exitCode = 1; });
