import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

test('API restart deduplicates confirmation and IPC, cancels without calls and retries after failure', async () => {
  const confirmations: ReturnType<typeof deferred<boolean>>[] = [];
  const restarts: ReturnType<typeof deferred<any>>[] = [];
  let error = '', notice = '', state: any;
  const h = loadHookModule(new URL('./useCodexApiServiceRestart.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { confirm() { const task = deferred<boolean>(); confirmations.push(task); return task.promise; } },
    '../services/codexLocalAccessService': { restartCodexLocalAccessSidecar() { const task = deferred<any>(); restarts.push(task); return task.promise; } },
  });
  h.render(() => h.exports.useCodexApiServiceRestart({ t: (key: string) => key,
    onState: (value: any) => { state = value; }, onError: (value: string) => { error = value; },
    onNotice: (value: string) => { notice = value; }, presentError: () => false }));
  h.flush().restart(); h.flush().restart();
  assert.equal(confirmations.length, 1);
  assert.equal(h.flush().restarting, true);
  confirmations[0].resolve(false); await settlePromises();
  assert.equal(restarts.length, 0);
  assert.equal(h.flush().restarting, false);
  h.flush().restart(); confirmations[1].resolve(true); await settlePromises();
  h.flush().restart(); assert.equal(restarts.length, 1); assert.equal(confirmations.length, 2);
  restarts[0].reject(new Error('port occupied')); await settlePromises();
  assert.equal(error, 'port occupied'); assert.equal(h.flush().restarting, false);
  h.flush().restart(); assert.equal(error, '');
  confirmations[2].resolve(true); await settlePromises();
  const saved = { recovering: false, collection: { port: 8317, apiKeys: ['key'], accountIds: ['account'] } };
  restarts[1].resolve(saved); await settlePromises();
  assert.equal(state, saved); assert.equal(notice, 'codex.localAccess.restartSuccess');
  assert.equal(h.flush().restarting, false);
  h.unmount();
});

test('confirmation errors also release the restart lock', async () => {
  let calls = 0, error = '';
  const h = loadHookModule(new URL('./useCodexApiServiceRestart.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { async confirm() { calls++; throw Error('dialog unavailable'); } },
    '../services/codexLocalAccessService': { async restartCodexLocalAccessSidecar() { assert.fail('must not restart'); } },
  });
  h.render(() => h.exports.useCodexApiServiceRestart({ t: String, onState() {}, onNotice() {},
    onError: (value: string) => { error = value; }, presentError: () => false }));
  await h.flush().restart(); await h.flush().restart();
  assert.equal(calls, 2); assert.equal(error, 'dialog unavailable'); assert.equal(h.flush().restarting, false);
});
