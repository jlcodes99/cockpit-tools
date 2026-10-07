// Isolated acceptance using installed Playwright and this repository's installed Vite.
// COCKPIT_PLAYWRIGHT_MODULE=/absolute/path/to/playwright
// PLAYWRIGHT_CHROMIUM_EXECUTABLE=/absolute/path/to/browser node tests/claude-handoff/continuity-flow.spec.cjs
// No dependency installation, account access, user browser profile, or desktop mutation.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { createRequire } = require('node:module');
const { pathToFileURL } = require('node:url');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { chromium } = require(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright');

const repo = path.resolve(__dirname, '../..');
const evidence = path.join(__dirname, 'evidence-continuity', 'success-completion');
fs.mkdirSync(evidence, { recursive: true });
// Playwright's disposable profile and Vite's cache are owned by this test run.
process.env.TMPDIR = path.join(evidence, 'runtime');
fs.mkdirSync(process.env.TMPDIR, { recursive: true });
const report = {
  observedAt: new Date().toISOString(), isolation: 'synthetic IPC; ephemeral browser contexts; localhost only',
  machineVerdict: 'running', visualVerdict: 'pending', independentReview: 'unavailable in this runtime',
  scenarios: [], limitations: [],
  fixtureData: { logicalConversations: 166, activePointers: 172, preservedBranches: 6, syntheticOnly: true },
};
const saveReport = () => fs.writeFileSync(path.join(evidence, 'results.json'), `${JSON.stringify(report, null, 2)}\n`);
const interpolate = (value, params) => value.replace(/{{(\w+)}}/g, (_, key) => String(params[key]));
const stringsFor = lang => JSON.parse(fs.readFileSync(path.join(repo, 'src/locales', `${lang}.json`))).claude.handoff;

function checkLocales() {
  const checkerPath = path.join(repo, 'scripts/check_locales.cjs');
  const checkerRequire = createRequire(checkerPath);
  const check = checkerRequire(checkerPath);
  const locales = fs.readdirSync(path.join(repo, 'src/locales')).filter(file => file.endsWith('.json'));
  const resources = new Map(locales.map(file => [file, JSON.parse(fs.readFileSync(path.join(repo, 'src/locales', file)))]));
  const baseline = resources.get('en-US.json');
  const baselineKeys = [...check.getAllKeys(baseline)].sort();
  const handoffValues = check.getLeafStringMap(baseline.claude.handoff);
  for (const [file, data] of resources) {
    assert.deepEqual([...check.getAllKeys(data)].sort(), baselineKeys, `${file}: complete locale key parity`);
    const values = check.getLeafStringMap(data.claude.handoff);
    for (const [key, reference] of handoffValues) {
      assert.ok(values.get(key)?.trim(), `${file}: nonempty handoff translation ${key}`);
      const tokens = value => (value.match(/{{\w+}}/g) || []).sort();
      assert.deepEqual(tokens(values.get(key)), tokens(reference), `${file}: interpolation tokens ${key}`);
    }
  }
  // Run the repository checker unchanged, redirecting its single generated report
  // into our evidence directory to respect this task's write boundary.
  const log = [];
  const checkerProcess = { env: process.env, exitCode: 0 };
  const sandboxRequire = name => name === 'fs' ? {
    ...fs,
    writeFileSync(target, ...args) {
      assert.equal(target, path.join(repo, 'locale-check-report.md'), 'unexpected locale-check write');
      return fs.writeFileSync(path.join(evidence, 'locale-check-report.md'), ...args);
    },
  } : checkerRequire(name);
  vm.runInNewContext(`${fs.readFileSync(checkerPath, 'utf8')}\nmain();`, {
    require: sandboxRequire, module: { exports: {} }, __dirname: path.dirname(checkerPath),
    process: checkerProcess, console: { log: (...args) => log.push(args.join(' ')) },
  }, { filename: checkerPath });
  fs.writeFileSync(path.join(evidence, 'locale-check.log'), `${log.join('\n')}\n`);
  assert.equal(checkerProcess.exitCode, 0, 'repository locale checker must pass');
  return { files: locales.length, keyParity: 'pass', placeholders: 'pass', repositoryChecker: 'pass' };
}

async function settled(page) {
  await page.locator('.claude-handoff-overlay').evaluate(async overlay => {
    await Promise.all(overlay.getAnimations({ subtree: true }).map(animation => animation.finished.catch(() => {})));
  });
}

async function exercise(browser, base, scenario, lang, viewport, options = {}) {
  const name = `${scenario}-${lang}${options.name ? `-${options.name}` : ''}`;
  const h = stringsFor(lang);
  const verifiedSource = scenario === 'switch-continuity-verified-source';
  const startWarning = scenario === 'switch-continuity-start-failed' ? 'DESKTOP_START_FAILED' : null;
  const errors = [], consoleErrors = [], networkErrors = [], blockedRequests = [], transcript = [];
  const testResult = { scenario, lang, viewport, options, verdict: 'running', errors, consoleErrors, networkErrors, blockedRequests, screenshots: [] };
  report.scenarios.push(testResult);
  let context;
  try {
    context = await browser.newContext({ viewport,
      reducedMotion: options.reducedMotion || 'no-preference',
      ...(report.videoSupported === false ? {} : { recordVideo: { dir: path.join(evidence, 'video'), size: viewport } }),
    });
  } catch (error) {
    if (!/ffmpeg|video/i.test(error.message)) throw error;
    const limitation = `Video unavailable: ${error.message}`;
    if (!report.limitations.includes(limitation)) report.limitations.push(limitation);
    context = await browser.newContext({ viewport, reducedMotion: options.reducedMotion || 'no-preference' });
  }
  const configureContext = async () => {
    await context.route('**/*', route => {
      if (new URL(route.request().url()).origin === new URL(base).origin) return route.continue();
      blockedRequests.push(route.request().url());
      return route.abort();
    });
    await context.tracing.start({ screenshots: true, snapshots: true, sources: true });
  };
  await configureContext();
  let page, video;
  const screenshot = async label => {
    const file = `${name}-${String(testResult.screenshots.length + 1).padStart(2, '0')}-${label}.png`;
    await page.screenshot({ path: path.join(evidence, file), fullPage: true });
    testResult.screenshots.push(file);
  };
  const mark = async action => {
    transcript.push(await page.evaluate(action => ({
      action, at: performance.now(), busy: window.__handoffBusy,
      busyText: document.querySelector('[data-testid="claude-handoff-busy"]')?.textContent || null,
      error: document.querySelector('[data-testid="claude-handoff-error"]')?.textContent || null,
      result: document.querySelector('[data-testid="claude-handoff-result"]')?.textContent || null,
      eventState: window.__handoffEventState(), calls: window.__handoffCalls.length,
    }), action));
  };
  const counts = async () => page.evaluate(() => ({
    preview: window.__handoffCalls.filter(call => call.command === 'claude_handoff_preview').length,
    write: window.__handoffCalls.filter(call => call.command === 'claude_handoff_apply_and_switch').length,
    standaloneWrite: window.__handoffCalls.filter(call => call.command === 'claude_handoff_apply').length,
    switchCalls: window.__switchCalls, completedSwitches: window.__completedSwitches,
  }));
  const assertNoSuccess = async () => {
    assert.equal(await page.locator('[data-testid="claude-handoff-result"]').count(), 0);
    assert.equal(await page.locator('[data-testid="claude-handoff-success-title"]').count(), 0);
    assert.equal(await page.locator('[data-testid="claude-handoff-done"]').count(), 0);
    assert.equal(await page.getByText(h.switchDone, { exact: true }).count(), 0);
    assert.deepEqual((await counts()).completedSwitches, []);
  };
  const clickApply = async () => {
    await page.locator('[data-testid="claude-handoff-apply"]').click();
    // Dispatch rapid input even after disabled paint, exercising the busy lock.
    await page.locator('[data-testid="claude-handoff-apply"]').evaluate(button => {
      button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    await page.locator('[data-testid="claude-handoff-busy"]').waitFor();
    await mark('apply + rapid duplicate clicks');
  };
  const observeStage = async (stage, completed) => {
    const text = h.progress[stage];
    await page.waitForFunction(({ text, completed }) => {
      const busy = document.querySelector('[data-testid="claude-handoff-busy"]');
      return busy && getComputedStyle(busy).visibility !== 'hidden' && busy.textContent.includes(text)
        && (completed === undefined || busy.textContent.includes(`(${completed}/172)`));
    }, { text, completed });
    const status = page.locator('[data-testid="claude-handoff-busy"]');
    await status.scrollIntoViewIfNeeded();
    assert.equal(await status.isVisible(), true);
    assert.equal(await status.getAttribute('role'), 'status');
    assert.equal(await page.locator('[data-testid="claude-handoff-dialog"]').getAttribute('aria-busy'), 'true');
    await assertNoSuccess();
    await mark(`visible stage: ${stage}${completed === undefined ? '' : ` ${completed}/172`}`);
    await screenshot(`${stage}${completed === undefined ? '' : `-${completed}`}`);
  };
  try {
    try {
      page = await context.newPage();
    } catch (error) {
      if (!/ffmpeg/i.test(error.message)) throw error;
      report.videoSupported = false;
      report.limitations.push('Video unavailable: installed Playwright has no ffmpeg binary. Raw screenshots and timed Playwright traces are retained.');
      fs.writeFileSync(path.join(evidence, 'video-unavailable.json'), `${JSON.stringify({ observedAt: new Date().toISOString(), error: error.message }, null, 2)}\n`);
      await context.tracing.stop();
      await context.close();
      context = await browser.newContext({ viewport, reducedMotion: options.reducedMotion || 'no-preference' });
      await configureContext();
      page = await context.newPage();
    }
    video = page.video();
    page.setDefaultTimeout(10000);
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
    page.on('response', response => { if (response.status() >= 400) networkErrors.push({ url: response.url(), status: response.status() }); });
    // Override only synthetic receipts; neither the fixture nor native IPC is modified.
    if (options.receipt) await page.addInitScript(receipt => {
      let internals;
      Object.defineProperty(window, '__TAURI_INTERNALS__', {
        configurable: true,
        get: () => internals,
        set: value => {
          internals = { ...value, invoke: async (command, args) => {
            const outcome = await value.invoke(command, args);
            if (command === 'claude_handoff_apply_and_switch') return {
              ...outcome, ...receipt, run: { ...outcome.run, ...receipt.run },
            };
            if (command === 'claude_handoff_status' && receipt.run) return {
              ...outcome, runs: outcome.runs.map(run => ({ ...run, ...receipt.run })),
            };
            return outcome;
          } };
        },
      });
    }, options.receipt);
    await page.goto(`${base}?scenario=${scenario}&lang=${lang}`);
    await page.evaluate(theme => document.documentElement.setAttribute('data-theme', theme), options.theme || 'light');
    const dialog = page.locator('[data-testid="claude-handoff-dialog"]');
    const apply = page.locator('[data-testid="claude-handoff-apply"]');
    await dialog.waitFor();
    await page.locator('.claude-handoff-status-row').getByText(h.readOnly, { exact: true }).waitFor();
    await page.waitForFunction(() => window.__handoffEventState().listeners === 1);
    await settled(page);
    const preview = page.locator('[data-testid="claude-handoff-preview"]');
    if (verifiedSource) {
      await preview.waitFor();
      const selection = await page.evaluate(() => ({
        accounts: window.__handoffFixtureAccounts,
        previewArgs: window.__handoffCalls.filter(call => call.command === 'claude_handoff_preview').map(call => call.args),
      }));
      const duplicate = selection.accounts[0];
      const desktop = selection.accounts.find(account => account.id === 'account-1');
      assert.equal(duplicate.id, 'account-cli-duplicate');
      assert.equal(duplicate.auth_mode, 'oauth');
      assert.equal(desktop.auth_mode, 'desktop_oauth');
      assert.equal(duplicate.account_uuid, desktop.account_uuid);
      assert.equal(duplicate.organization_uuid, desktop.organization_uuid);
      assert.deepEqual(selection.previewArgs, [{ sourceAccountId: 'account-1', targetAccountId: 'account-2' }],
        'currentIdentity must resolve to the Desktop parent row despite an earlier duplicate CLI row and CLI currentAccountId');
      testResult.verifiedSourceSelection = { duplicateCliId: duplicate.id, selectedDesktopId: desktop.id,
        sharedAccount: desktop.account_uuid, sharedOrg: desktop.organization_uuid, previewArgs: selection.previewArgs };
      assert.match(await page.locator('[data-handoff-account="source"]').innerText(), /account-1@example\.test/,
        'optional synthetic Status identity must select the source even when Cockpit prop points at the target');
      assert.equal(await apply.isEnabled(), true);
      assert.equal((await counts()).preview, 1);
      assert.equal(await page.locator('.claude-handoff-select-menu').count(), 0);
      await mark('initial: mock optional process-identity hint selects the Desktop source and automatically previews');
    } else {
      await mark('initial: source selection required');
      assert.equal(await apply.isDisabled(), true);
      assert.match(await page.locator('[data-handoff-account="source"]').innerText(), new RegExp(h.selectSource));
      assert.equal((await counts()).preview, 0);
    }
    assert.equal(await page.locator('[data-testid="claude-handoff-preview-button"]').count(), 0);
    assert.equal(await page.locator('[data-testid="claude-handoff-busy"]').count(), 0);
    await screenshot('initial');

    if (!verifiedSource) {
      // The listbox portal must be operable from the keyboard, within the dialog.
      await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').focus();
      await page.keyboard.press('Enter');
      await page.locator('.claude-handoff-select-menu [role="option"]').waitFor();
      await page.keyboard.press('Home');
      await page.keyboard.press('Enter');
    }
    await preview.waitFor();
    await settled(page);
    assert.deepEqual(await preview.locator('dd').allTextContents(), ['172', '0', '0']);
    assert.equal(await page.locator('[data-testid="claude-handoff-preserved"]').innerText(), interpolate(h.preservedBranches, { count: 6 }));
    assert.equal(await apply.isEnabled(), true, 'preserved branches must not be treated as unresolved conflicts');
    assert.equal((await counts()).preview, 1);
    await mark(`${verifiedSource ? 'verified source' : 'manual source chosen'}; one automatic preview; six branches preserved`);
    // Reselecting the current source must retain the already loaded preview.
    await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').click();
    await page.locator('.claude-handoff-select-menu [role="option"][aria-selected="true"]').click();
    assert.equal(await preview.isVisible(), true);
    assert.equal(await apply.isEnabled(), true);
    assert.equal((await counts()).preview, 1);
    await screenshot('preview');

    // Events received while idle must not create a success result or a busy state.
    await page.evaluate(() => window.__emitHandoffProgress({ stage: 'complete', completed: 172, total: 172 }));
    await assertNoSuccess();
    assert.equal(await page.locator('[data-testid="claude-handoff-busy"]').count(), 0);
    await clickApply();
    await observeStage('checking');
    // Closing and source editing are locked for the entire mutation.
    assert.equal(await dialog.locator('.modal-close').isDisabled(), true);
    assert.equal(await page.locator('[data-handoff-account="source"] button').isDisabled(), true);
    assert.equal(await page.locator('.claude-handoff-status-row button').isDisabled(), true);
    await page.keyboard.press('Escape');
    assert.equal(await dialog.isVisible(), true);
    await observeStage('stopping');
    await observeStage('hashing');

    if (scenario === 'switch-continuity-retry') {
      await page.locator('[data-testid="claude-handoff-error"]').getByText(h.errors.PREVIEW_CHANGED, { exact: true }).waitFor();
      await page.waitForFunction(() => window.__handoffBusy === false);
      await assertNoSuccess();
      assert.equal(await preview.count(), 0);
      assert.equal(await apply.isDisabled(), true);
      assert.equal((await counts()).preview, 1, 'failure must not silently rerun the preview');
      await mark('first apply rejected; no success or automatic retry');
      await screenshot('error');
      // A late completion event cannot overrule the rejected operation.
      await page.evaluate(() => window.__emitHandoffProgress({ stage: 'complete', completed: 172, total: 172 }));
      await assertNoSuccess();
      await page.locator('.claude-handoff-status-row button').click();
      await preview.waitFor();
      assert.equal((await counts()).preview, 2);
      assert.equal(await page.locator('[data-testid="claude-handoff-error"]').count(), 0);
      await clickApply();
      await observeStage('checking');
      assert.match(await page.locator('[data-testid="claude-handoff-busy"]').innerText(), new RegExp(interpolate(h.elapsed, { seconds: 0 })),
        'elapsed time must restart on an explicit retry');
      await observeStage('stopping');
      await observeStage('hashing');
    }
    await observeStage('backup');
    await observeStage('writing', 0);
    await observeStage('writing', 86);
    const escapeRegex = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const elapsedPattern = new RegExp(h.elapsed.split('{{seconds}}').map(escapeRegex).join('(\\d+)'));
    const busyText = await page.locator('[data-testid="claude-handoff-busy"]').innerText();
    const elapsed = Number(busyText.match(elapsedPattern)?.[1]);
    assert.ok(elapsed >= 2, `real elapsed seconds must advance while work runs: ${busyText}`);
    testResult.elapsedSecondsObserved = elapsed;
    await observeStage('writing', 172);
    await observeStage('verifying');

    if (scenario !== 'switch-continuity-verification-error') {
      await observeStage('switching');
      if (scenario !== 'switch-continuity-uncertain') await observeStage('confirming');
    }
    await page.waitForFunction(() => window.__handoffBusy === false);
    await settled(page);
    assert.equal(await page.locator('[data-testid="claude-handoff-busy"]').count(), 0);
    assert.equal(await dialog.getAttribute('aria-busy'), 'false');
    const actualCounts = await counts();
    assert.equal(actualCounts.preview, scenario === 'switch-continuity-retry' ? 2 : 1);
    assert.equal(actualCounts.write, scenario === 'switch-continuity-retry' ? 2 : 1);
    assert.equal(actualCounts.standaloneWrite, 0);
    assert.deepEqual(actualCounts.switchCalls, [], 'the frontend must not perform a second account switch');
    assert.equal(await page.evaluate(() => window.__handoffCalls.some(call => call.command === 'plugin:dialog|message')), false);
    if (scenario === 'switch-continuity-verification-error') {
      await assertNoSuccess();
      assert.equal(await page.locator('[data-testid="claude-handoff-error"]').innerText(), h.errors.POST_IMAGE_MISMATCH);
      assert.equal(await page.locator('[data-testid="claude-handoff-run"]').getAttribute('data-state'), 'applying');
      assert.equal(await page.locator('[data-testid="claude-handoff-recover"]').isEnabled(), true);
      assert.equal(await page.locator('[data-testid="claude-switch-only"]').isDisabled(), true);
      assert.equal(await apply.isDisabled(), true);
    } else {
      const result = page.locator('[data-testid="claude-handoff-result"]');
      const expectedCounts = { created: 172, updated: 0, ...options.receipt?.run };
      const incomplete = Boolean(options.receipt?.run?.skippedMissing);
      const warningCode = options.receipt?.warning || startWarning
        || (scenario === 'switch-continuity-uncertain' ? 'ACCOUNT_SWITCH_UNCERTAIN' : null);
      if (warningCode || incomplete) {
        assert.equal(await result.isVisible(), true);
        assert.equal(await page.locator('[data-testid="claude-handoff-success-title"]').count(), 0,
          'warning or incomplete receipt must never show a green completion hero');
        assert.equal(await page.locator('[data-testid="claude-handoff-done"]').count(), 0);
        assert.equal(await page.locator('[data-handoff-account="source"] [aria-haspopup="listbox"]').isVisible(), true);
        assert.ok((await result.innerText()).includes(interpolate(h.resultCounts, expectedCounts)));
        const warningText = warningCode ? h.errors[warningCode] : interpolate(h.missingAfterApply, { count: 1 });
        assert.ok((await result.innerText()).includes(warningText));
        assert.equal(await result.getByText(h.switchDone, { exact: true }).count(), 0);
        assert.deepEqual(actualCounts.completedSwitches, options.receipt ? ['account-2'] : []);
        assert.equal(await page.locator('[data-testid="claude-switch-only"]').isEnabled(), true);
        if (startWarning) {
          assert.equal(await page.locator('[data-testid="claude-handoff-run"]').getAttribute('data-state'), 'applied',
            'startup failure must retain the committed data receipt');
          assert.equal(await page.locator('[data-testid="claude-handoff-rollback"]').isEnabled(), true);
          assert.equal(await result.getByText(h.reopened, { exact: true }).count(), 0,
            'failed executable startup must not claim Desktop reopened');
          assert.equal(await page.evaluate(() => window.__handoffEvents.filter(event => event.payload.stage === 'complete').length), 1,
            'only the synthetic idle event exists; confirmation failure must not emit completion');
          await page.locator('.claude-handoff-status-row button').click();
          await page.locator('.claude-handoff-status-row').getByText(h.readOnly, { exact: true }).waitFor();
          assert.ok((await result.innerText()).includes(warningText), 'status refresh must preserve the receipt and startup warning');
          assert.equal((await counts()).preview, 1);
          assert.equal((await counts()).write, 1);
          await mark('startup warning retained after status refresh; no extra preview or write');
        }
      } else {
        const hero = page.locator('.claude-handoff-success');
        const heading = hero.locator('[data-testid="claude-handoff-success-title"]');
        const done = page.locator('[data-testid="claude-handoff-done"]');
        assert.equal(await heading.innerText(), h.successTitle);
        assert.equal(await heading.evaluate(node => node === document.activeElement), true,
          'success must take keyboard focus after the mutation settles');
        assert.equal(await hero.getAttribute('aria-live'), 'polite');
        assert.equal(await hero.getAttribute('aria-atomic'), 'true');
        assert.equal(await hero.getAttribute('role'), 'status');
        assert.equal(await hero.locator('.claude-handoff-success-account strong').innerText(), 'account-2@example.test');
        assert.equal(await hero.locator('.claude-handoff-pair-summary').innerText(), 'account-1@example.test → account-2@example.test');
        assert.ok((await hero.innerText()).includes(interpolate(h.resultCounts, expectedCounts)),
          'hero counts must come from the authoritative receipt, not the preview');
        assert.equal(await hero.getByText(h.switchDone, { exact: true }).isVisible(), true);
        assert.equal(await hero.getByText(h.notReopened, { exact: true }).count(), 0);
        assert.equal(await page.locator('[data-handoff-account], [aria-haspopup="listbox"]').count(), 0,
          'completed accounts must no longer be editable');
        assert.equal(await page.getByText(h.switchDescription, { exact: true }).count(), 0);
        assert.equal(await page.getByText(h.scopeTitle, { exact: true }).count(), 0);
        assert.equal(await apply.count(), 0, 'remove the disabled apply action after success');
        assert.equal(await page.locator('[data-testid="claude-switch-only"]').count(), 0);
        assert.equal(await dialog.locator('.modal-footer button').count(), 1);
        assert.equal(await done.innerText(), h.successDone);
        assert.equal(await done.isEnabled(), true);
        testResult.successGeometry = await hero.evaluate(node => {
          const body = node.closest('.modal-body');
          const rect = node.getBoundingClientRect(), bodyRect = body.getBoundingClientRect();
          const footerRect = document.querySelector('[data-testid="claude-handoff-done"]').getBoundingClientRect();
          return { scrollTop: body.scrollTop, heroTop: rect.top, heroBottom: rect.bottom,
            bodyTop: bodyRect.top, bodyBottom: bodyRect.bottom, doneBottom: footerRect.bottom,
            viewportHeight: innerHeight, firstBodyChild: body.firstElementChild === node };
        });
        const geometry = testResult.successGeometry;
        assert.equal(geometry.scrollTop, 0, 'completion must reset the scrolled preflight immediately');
        assert.equal(geometry.firstBodyChild, true);
        assert.ok(geometry.heroTop >= geometry.bodyTop && geometry.heroBottom <= geometry.bodyBottom,
          'the full success hero must be visible without scrolling');
        assert.ok(geometry.doneBottom <= geometry.viewportHeight, 'Done must be visible without scrolling');
        testResult.successAccessibility = await hero.ariaSnapshot();
        await screenshot('success-prominent');
        const receipt = page.locator('details.claude-handoff-receipt');
        assert.equal(await receipt.getAttribute('open'), null);
        await receipt.locator('summary').click();
        assert.equal(await result.isVisible(), true, 'authoritative receipt remains available');
        assert.ok((await result.innerText()).includes(interpolate(h.resultCounts, expectedCounts)));
        assert.equal(await result.getByText(h.switchDone, { exact: true }).isVisible(), true);
        assert.deepEqual(actualCounts.completedSwitches, ['account-2']);
        assert.equal(await page.locator('[data-testid="claude-handoff-rollback"]').isEnabled(), true,
          'recovery history and its action remain available after completion');
        await screenshot('success-receipt');
        await receipt.locator('summary').click();
        await hero.scrollIntoViewIfNeeded();
      }
    }
    await mark('settled authoritative outcome');
    await screenshot('outcome');
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth), false);
    const dialogDescription = await dialog.getAttribute('aria-describedby');
    const dialogLabel = await dialog.getAttribute('aria-labelledby');
    assert.ok(await page.locator(`[id="${dialogDescription}"]`).innerText());
    assert.ok(await page.locator(`[id="${dialogLabel}"]`).innerText());
    await page.keyboard.press('Tab');
    assert.equal(await dialog.evaluate(node => node.contains(document.activeElement)), true);
    const done = page.locator('[data-testid="claude-handoff-done"]');
    if (await done.count()) {
      await done.focus();
      await page.keyboard.press('Enter');
    } else await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    assert.equal(await page.locator('[data-testid="claude-handoff-open"]').evaluate(node => node === document.activeElement), true,
      'Done or Escape must restore focus to the handoff trigger');
    await page.waitForFunction(() => window.__handoffEventState().listeners === 0 && window.__handoffEventState().callbacks === 0);
    await page.evaluate(() => window.__emitHandoffProgress({ stage: 'writing', completed: 172, total: 172 }));
    assert.equal(await page.evaluate(() => window.__handoffEvents.at(-1).delivered), 0);
    await mark('closed: listener and callback removed; late event not delivered');
    testResult.calls = await counts();
    testResult.events = await page.evaluate(() => window.__handoffEvents);
    assert.ok(testResult.events.filter(event => event.delivered > 0).every(event => event.delivered === 1), 'one active subscriber per event');
    testResult.busyTransitions = await page.evaluate(() => window.__handoffBusyTransitions);
    assert.deepEqual(testResult.busyTransitions.map(item => item.busy), scenario === 'switch-continuity-retry' ? [true, false, true, false] : [true, false]);
    testResult.unlistenCalls = await page.evaluate(() => window.__handoffCalls.filter(call => call.command === 'plugin:event|unlisten').length);
    assert.ok(testResult.unlistenCalls >= 1);
    testResult.performance = await page.evaluate(() => {
      const navigation = performance.getEntriesByType('navigation')[0];
      return { domContentLoadedMs: navigation.domContentLoadedEventEnd, durationMs: performance.now() };
    });
    assert.deepEqual(errors, [], 'no uncaught browser errors');
    assert.deepEqual(consoleErrors, [], 'no browser console errors');
    assert.deepEqual(networkErrors, [], 'no failed local resource responses');
    assert.deepEqual(blockedRequests, [], 'no external requests attempted');
    testResult.verdict = 'pass';
  } catch (error) {
    testResult.verdict = 'fail';
    testResult.failure = error.stack;
    if (page) {
      await screenshot('failure').catch(() => {});
      await mark('failure').catch(() => {});
    }
    throw error;
  } finally {
    fs.writeFileSync(path.join(evidence, `${name}-interactions.json`), `${JSON.stringify(transcript, null, 2)}\n`);
    await context.tracing.stop({ path: path.join(evidence, `${name}-trace.zip`) });
    await context.close();
    if (video) {
      testResult.video = `${name}.webm`;
      await video.saveAs(path.join(evidence, testResult.video));
    }
    saveReport();
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
      server: {
        host: '127.0.0.1', port: 0, strictPort: true, open: false,
        watch: { ignored: ['**/tests/claude-handoff/evidence-continuity/**', '**/src-tauri/**', '**/target/**'] },
      },
    });
    await server.listen();
    const address = server.httpServer.address();
    assert.equal(address.address, '127.0.0.1');
    const base = `http://127.0.0.1:${address.port}/tests/claude-handoff/index.html`;
    report.fixtureUrl = base;
    report.serverOwnership = 'created and closed by this runner; OS-assigned port';
    report.toolchain = {
      node: process.version, playwright: require(path.join(path.dirname(require.resolve(process.env.COCKPIT_PLAYWRIGHT_MODULE || 'playwright')), 'package.json')).version,
      vite: require(path.join(repo, 'node_modules/vite/package.json')).version,
    };
    browser = await chromium.launch({
      headless: true,
      ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}),
    });
    report.toolchain.browser = browser.version();
    const desktop = { width: 1280, height: 900 }, mobile = { width: 390, height: 844 };
    for (const [scenario, lang, viewport, options] of [
      ['switch-continuity', 'zh-tw', desktop],
      ['switch-continuity', 'en', desktop],
      ['switch-continuity', 'zh-CN', mobile],
      ['switch-continuity-retry', 'zh-tw', desktop],
      ['switch-continuity-verification-error', 'zh-tw', desktop],
      ['switch-continuity-uncertain', 'zh-tw', desktop],
      ['switch-continuity-verified-source', 'zh-tw', desktop],
      ['switch-continuity-start-failed', 'zh-tw', desktop],
      ['switch-continuity', 'zh-tw', mobile, { name: 'dark-reduced-motion-receipt', theme: 'dark', reducedMotion: 'reduce',
        receipt: { run: { created: 33, updated: 6 } } }],
      ['switch-continuity', 'en', desktop, { name: 'warning-contradiction',
        receipt: { accountSwitched: true, reopened: true, warning: 'ACCOUNT_SWITCH_IDENTITY_MISMATCH' } }],
      ['switch-continuity', 'zh-tw', desktop, { name: 'incomplete-receipt',
        receipt: { run: { skippedMissing: 1 } } }],
    ]) {
      await exercise(browser, base, scenario, lang, viewport, options);
      process.stdout.write(`PASS ${scenario} ${lang}${options?.name ? ` ${options.name}` : ''}\n`);
    }
    const regression = await promisify(execFile)(process.execPath, [path.join(__dirname, 'switch-flow.spec.cjs')], {
      cwd: repo,
      env: { ...process.env, HANDOFF_FIXTURE_URL: base, HANDOFF_SWITCH_EVIDENCE_DIR: path.join(evidence, 'switch-regression') },
      maxBuffer: 1024 * 1024,
    });
    fs.writeFileSync(path.join(evidence, 'switch-regression.log'), regression.stdout + regression.stderr);
    report.switchRegression = JSON.parse(regression.stdout);
    report.machineVerdict = report.limitations.length ? 'pass-with-warnings' : 'pass';
  } catch (error) {
    report.machineVerdict = report.scenarios.some(item => item.verdict === 'fail') ? 'fail' : 'blocked';
    report.failure = error.stack;
    throw error;
  } finally {
    // Graceful resource disposal affects only objects created by this runner.
    if (browser) await browser.close();
    if (server) await server.close();
    report.serverClosed = !server?.httpServer?.listening;
    report.totals = {
      focusedUiTests: report.scenarios.filter(item => item.verdict === 'pass').length + (report.switchRegression?.length || 0),
      continuityTests: report.scenarios.filter(item => item.verdict === 'pass').length,
      switchTests: report.switchRegression?.length || 0,
      screenshots: report.scenarios.reduce((sum, item) => sum + item.screenshots.length, 0)
        + (report.switchRegression || []).reduce((sum, item) => sum + item.screenshots.length, 0),
      videos: report.scenarios.filter(item => item.video).length,
      traces: report.scenarios.length,
    };
    saveReport();
  }
  process.stdout.write(`Evidence: ${evidence}\n`);
}

run().catch(error => { console.error(error); process.exitCode = 1; });
