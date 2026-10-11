// Isolated storage-contract acceptance. Requires existing Vite and Playwright;
// COCKPIT_PLAYWRIGHT_MODULE and PLAYWRIGHT_CHROMIUM_EXECUTABLE may select installed tools.
// HANDOFF_CONTRACT_SCENARIOS selects comma-separated scenario names for focused reruns;
// HANDOFF_CONTRACT_EVIDENCE_DIR keeps their receipts separate from the full acceptance.
// All IPC is synthetic; the browser uses a disposable profile and localhost only.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { getAllKeys, getLeafStringMap } = require('../../scripts/check_locales.cjs');
const { chromium } = require(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright');

const repo = path.resolve(__dirname, '../..');
const evidence = process.env.HANDOFF_CONTRACT_EVIDENCE_DIR || path.join(__dirname, 'evidence-storage-contract');
fs.mkdirSync(path.join(evidence, 'runtime'), { recursive: true });
process.env.TMPDIR = path.join(evidence, 'runtime');
const report = {
  observedAt: new Date().toISOString(),
  isolation: 'synthetic IPC; disposable browser profiles; localhost only; no live applications or accounts',
  machineVerdict: 'running', visualVerdict: 'pending', independentReview: 'unavailable in this runtime',
  scenarios: [], limitations: [],
};
const stringsFor = lang => JSON.parse(fs.readFileSync(path.join(repo, 'src/locales', `${lang}.json`))).claude.handoff;
const save = () => fs.writeFileSync(path.join(evidence, 'results.json'), `${JSON.stringify(report, null, 2)}\n`);

function checkLocales() {
  const codes = ['DESKTOP_CONTRACT_UNAVAILABLE', 'DESKTOP_CONTRACT_UNSUPPORTED', 'DESKTOP_CONTRACT_CHANGED', 'UNSUPPORTED_PERSISTED_FIELD'];
  const files = fs.readdirSync(path.join(repo, 'src/locales')).filter(file => file.endsWith('.json'));
  assert.equal(files.length, 18);
  const baselineResource = JSON.parse(fs.readFileSync(path.join(repo, 'src/locales/en-US.json')));
  const baseline = baselineResource.claude.handoff;
  const baselineKeys = [...getAllKeys(baselineResource)].sort();
  const baselineHandoffValues = getLeafStringMap(baseline);
  const placeholders = value => (value.match(/{{[^{}]+}}/g) || []).sort();
  const obsoleteVersionScope = /Unreviewed Desktop versions|لم تُراجع|Neprověřené verze|Ungeprüfte Desktop-Versionen|versiones de Desktop no revisadas|versions Desktop non vérifiées|Versi Desktop yang belum ditinjau|versioni Desktop non verificate|未検証の Desktop バージョン|검토되지 않은 Desktop 버전|Niezweryfikowane wersje Desktop|Versões do Desktop não verificadas|Непроверенные версии Desktop|İncelenmemiş Desktop sürümleri|phiên bản Desktop chưa được kiểm tra|未审核.*版本|未審查.*版本/i;
  for (const file of files) {
    const lang = path.basename(file, '.json');
    const resource = JSON.parse(fs.readFileSync(path.join(repo, 'src/locales', file)));
    const h = resource.claude.handoff;
    assert.deepEqual([...getAllKeys(resource)].sort(), baselineKeys, `${lang}: complete locale key parity`);
    const values = getLeafStringMap(h);
    for (const [key, reference] of baselineHandoffValues) {
      assert.ok(values.get(key)?.trim(), `${lang}: nonempty handoff translation ${key}`);
      assert.deepEqual(placeholders(values.get(key)), placeholders(reference), `${lang}: handoff placeholders ${key}`);
    }
    assert.doesNotMatch(h.scope, obsoleteVersionScope, `${lang}: scope must not qualify by unreviewed version`);
    for (const required of ['macOS', 'Claude Desktop', 'Code', 'Chat', 'Cowork']) {
      assert.ok(h.scope.includes(required), `${lang}: preserve scope ${required}`);
    }
    assert.deepEqual(Object.keys(h.errors).sort(), Object.keys(baseline.errors).sort(), `${lang}: handoff error key parity`);
    for (const code of codes) {
      assert.ok(h.errors[code]?.trim(), `${lang}: translated ${code}`);
      assert.notEqual(h.errors[code], h.errors.UNKNOWN, `${lang}: no generic fallback`);
      assert.deepEqual(h.errors[code].match(/{{\w+}}/g) || [], [], `${lang}: no unresolved interpolation`);
      if (!lang.startsWith('en')) assert.notEqual(h.errors[code], baseline.errors[code], `${lang}: no English fallback`);
    }
    assert.ok(!/verified by Cockpit|Cockpit 已驗證|Cockpit 已验证/.test(h.errors.DESKTOP_VERSION_REQUIRES_REVIEW),
      `${lang}: legacy compatibility notice must not recommend a pinned Desktop version`);
  }
  return { files: files.length, translatedCodes: codes, errorKeyParity: 'pass',
    completeLocaleKeyParity: 'pass', keysPerLocale: baselineKeys.length,
    handoffPlaceholderParity: 'pass', handoffStringsPerLocale: baselineHandoffValues.size,
    storageBasedScope: 'pass' };
}

async function settled(page) {
  await page.locator('.claude-handoff-overlay').evaluate(async overlay => {
    await Promise.all(overlay.getAnimations({ subtree: true }).map(animation => animation.finished.catch(() => {})));
  });
}

async function exercise(browser, base, scenario, lang, viewport) {
  const name = `${scenario}-${lang}`;
  const h = stringsFor(lang);
  const errors = [], consoleErrors = [], networkErrors = [], blockedRequests = [], interactions = [];
  const result = { scenario, lang, viewport, verdict: 'running', errors, consoleErrors, networkErrors, blockedRequests, screenshots: [] };
  report.scenarios.push(result);
  const context = await browser.newContext({ viewport, reducedMotion: 'reduce' });
  await context.tracing.start({ screenshots: true, snapshots: true, sources: false });
  try {
    await context.route('**/*', route => {
      const url = new URL(route.request().url());
      if (url.origin === new URL(base).origin || ['data:', 'blob:'].includes(url.protocol)) return route.continue();
      blockedRequests.push(route.request().url());
      return route.abort();
    });
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
    page.on('requestfailed', request => networkErrors.push({ url: request.url(), error: request.failure()?.errorText }));
    const mark = async label => interactions.push({ label, at: new Date().toISOString(), state: await page.evaluate(() => ({
      time: performance.now(), busy: window.__handoffBusy, calls: window.__handoffCalls,
    })) });
    const screenshot = async label => {
      await settled(page);
      const file = `${name}-${label}.png`;
      await page.screenshot({ path: path.join(evidence, file), fullPage: true });
      result.screenshots.push(file);
    };
    const counts = () => page.evaluate(() => ({
      preview: window.__handoffCalls.filter(call => call.command === 'claude_handoff_preview').length,
      apply: window.__handoffCalls.filter(call => call.command === 'claude_handoff_apply_and_switch').length,
      rollback: window.__handoffCalls.filter(call => call.command === 'claude_handoff_rollback').length,
      standaloneApply: window.__handoffCalls.filter(call => call.command === 'claude_handoff_apply').length,
      directSwitch: window.__switchCalls.length,
    }));
    await page.goto(`${base}?scenario=${scenario}&lang=${lang}`);
    const dialog = page.locator('[data-testid="claude-handoff-dialog"]');
    await dialog.waitFor();
    await page.locator('.claude-handoff-status-row').getByText(h.readOnly, { exact: true }).waitFor();
    await settled(page);
    await dialog.locator('details').first().locator('summary').click();
    await dialog.getByText(h.scope, { exact: true }).waitFor();
    await dialog.locator('details').first().locator('summary').click();
    for (const code of ['UNSUPPORTED_VERSION', 'DESKTOP_VERSION_UNAVAILABLE', 'DESKTOP_VERSION_CHANGED', 'DESKTOP_VERSION_REQUIRES_REVIEW']) {
      assert.equal(await dialog.getByText(h.errors[code], { exact: true }).count(), 0,
        'new backend storage-contract states must not display legacy version qualification errors');
    }
    const apply = page.locator('[data-testid="claude-handoff-apply"]');
    const preview = page.locator('[data-testid="claude-handoff-preview"]');
    const unsupported = scenario.includes('-unsupported-') || scenario.includes('-unavailable-') || scenario === 'switch-contract-changed-pending';
    const defaultProfile = scenario.includes('-default-profile-');
    const selectSource = async () => {
      await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
      await page.locator('.claude-handoff-select-menu [role="option"]').filter({ hasText: 'account-1@example.test' }).click();
    };

    if (unsupported || defaultProfile) {
      const code = defaultProfile ? 'DEFAULT_PROFILE_REQUIRED'
        : scenario === 'switch-contract-changed-pending' ? 'DESKTOP_CONTRACT_CHANGED'
          : scenario.includes('-unsupported-') ? 'DESKTOP_CONTRACT_UNSUPPORTED' : 'DESKTOP_CONTRACT_UNAVAILABLE';
      await dialog.getByText(h.errors[code], { exact: true }).waitFor();
      await selectSource();
      assert.equal(await apply.isDisabled(), true, 'unsupported status must block apply');
      assert.equal(await preview.count(), 0);
      assert.equal((await counts()).preview, 0, 'blocked storage must not request a preview');
      const recovery = page.locator(`[data-testid="claude-handoff-${scenario.endsWith('-applied') ? 'rollback' : 'recover'}"]`);
      await recovery.waitFor();
      assert.equal(await recovery.isEnabled(), !defaultProfile, 'default-profile guard must still protect recovery');
      await screenshot('blocked-with-recovery');
      await mark('new handoff blocked; saved-preimage recovery eligibility observed');
      if (!defaultProfile) {
        await recovery.click();
        await page.locator('[data-testid="claude-handoff-run"][data-state="rolled_back"]').waitFor();
        await page.waitForFunction(() => window.__handoffBusy === false);
        assert.equal((await counts()).rollback, 1);
        assert.equal(await recovery.count(), 0);
        await mark('synthetic recovery completed');
        await screenshot('recovered');
      }
      assert.equal((await counts()).apply, 0);
    } else if (scenario === 'switch-contract-native-state') {
      await selectSource();
      await preview.waitFor();
      assert.equal(await apply.isDisabled(), true, 'unsupported per-row native state must block apply');
      await preview.locator('.claude-handoff-details summary').click();
      await preview.getByText(h.errors.UNSUPPORTED_PERSISTED_FIELD, { exact: true }).waitFor();
      assert.equal(await preview.getByText(h.errors.SESSION_SKIPPED, { exact: true }).count(), 0, 'render the specific row issue');
      assert.equal((await counts()).preview, 1);
      assert.equal((await counts()).apply, 0);
      await mark('specific unsupported native-state issue rendered; apply blocked');
      await screenshot('native-state-blocked');
    } else {
      await selectSource();
      await preview.waitFor();
      assert.equal(await apply.isEnabled(), true, 'supported storage must ignore diagnostic version');
      if (scenario === 'switch-contract-version-unavailable') {
        result.legacyVersionAlert = await dialog.getByText(h.errors.UNSUPPORTED_VERSION, { exact: true }).count();
        assert.equal(result.legacyVersionAlert, 0, 'missing diagnostic version must not render a version qualification alert');
      }
      await mark('supported preview ready');
      await screenshot('eligible-preview');
      await apply.click();
      if (scenario === 'switch-contract-changed') {
        const notice = page.locator('[data-testid="claude-handoff-error"]');
        await notice.waitFor();
        await page.waitForFunction(() => window.__handoffBusy === false);
        assert.equal(await notice.innerText(), h.errors.DESKTOP_CONTRACT_CHANGED);
        assert.equal(await notice.getAttribute('role'), 'alert');
        assert.ok(!(await dialog.innerText()).includes('/private/synthetic/storage'));
        assert.ok(!(await dialog.innerText()).includes('account-private'));
        assert.equal(await preview.count(), 0, 'stale preview must be discarded');
        assert.equal(await apply.isDisabled(), true);
        assert.equal(await page.locator('[data-testid="claude-handoff-success-title"]').count(), 0);
        assert.equal((await counts()).preview, 1, 'failed apply must wait for explicit refresh');
        await mark('in-flight contract change requires refresh; stale preview removed');
        await screenshot('refresh-required');
        await page.locator('.claude-handoff-status-row button').click();
        await preview.waitFor();
        assert.equal((await counts()).preview, 2);
        assert.equal(await notice.count(), 0);
        assert.equal(await apply.isEnabled(), true);
        await mark('explicit refresh obtained a new preview');
        await apply.click();
      }
      await page.locator('[data-testid="claude-handoff-success-title"]').waitFor();
      await page.waitForFunction(() => window.__handoffBusy === false);
      const calls = await page.evaluate(() => window.__handoffCalls);
      const inputs = calls.filter(call => call.command === 'claude_handoff_apply_and_switch').map(call => call.args);
      const input = inputs[0];
      assert.equal(input.fingerprint, 'synthetic-approval-1', 'relay opaque approval independently of the synthetic plan');
      if (scenario === 'switch-contract-changed') {
        assert.equal(inputs[1].fingerprint, 'synthetic-approval-2', 'explicit refresh must use the new approval nonce');
      }
      assert.equal(Object.hasOwn(input, 'desktopVersion'), true, 'preserve IPC desktopVersion field');
      assert.equal(input.desktopVersion, scenario === 'switch-contract-version-unavailable' ? ''
        : scenario === 'switch-contract-future' ? '999.0.future' : '2.110.0');
      assert.equal((await counts()).apply, scenario === 'switch-contract-changed' ? 2 : 1);
      await mark('synthetic apply completed with diagnostic IPC version preserved');
      await screenshot('applied');
    }

    assert.equal((await counts()).standaloneApply, 0);
    assert.equal((await counts()).directSwitch, 0);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth), false,
      'no horizontal viewport overflow');
    const label = await dialog.getAttribute('aria-labelledby');
    const description = await dialog.getAttribute('aria-describedby');
    assert.ok(await page.locator(`[id="${label}"]`).innerText());
    assert.ok(await page.locator(`[id="${description}"]`).innerText());
    await page.keyboard.press('Tab');
    assert.equal(await dialog.evaluate(node => node.contains(document.activeElement)), true, 'keyboard focus remains inside dialog');
    result.accessibility = await dialog.ariaSnapshot();
    result.counts = await counts();
    assert.deepEqual(errors, []);
    assert.deepEqual(consoleErrors, []);
    assert.deepEqual(networkErrors, []);
    assert.deepEqual(blockedRequests, []);
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    result.verdict = result.legacyVersionAlert ? 'pass-with-warnings' : 'pass';
  } catch (error) {
    result.verdict = 'fail';
    result.failure = error.stack || String(error);
    throw error;
  } finally {
    fs.writeFileSync(path.join(evidence, `${name}-interactions.json`), `${JSON.stringify(interactions, null, 2)}\n`);
    await context.tracing.stop({ path: path.join(evidence, `${name}-trace.zip`) });
    await context.close();
    save();
  }
}

async function run() {
  let server, browser;
  try {
    report.locales = checkLocales();
    const { createServer } = await import(pathToFileURL(require.resolve('vite')));
    server = await createServer({
      root: repo, configFile: path.join(repo, 'vite.config.ts'), configLoader: 'runner',
      cacheDir: path.join(evidence, 'runtime/node_modules/.vite'),
      server: { host: '127.0.0.1', port: 0, strictPort: true, open: false,
        watch: { ignored: ['**/tests/claude-handoff/evidence-*/**', '**/src-tauri/**', '**/target/**'] } },
    });
    await server.listen();
    const address = server.httpServer.address();
    assert.equal(address.address, '127.0.0.1');
    const base = `http://127.0.0.1:${address.port}/tests/claude-handoff/index.html`;
    report.fixtureUrl = base;
    browser = await chromium.launch({ headless: true,
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}) });
    report.toolchain = { node: process.version, browser: browser.version(),
      playwright: require(path.join(path.dirname(require.resolve(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright')), 'package.json')).version,
      vite: require(path.join(repo, 'node_modules/vite/package.json')).version };
    const desktop = { width: 1280, height: 900 }, mobile = { width: 390, height: 844 };
    const cases = [
      ['switch-contract-future', 'en', desktop],
      ['switch-contract-version-unavailable', 'zh-tw', desktop],
      ['switch-contract-unavailable-pending', 'en', desktop],
      ['switch-contract-unavailable-applied', 'zh-tw', mobile],
      ['switch-contract-unsupported-pending', 'zh-tw', desktop],
      ['switch-contract-unsupported-applied', 'en', desktop],
      ['switch-contract-changed-pending', 'zh-tw', desktop],
      ['switch-contract-default-profile-pending', 'en', desktop],
      ['switch-contract-native-state', 'zh-tw', mobile],
      ['switch-contract-changed', 'en', desktop],
      ['switch-contract-changed', 'zh-tw', mobile],
      ['switch-contract-changed', 'ar', mobile],
    ];
    const selected = process.env.HANDOFF_CONTRACT_SCENARIOS?.split(',').map(value => value.trim()).filter(Boolean);
    if (selected) {
      for (const name of selected) assert.ok(cases.some(([scenario]) => scenario === name), `unknown scenario ${name}`);
    }
    report.selection = selected || 'all';
    for (const [scenario, lang, viewport] of cases) {
      if (!selected || selected.includes(scenario)) await exercise(browser, base, scenario, lang, viewport);
    }
    report.machineVerdict = report.limitations.length ? 'pass-with-warnings' : 'pass';
  } catch (error) {
    report.machineVerdict = 'fail';
    report.failure = error.stack || String(error);
    throw error;
  } finally {
    await browser?.close();
    await server?.close();
    report.completedAt = new Date().toISOString();
    save();
  }
  process.stdout.write(`${JSON.stringify({ machineVerdict: report.machineVerdict,
    scenarios: report.scenarios.length, locales: report.locales.files, limitations: report.limitations, evidence }, null, 2)}\n`);
}

run().catch(error => { console.error(error); process.exitCode = 1; });
