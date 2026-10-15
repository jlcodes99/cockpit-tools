import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function harness(scope = 'api', options: Record<string, any> = {}) {
  const calls: any[] = [];
  const errors: any[] = [];
  const busy: boolean[] = [];
  const accounts: any[] = [{ id: 'a', app_speed: 'standard' }];
  const instances: any[] = [{ id: 'i', appSpeed: 'standard' }];
  const listeners = new Map<string, (event: any) => void>();
  const t = (key: string, values?: any) => values?.error ? `${key}:${values.error}` : key;
  const accountStore: any = (selector: any) => selector({ accounts });
  accountStore.getState = () => ({ applyAccountSnapshot: (value: any) => { accounts[0] = value; } });
  const instanceStore: any = (selector: any) => selector({ instances });
  instanceStore.getState = () => ({
    updateInstance: async (payload: any) => { calls.push(['instance', payload]); instances[0].appSpeed = payload.appSpeed; },
    fetchInstances: async () => {},
  });
  const services = {
    getCodexApiServiceAppSpeedConfig: async () => ({ speed: 'standard' }),
    saveCodexApiServiceAppSpeed: async (speed: string) => { calls.push(['api', speed]); return { speed }; },
    updateCodexAccountAppSpeed: async (id: string, speed: string) => { calls.push(['account', id, speed]); return { id, app_speed: speed }; },
    ...options,
  };
  const h = loadHookModule(new URL('./CodexSpeedControl.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '../../services/codexService': services,
    '../../stores/useCodexAccountStore': { useCodexAccountStore: accountStore },
    '../../stores/useCodexInstanceStore': { useCodexInstanceStore: instanceStore },
    './CodexSpeedSelect': { CodexSpeedSelect: () => null },
    '../../utils/codexDeepSeekAccess': { isDeepSeekAccount: (account: any) => account.api_provider_id === 'deepseek' },
  }, {
    setTimeout, clearTimeout,
    CustomEvent: class { type: string; detail: any; constructor(type: string, init: any) { this.type = type; this.detail = init.detail; } },
    window: {
      addEventListener: (name: string, listener: any) => listeners.set(name, listener),
      removeEventListener: (name: string) => listeners.delete(name),
      dispatchEvent: (event: any) => listeners.get(event.type)?.(event),
    },
  });
  const props: any = { scope, account: accounts[0], instanceId: 'i', onError: (value: any) => errors.push(value), onSavingChange: (value: boolean) => busy.push(value) };
  let tree = h.render(() => h.exports.CodexSpeedControl(props));
  const render = () => tree = h.flush();
  const select = () => tree.props.children[0].props;
  return { h, props, calls, errors, busy, render, select, services,
    async ready() { await settlePromises(); render(); },
  };
}

test('mounting API control reads its saved tier without opting into Fast', async () => {
  const h = harness();
  assert.equal(h.select().disabled, true);
  await h.ready();
  assert.equal(h.select().value, 'standard');
  assert.equal(h.select().disabled, false);
  assert.deepEqual(h.calls, []);
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'fast');
  assert.deepEqual(h.calls, [['api', 'fast']]);
  assert.deepEqual(h.busy, [true, false]);
  h.h.unmount();
});

test('a remounted API control reflects persisted Fast instead of defaulting its label', async () => {
  const h = harness('api', { getCodexApiServiceAppSpeedConfig: async () => ({ speed: 'fast' }) });
  await h.ready();
  assert.equal(h.select().value, 'fast');
  assert.deepEqual(h.calls, []);
  h.h.unmount();
});

test('account changes update the shared account snapshot and do not save API defaults', async () => {
  const h = harness('account');
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'fast');
  assert.deepEqual(h.calls, [['account', 'a', 'fast']]);
  h.h.unmount();
});

test('instance controls persist the selected target speed through the existing transaction', async () => {
  const h = harness('instance');
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'fast');
  assert.equal(JSON.stringify(h.calls), JSON.stringify([['instance', { instanceId: 'i', appSpeed: 'fast', deferBindAccountApplication: true }]]));
  h.h.unmount();
});

test('a failed save keeps the saved label, reports locally, and allows retry', async () => {
  let failing = true;
  const h = harness('account', { updateCodexAccountAppSpeed: async (id: string, speed: string) => {
    if (failing) throw Error('disk unavailable');
    return { id, app_speed: speed };
  } });
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'standard');
  assert.equal(h.select().busy, false);
  assert.match(h.errors[h.errors.length - 1], /codex.speed.saveFailed.*disk unavailable/);
  failing = false;
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'fast');
  assert.equal(h.errors[h.errors.length - 1], null);
  h.h.unmount();
});

test('duplicate clicks are single flight and a dismissed preview gets no late error', async () => {
  const save = deferred<any>();
  let count = 0;
  const h = harness('account', { updateCodexAccountAppSpeed: () => { count++; return save.promise; } });
  const first = h.select().onChange('fast');
  await h.select().onChange('fast');
  h.render();
  assert.equal(count, 1);
  assert.equal(h.select().busy, true);
  h.h.unmount();
  save.reject(Error('late failure')); await first;
  assert.deepEqual(h.errors, [null]);
  assert.deepEqual(h.busy, [true, false]);
});

test('API runtime sync failure reports a localized error and permits retry', async () => {
  let failing = true;
  const h = harness('api', { saveCodexApiServiceAppSpeed: async (speed: string) => {
    if (failing) throw Error('codex_api_service_speed_sync_failed: IPC timeout');
    return { speed };
  } });
  await h.ready();
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'standard');
  assert.equal(h.select().busy, false);
  assert.deepEqual(h.errors, [null, 'codex.speed.apiSyncFailed']);
  failing = false;
  await h.select().onChange('fast'); h.render();
  assert.equal(h.select().value, 'fast');
  assert.equal(h.errors[h.errors.length - 1], null);
  h.h.unmount();
});

test('a dismissed API preview receives no late runtime sync error', async () => {
  const save = deferred<any>();
  const h = harness('api', { saveCodexApiServiceAppSpeed: () => save.promise });
  await h.ready();
  const pending = h.select().onChange('fast');
  h.h.unmount();
  save.reject(Error('codex_api_service_speed_sync_failed')); await pending;
  assert.deepEqual(h.errors, [null]);
});

test('API read failure exposes retry without saving a guessed tier', async () => {
  let failing = true;
  const h = harness('api', { getCodexApiServiceAppSpeedConfig: async () => {
    if (failing) throw Error('read failed');
    return { speed: 'fast' };
  } });
  await h.ready();
  assert.equal(h.select().disabled, true);
  assert.match(h.errors[h.errors.length - 1], /codex.speed.loadFailed/);
  failing = false;
  const retry = h.render().props.children[1];
  retry.props.onClick(); h.render(); await h.ready();
  assert.equal(h.select().disabled, false);
  assert.equal(h.select().value, 'fast');
  assert.deepEqual(h.calls, []);
  h.h.unmount();
});
