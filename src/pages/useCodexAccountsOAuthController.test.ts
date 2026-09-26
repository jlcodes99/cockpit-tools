import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';

// Run the controller's actual transition handlers and effect predicates. The
// unrelated account/provider UI is excluded; IPC completion order is explicit.
const source = readFileSync(new URL('./useCodexAccountsOAuthController.ts', import.meta.url), 'utf8');
function harness(overrides: Record<string, unknown> = {}) {
  const starts: { args: unknown[]; task: ReturnType<typeof deferred<any>> }[] = [];
  const cancellations: { id: string; task: ReturnType<typeof deferred<void>> }[] = [];
  const c: Record<string, any> = {
    oauthProxyEnabled: false, oauthProxyReady: false, oauthProxyInput: '', oauthProxyUsesAccountExit: false,
    oauthProxyExitLabel: null, oauthProxyFieldError: null, oauthPrepareError: null, oauthTimeoutInfo: null,
    oauthCallbackError: null, deviceAuthInfo: null, deviceAuthError: null, deviceAuthStarting: false,
    oauthMethod: 'browser', oauthUrl: 'old-url', oauthSessionRevision: 0, oauthPortInUse: null,
    oauthCallbackInput: '', oauthCallbackSubmitting: false, oauthTokenExchangeRetryVisible: false,
    oauthProxyDefaultAccountId: null, oauthUrlCopied: false, reauthProxyDefault: false,
    reauthTargetAccountId: '', showAddModal: true, addTab: 'oauth',
    oauthAttemptSeqRef: { current: 1 }, oauthActiveRef: { current: true }, oauthLoginIdRef: { current: 'old' },
    oauthCompletingRef: { current: false }, showAddModalRef: { current: true }, addTabRef: { current: 'oauth' },
    oauthProxyChangeSequence: { current: 0 }, oauthProxyChanging: { current: false },
    oauthStartTask: { current: null }, oauthCancelTask: { current: null }, oauthCancelLoginId: { current: null },
    t: (key: string) => key, oauthLog() {}, useCallback: (callback: unknown) => callback,
    useEffect: (effect: () => void) => { c.effect = effect; },
    oauthStartProxyArgs: (choice: any) => ({ proxyUrl: choice.enabled ? choice.input : null, reauthAccountId: null }),
    oauthProxyUseAfterStart: () => null,
    handleOauthPrepareError: () => { c.oauthActiveRef.current = false; c.oauthPrepareError = 'startFailed'; },
    codexService: {
      cancelCodexOAuthLogin(id: string) { const task = deferred<void>(); cancellations.push({ id, task }); return task.promise; },
      startCodexOAuthLogin(...args: unknown[]) { const task = deferred<any>(); starts.push({ args, task }); return task.promise; },
    },
    ...overrides,
  };
  for (const field of ['oauthProxyReady', 'oauthProxyEnabled', 'oauthProxyFieldError', 'oauthProxyInput',
    'oauthProxyUsesAccountExit', 'oauthProxyExitLabel', 'oauthUrl', 'oauthPrepareError', 'oauthTimeoutInfo',
    'oauthCallbackError', 'deviceAuthInfo', 'deviceAuthError', 'deviceAuthStarting', 'oauthMethod',
    'oauthPortInUse', 'oauthCallbackInput', 'oauthCallbackSubmitting', 'oauthTokenExchangeRetryVisible',
    'oauthSessionRevision', 'oauthUrlCopied', 'deviceCodeCopied', 'oauthProxyDefaultAccountId']) {
    c[`set${field[0].toUpperCase()}${field.slice(1)}`] = (next: any) => { c[field] = typeof next === 'function' ? next(c[field]) : next; };
  }
  vm.createContext(c);
  const evaluate = (text: string) => vm.runInContext(ts.transpileModule(text, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, c);
  const start = source.indexOf('    const cancelOauthSession =');
  const end = source.indexOf('    useEffect(() => {\n      if (!reauthProxyDefault)', start);
  assert.ok(start > 0 && end > start);
  evaluate(source.slice(start, end) + '\nglobalThis.toggle = handleOauthProxyToggle; globalThis.edit = handleOauthProxyInputChange; globalThis.startProxy = handleOauthProxyStart;');
  const autoStart = source.indexOf('    useEffect(() => {\n      if (\n        !showAddModal', end);
  const closeStart = source.indexOf('    useEffect(() => {\n      if (showAddModal && addTab === "oauth") return;', autoStart);
  evaluate(source.slice(autoStart, closeStart)); const auto = c.effect as () => void;
  const closeEnd = source.indexOf('    useEffect(\n      () => () => {', closeStart);
  evaluate(source.slice(closeStart, closeEnd)); const close = c.effect as () => void;
  return { state: c, starts, cancellations, auto, close };
}

test('enabling proxy blocks automatic direct starts until cancellation and explicit setup finish', async () => {
  const h = harness(); const switching = h.state.toggle(true);
  h.auto(); assert.equal(h.starts.length, 0);
  h.cancellations[0].task.resolve(); await switching; await settlePromises();
  h.auto(); assert.equal(h.starts.length, 0);
  h.state.edit('http://chosen:8080'); await h.state.startProxy(); h.auto();
  assert.equal(h.starts.length, 1); assert.equal(h.starts[0].args[0], 'http://chosen:8080');
  h.starts[0].task.resolve({ loginId: 'proxy', authUrl: 'proxy-url' }); await settlePromises();
  assert.equal(h.state.oauthUrl, 'proxy-url');
});

test('disabling proxy and rapid toggle changes share cancellation and only the latest mode wins', async () => {
  const h = harness({ oauthProxyEnabled: true, oauthProxyReady: true, oauthProxyInput: 'http://old' });
  const off = h.state.toggle(false); const on = h.state.toggle(true); const last = h.state.toggle(false);
  h.auto(); assert.equal(h.starts.length, 0); assert.equal(h.cancellations.length, 1);
  h.cancellations[0].task.resolve(); await Promise.all([off, on, last]); await settlePromises();
  assert.equal(h.state.oauthProxyEnabled, false); h.auto();
  assert.equal(h.starts.length, 1); assert.equal(h.starts[0].args[0], undefined);
});

test('failed cancellation keeps the old identity and supports retry without starting a new login', async () => {
  const h = harness(); const first = h.state.toggle(true);
  h.cancellations[0].task.reject(new Error('cancel failed')); await first; await settlePromises();
  h.auto(); assert.equal(h.starts.length, 0); assert.equal(h.state.oauthPrepareError, 'codex.oauthProxy.cancelFailed');
  const retry = h.state.toggle(true); assert.equal(h.cancellations[1].id, 'old');
  h.cancellations[1].task.resolve(); await retry; await settlePromises();
  assert.equal(h.state.oauthCancelLoginId.current, null); assert.equal(h.state.oauthProxyEnabled, true);
});

test('closing and reopening while cancellation is pending cannot apply the old toggle', async () => {
  const h = harness(); const switching = h.state.toggle(true);
  h.state.showAddModal = false; h.state.showAddModalRef.current = false; h.close();
  h.state.showAddModal = true; h.state.showAddModalRef.current = true; h.auto();
  assert.equal(h.starts.length, 0);
  h.cancellations[0].task.resolve(); await switching; await settlePromises();
  assert.equal(h.state.oauthProxyEnabled, false); h.auto(); assert.equal(h.starts.length, 1);
});

test('changing proxy while start is pending waits for the stale session to be cancelled', async () => {
  const h = harness({ oauthUrl: null, oauthActiveRef: { current: false }, oauthLoginIdRef: { current: null } });
  h.auto(); assert.equal(h.starts.length, 1);
  const switching = h.state.toggle(true); h.auto(); assert.equal(h.starts.length, 1);
  h.starts[0].task.resolve({ loginId: 'late', authUrl: 'late-url' }); await settlePromises();
  assert.equal(h.cancellations[0].id, 'late'); assert.equal(h.state.oauthUrl, null);
  h.auto(); assert.equal(h.starts.length, 1);
  h.cancellations[0].task.resolve(); await switching; await settlePromises();
  h.state.edit('http://chosen'); await h.state.startProxy(); h.auto();
  assert.equal(h.starts.length, 2); assert.equal(h.starts[1].args[0], 'http://chosen');
});

test('failed cleanup of a late start blocks replacements until the same session is cancelled', async () => {
  const h = harness({ oauthUrl: null, oauthActiveRef: { current: false }, oauthLoginIdRef: { current: null } });
  h.auto();
  const switching = h.state.toggle(true);
  h.starts[0].task.resolve({ loginId: 'late', authUrl: 'late-url' });
  await settlePromises();
  h.cancellations[0].task.reject(new Error('cleanup failed'));
  await switching; await settlePromises();
  h.auto();
  assert.equal(h.starts.length, 1);
  assert.equal(h.state.oauthUrl, null);
  assert.equal(h.state.oauthPrepareError, 'codex.oauthProxy.cancelFailed');
  assert.equal(h.state.oauthCancelLoginId.current, 'late');
  const retry = h.state.toggle(true);
  assert.equal(h.cancellations[1].id, 'late');
  h.cancellations[1].task.resolve();
  await retry; await settlePromises();
  h.state.edit('http://chosen'); await h.state.startProxy(); h.auto();
  assert.equal(h.starts.length, 2);
  assert.equal(h.starts[1].args[0], 'http://chosen');
});

test('editing repeatedly then starting during cancellation uses only the latest address', async () => {
  const h = harness({ oauthProxyEnabled: true, oauthProxyReady: true, oauthProxyInput: 'http://old' });
  h.state.edit('http://first'); h.state.edit('http://latest');
  const starting = h.state.startProxy(); h.auto();
  assert.equal(h.starts.length, 0); assert.equal(h.cancellations.length, 1);
  h.cancellations[0].task.resolve(); await starting; await settlePromises(); h.auto();
  assert.equal(h.starts.length, 1); assert.equal(h.starts[0].args[0], 'http://latest');
});

test('start failure does not cause a retry loop when the pending-task guard clears', async () => {
  const h = harness({ oauthUrl: null, oauthActiveRef: { current: false }, oauthLoginIdRef: { current: null } });
  h.auto(); h.starts[0].task.reject(new Error('start failed')); await settlePromises();
  h.auto(); assert.equal(h.starts.length, 1); assert.equal(h.state.oauthPrepareError, 'startFailed');
});
