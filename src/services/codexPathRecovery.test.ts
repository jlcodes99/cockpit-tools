import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import * as recovery from '../stores/useCodexPathRecoveryStore';

const clone = (value: unknown) => JSON.parse(JSON.stringify(value));
const missing = new Error('APP_PATH_NOT_FOUND:codex');

function loadService(name: string, invoke: (command: string, args: any) => Promise<unknown>) {
  const events: any[] = [];
  const service = loadHookModule(new URL(name, import.meta.url), {
    '@tauri-apps/api/core': { invoke },
    '../stores/useCodexPathRecoveryStore': recovery,
    '../utils/codexProxyEnginePrerequisite': {
      withProxyEnginePrerequisite: (promise: Promise<unknown>) => promise,
      presentProxyEnginePrerequisite: () => false,
    },
    './platform/createPlatformInstanceService': {
      createPlatformInstanceService: () => ({
        openInstanceWindow: (instanceId: string) => invoke('codex_open_instance_window', { instanceId }),
      }),
    },
    '../utils/codexSwitchAuthFailure': { normalizeCodexSwitchError: (error: unknown) => error },
    './codexLaunchPreviewConfigService': {},
  }, {
    performance,
    console: { info() {} },
    window: { dispatchEvent: (event: unknown) => { events.push(event); return true; } },
    CustomEvent: class { constructor(public type: string, public options: { detail: unknown }) {} },
  }).exports;
  return { service, events };
}

test('all desktop and CLI entry services retain exact target and launch options on repair', async () => {
  const cases: [string, unknown[], string, Record<string, unknown>][] = [
    ['startInstance', ['managed', { transferConflictingAccount: true, skipFailedStep: 'proxy' }],
      'codex_start_instance', { instanceId: 'managed', transferConflictingAccount: true, skipFailedStep: 'proxy' }],
    ['openInstanceWindow', ['managed'], 'codex_open_instance_window', { instanceId: 'managed' }],
    ['updateInstance', [{ instanceId: 'managed', bindAccountId: 'account', deferBindAccountApplication: true }],
      'codex_update_instance', { instanceId: 'managed', bindAccountId: 'account', deferBindAccountApplication: true }],
    ['previewCodexInstanceLaunchCommand', [{ userDataDir: 'C:/profile', workingDir: 'C:/repo', extraArgs: '--extra', terminal: 'pwsh', launchCommand: 'codex --model test' }],
      'codex_preview_instance_launch_command', { userDataDir: 'C:/profile', workingDir: 'C:/repo', extraArgs: '--extra', terminal: 'pwsh', launchCommand: 'codex --model test' }],
    ['getCodexInstanceLaunchCommand', ['managed', 'pwsh'], 'codex_get_instance_launch_command', { instanceId: 'managed', terminal: 'pwsh' }],
    ['executeCodexInstanceLaunchCommand', ['managed', 'pwsh'], 'codex_execute_instance_launch_command', { instanceId: 'managed', terminal: 'pwsh' }],
  ];
  for (const [method, args, command, expected] of cases) {
    const calls: any[] = [];
    const result = { instanceId: 'managed', value: 'native result' };
    const { service } = loadService('./codexInstanceService.ts', async (name, body) => {
      calls.push({ name, body: clone(body) });
      if (calls.length === 1) throw missing;
      return result;
    });
    const pending = service[method](...args);
    await settlePromises();
    const request = recovery.useCodexPathRecoveryStore.getState().request;
    assert.ok(request, method);
    assert.equal(request.continueWithoutDesktop, undefined, 'manual/CLI start must not be reported as a switch-only success');
    await request.retry();
    assert.equal(await pending, result);
    assert.deepEqual(calls, [{ name: command, body: expected }, { name: command, body: expected }]);
  }
});

test('API activation repair keeps the selected profile and can explicitly continue without desktop launch', async () => {
  const calls: any[] = [];
  const result = { running: true, collection: { enabled: true } };
  const { service, events } = loadService('./codexLocalAccessService.ts', async (name, body) => {
    calls.push({ name, body: clone(body) });
    if (calls.length === 1) throw missing;
    return result;
  });
  const pending = service.activateCodexLocalAccess('managed');
  await settlePromises();
  const request = recovery.useCodexPathRecoveryStore.getState().request!;
  await request.continueWithoutDesktop!();
  assert.equal(await pending, result);
  assert.deepEqual(calls, [
    { name: 'codex_local_access_activate', body: { autoRepairMode: null, pathRecoveryManaged: true, instanceId: 'managed', launchAfterSwitch: null } },
    { name: 'codex_local_access_activate', body: { autoRepairMode: null, pathRecoveryManaged: true, instanceId: 'managed', launchAfterSwitch: false } },
  ]);
  assert.deepEqual(events.map((event) => clone(event.options.detail)), [{
    type: 'cancelled', cancelled: true, instanceId: 'managed', isDefault: false,
    operation: 'instance-launch', canRetry: false,
  }]);
});

test('API enable catches early-stop path errors but disabling never starts a recovery transaction', async () => {
  const calls: any[] = [];
  const { service } = loadService('./codexLocalAccessService.ts', async (name, body) => {
    calls.push({ name, body: clone(body) });
    if (calls.length === 1) throw missing;
    return { running: body.enabled };
  });
  const pending = service.setCodexLocalAccessEnabled(true);
  await settlePromises();
  await recovery.useCodexPathRecoveryStore.getState().request!.retry();
  assert.deepEqual(await pending, { running: true });
  await service.setCodexLocalAccessEnabled(false);
  assert.deepEqual(calls.map((call) => call.body), [{ enabled: true }, { enabled: true }, { enabled: false }]);
  assert.equal(recovery.useCodexPathRecoveryStore.getState().request, null);
});

test('switch-only recovery keeps authorization generation and does not report desktop launch completion', async () => {
  const calls: any[] = [];
  const result = { id: 'account' };
  const { service, events } = loadService('./codexService.ts', async (name, body) => {
    calls.push({ name, body: clone(body) });
    if (calls.length === 1) throw missing;
    return result;
  });
  const pending = service.switchCodexAccount('account', { launchAfterSwitch: true, reauthTokenGeneration: 42 });
  await settlePromises();
  await recovery.useCodexPathRecoveryStore.getState().request!.continueWithoutDesktop!();
  assert.equal(await pending, result);
  assert.deepEqual(calls, [
    { name: 'switch_codex_account', body: { accountId: 'account', autoRepairMode: null, pathRecoveryManaged: true, reauthTokenGeneration: 42, launchAfterSwitch: true } },
    { name: 'switch_codex_account', body: { accountId: 'account', autoRepairMode: null, pathRecoveryManaged: true, reauthTokenGeneration: 42, launchAfterSwitch: false } },
  ]);
  const launchEvents = events.filter((event) => event.type === 'codex:instance-launch-progress');
  assert.deepEqual(launchEvents.map((event) => event.options.detail.type), ['start', 'cancelled']);
  const switchStarts = events.filter((event) => event.type === 'codex-switch-progress' && event.options.detail.type === 'start');
  assert.deepEqual(switchStarts.map((event) => event.options.detail.launchAfterSwitch), [true, false]);
});

test('cancelling instance path recovery clears only its own launch progress; blocked newcomers emit nothing', async () => {
  const calls: string[] = [];
  const { service, events } = loadService('./codexInstanceService.ts', async (_command, args) => {
    calls.push(args.instanceId);
    throw missing;
  });
  const first = service.startInstance('first');
  const rejected = assert.rejects(first, /CODEX_START_CANCELLED/);
  await settlePromises();
  const request = recovery.useCodexPathRecoveryStore.getState().request!;
  await assert.rejects(service.startInstance('second'), /CODEX_START_CANCELLED/);
  assert.deepEqual(calls, ['first']);
  assert.equal(events.length, 0, 'an operation never dispatched must not cancel another instance progress');
  request.cancel();
  await rejected;
  assert.deepEqual(events.map((event) => clone(event.options.detail)), [{
    type: 'cancelled', cancelled: true, instanceId: 'first', isDefault: false,
    operation: 'instance-launch', canRetry: false,
  }]);
});

test('API path recovery cancellation targets the original default or managed instance without success events', async () => {
  for (const [instanceId, targetId] of [[undefined, '__default__'], ['managed', 'managed']]) {
    const { service, events } = loadService('./codexLocalAccessService.ts', async () => { throw missing; });
    const pending = service.activateCodexLocalAccess(instanceId);
    const rejected = assert.rejects(pending, /CODEX_START_CANCELLED/);
    await settlePromises();
    recovery.useCodexPathRecoveryStore.getState().request!.cancel();
    await rejected;
    assert.equal(events.length, 1);
    assert.equal(events[0].type, 'codex:instance-launch-progress');
    assert.deepEqual(clone(events[0].options.detail), {
      type: 'cancelled', cancelled: true, instanceId: targetId, isDefault: targetId === '__default__',
      operation: 'instance-launch', canRetry: false,
    });
  }
});

test('a blocked account switch cannot overwrite the active switch progress, and repair restarts only its original account', async () => {
  const calls: string[] = [];
  const { service, events } = loadService('./codexService.ts', async (_command, args) => {
    calls.push(args.accountId);
    if (calls.length === 1) throw missing;
    return { id: args.accountId };
  });
  const pending = service.switchCodexAccount('first', { launchAfterSwitch: true });
  await settlePromises();
  const request = recovery.useCodexPathRecoveryStore.getState().request!;
  const before = events.length;
  await assert.rejects(service.switchCodexAccount('second', { launchAfterSwitch: true }), /CODEX_START_CANCELLED/);
  assert.equal(events.length, before, 'no start or cancelled events for a switch that never reached the backend');
  await request.retry();
  assert.deepEqual(await pending, { id: 'first' });
  assert.deepEqual(calls, ['first', 'first']);
  const starts = events.filter((event) => event.type === 'codex:instance-launch-progress' && event.options.detail.type === 'start');
  assert.deepEqual(starts.map((event) => event.options.detail.accountId), ['first', 'first']);
});

test('duplicate switch callers share a switch-only result without a false desktop-complete event', async () => {
  let calls = 0;
  const { service, events } = loadService('./codexService.ts', async () => {
    if (++calls === 1) throw missing;
    return { id: 'account' };
  });
  const first = service.switchCodexAccount('account', { launchAfterSwitch: true });
  const duplicate = service.switchCodexAccount('account', { launchAfterSwitch: true });
  await settlePromises();
  await recovery.useCodexPathRecoveryStore.getState().request!.continueWithoutDesktop!();
  await Promise.all([first, duplicate]);
  assert.equal(calls, 2);
  const launchEvents = events.filter((event) => event.type === 'codex:instance-launch-progress');
  assert.deepEqual(launchEvents.map((event) => event.options.detail.type), ['start', 'cancelled']);
});
