import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { isCodexPathMissingError } from '../../stores/useCodexPathRecoveryStore';

const ROOT = 'modal codex-path-recovery-modal';
function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
function harness(options: {
  config?: () => Promise<any>;
  invoke?: (command: string, args: any) => Promise<any>;
  retry?: () => Promise<void>;
  withoutDesktop?: boolean;
  windows?: boolean;
  timers?: boolean;
} = {}) {
  const calls: Array<[string, any]> = [];
  const events: string[] = [];
  const traps: any[] = []; const escapes: any[] = [];
  const timers = new Map<number, () => void>(); let timerId = 0;
  let retried = 0; let continued = 0; let canceled = 0;
  const state: any = { request: null };
  const makeRequest = (id: number) => ({ id, key: String(id),
    retry: async () => { retried++; await options.retry?.(); state.request = null; },
    continueWithoutDesktop: options.withoutDesktop ? async () => { continued++; state.request = null; } : undefined,
    cancel() { canceled++; state.request = null; },
  });
  state.request = makeRequest(1);
  const store: any = (selector: any) => selector(state);
  store.getState = () => state;
  const errors: any = { message: null, scrollKey: 0,
    clear() { this.message = null; }, report(value: string) { this.message = value; this.scrollKey++; } };
  const h = loadHookModule(new URL('./CodexPathRecoveryDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (tree: any, target: any) => { assert.equal(target, body); return tree; } },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/api/core': { invoke: async (command: string, args: any) => {
      calls.push([command, args == null ? args : JSON.parse(JSON.stringify(args))]);
      if (command === 'get_general_config') return options.config?.() ?? { codex_app_path: '', codex_launch_on_switch: true };
      if (options.invoke) return options.invoke(command, args);
      return command === 'scan_app_launch_targets' ? [] : undefined;
    } },
    '@tauri-apps/plugin-dialog': { open: async () => 'C:/Apps/ChatGPT.exe' },
    '../../stores/useCodexPathRecoveryStore': { useCodexPathRecoveryStore: store, isCodexPathMissingError },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean) => traps.push({ ref, open }) },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: any, ref: any) => escapes.push({ open, close, ref }) },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../../utils/reducedMotion': { scrollElementIntoView() {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => errors },
  }, { Event: class { constructor(public type: string) {} }, window: { dispatchEvent(event: { type: string }) { events.push(event.type); } }, document: { body: body }, navigator: { platform: options.windows === false ? 'MacIntel' : 'Win32' },
    ...(options.timers ? { setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; }, clearTimeout: (id: number) => timers.delete(id) } : {}),
  });
  h.render(() => { traps.length = 0; escapes.length = 0; return h.exports.CodexPathRecoveryDialog(); });
  const all = () => nodes(h.flush());
  const button = (key: string) => all().find((node) => node.type === 'button' && node.props.children === key);
  return { h, all, button, calls, traps, escapes, errors, timers, events,
    get retried() { return retried; }, get continued() { return continued; }, get canceled() { return canceled; },
    get request() { return state.request; },
    async ready() { await settlePromises(); h.flush(); },
    input(value: string) { all().find((node) => node.type === 'input').props.onChange({ target: { value } }); h.flush(); },
    toggle() { all().find((node) => node.props?.role === 'switch').props.onClick(); h.flush(); },
    replace(id: number) { state.request = makeRequest(id); h.flush(); },
  };
}
const body = {};

test('path recovery is portaled, shares its root with focus/Escape, and leaves the backdrop inert', async () => {
  const h = harness(); await h.ready();
  const dialog = h.all().find((node) => node.props?.className === ROOT);
  const overlay = h.all().find((node) => node.props?.className === 'modal-overlay codex-path-recovery-overlay');
  assert.equal(dialog.props.role, 'dialog'); assert.equal(dialog.props.tabIndex, -1);
  assert.equal(dialog.props['aria-modal'], 'true'); assert.equal(overlay.props.onClick, undefined);
  assert.equal(h.traps[0].ref, dialog.props.ref); assert.equal(h.escapes[0].ref, dialog.props.ref);
  const css = readFileSync(new URL('./CodexPathRecoveryDialog.css', import.meta.url), 'utf8');
  assert.match(css, /z-index: 2147483647/);
  assert.match(css, /\.modal\.codex-path-recovery-modal\s*\{[^}]*max-height: calc\(100dvh - 32px\)/);
  assert.match(css, /\.codex-path-recovery-body\s*\{[^}]*min-height: 0;[^}]*overflow-y: auto/);
  assert.match(css, /\.modal-footer \{ flex-shrink: 0; \}/);
  h.escapes[0].close(); assert.equal(h.canceled, 1); assert.equal(h.all().length, 0);
  h.h.unmount();
});

test('configuration read failure never becomes editable defaults and exposes a retry', async () => {
  let fail = true;
  const h = harness({ config: async () => { if (fail) throw Error('config unavailable'); return { codex_app_path: 'D:/Codex/ChatGPT.exe', codex_launch_on_switch: false }; } });
  await h.ready();
  assert.equal(h.all().find((node) => node.type === 'input').props.disabled, true);
  assert.equal(h.button('codex.pathRecovery.saveAndRetry').props.disabled, true);
  assert.equal(h.errors.message, 'config unavailable');
  fail = false; h.button('codex.pathRecovery.reload').props.onClick(); await h.ready();
  assert.equal(h.all().find((node) => node.type === 'input').props.value, 'D:/Codex/ChatGPT.exe');
  assert.equal(h.all().find((node) => node.props?.role === 'switch').props['aria-checked'], false);
  assert.equal(h.errors.message, null);
  h.h.unmount();
});

test('saving validates path then persists the shared switch and retries the original operation', async () => {
  const h = harness(); await h.ready(); h.input(' C:/Apps/ChatGPT.exe ');
  h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  assert.deepEqual(h.calls.slice(1), [
    ['validate_codex_app_path', { path: 'C:/Apps/ChatGPT.exe' }],
    ['set_app_path', { app: 'codex', path: 'C:/Apps/ChatGPT.exe' }],
    ['set_codex_launch_on_switch', { enabled: true }],
  ]);
  assert.equal(h.retried, 1); assert.equal(h.request, null);
  assert.deepEqual(h.events, ['config-updated', 'config-updated']);
  h.h.unmount();
});

test('an invalid GUI path stays beside the field and cannot proceed to settings or launch', async () => {
  const h = harness({ invoke: async () => { throw Error('CODEX_APP_PATH_INVALID'); } });
  await h.ready(); h.input('C:/Apps/resources/codex.exe');
  h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  assert.equal(h.retried, 0); assert.ok(h.request);
  const input = h.all().find((node) => node.type === 'input'); assert.equal(input.props['aria-invalid'], true);
  assert.equal(h.all().find((node) => node.props?.id === 'codex-path-recovery-field-error').props.children, 'codex.pathRecovery.invalidPath');
  assert.equal(h.calls.length, 2);
  h.input('C:/Apps/ChatGPT.exe'); assert.equal(h.all().find((node) => node.type === 'input').props['aria-invalid'], false);
  h.h.unmount();
});

for (const withoutDesktop of [false, true]) {
  test(`automatic startup off supports ${withoutDesktop ? 'continuing the switch' : 'settings-only cancellation'} without a path`, async () => {
    const h = harness({ withoutDesktop }); await h.ready(); h.toggle();
    const key = withoutDesktop ? 'codex.pathRecovery.saveWithoutLaunch' : 'codex.pathRecovery.saveOnly';
    h.button(key).props.onClick(); await h.ready();
    assert.deepEqual(h.calls.slice(1), [['set_codex_launch_on_switch', { enabled: false }]]);
    assert.equal(h.retried, 0); assert.equal(h.continued, withoutDesktop ? 1 : 0); assert.equal(h.canceled, withoutDesktop ? 0 : 1);
    h.h.unmount();
  });
}

test('turning off automatic startup does not require repairing an unchanged stale path', async () => {
  const h = harness({ withoutDesktop: true, config: async () => ({ codex_app_path: 'C:/removed/ChatGPT.exe', codex_launch_on_switch: true }) });
  await h.ready(); h.toggle(); h.button('codex.pathRecovery.saveWithoutLaunch').props.onClick(); await h.ready();
  assert.equal(h.calls.some(([command]) => command === 'set_app_path'), false);
  assert.equal(h.continued, 1); assert.equal(h.retried, 0);
  h.h.unmount();
});

test('Windows detection lists candidates without guessing between multiple clients', async () => {
  const h = harness({ invoke: async () => [
    { label: 'First', target: 'C:/First/ChatGPT.exe', target_type: 'exe' },
    { label: 'Second', target: 'C:/Second/ChatGPT.exe', target_type: 'exe' },
  ] });
  await h.ready();
  h.all().find((node) => node.type === 'button' && Array.isArray(node.props.children) && node.props.children.includes('appPath.missing.scanApps')).props.onClick(); await h.ready();
  const candidates = h.all().filter((node) => node.props?.className?.includes('codex-path-recovery-candidate') && node.type === 'button');
  assert.equal(candidates.length, 2); assert.equal(h.all().find((node) => node.type === 'input').props.value, '');
  candidates[1].props.onClick(); assert.equal(h.all().find((node) => node.type === 'input').props.value, 'C:/Second/ChatGPT.exe');
  assert.deepEqual(h.calls[1], ['scan_app_launch_targets', { app: 'codex' }]);
  h.h.unmount();
});

test('canceling during a path write prevents a late completion from starting or changing a replacement dialog', async () => {
  const write = deferred<void>(); const h = harness({ invoke: async (command) => command === 'set_app_path' ? write.promise : undefined });
  await h.ready(); h.input('C:/First/ChatGPT.exe'); h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  h.button('common.cancel').props.onClick(); h.replace(2); await h.ready(); h.input('D:/Second/ChatGPT.exe');
  write.resolve(); await h.ready();
  assert.equal(h.retried, 0); assert.equal(h.calls.some(([command]) => command === 'set_codex_launch_on_switch'), false);
  assert.equal(h.request.id, 2); assert.equal(h.all().find((node) => node.type === 'input').props.value, 'D:/Second/ChatGPT.exe');
  h.h.unmount();
});

test('a timed-out path write never launches later and leaves a retryable error', async () => {
  const write = deferred<void>(); const h = harness({ timers: true, invoke: async (command) => command === 'set_app_path' ? write.promise : undefined });
  await h.ready(); h.input('C:/Apps/ChatGPT.exe'); h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  for (const callback of h.timers.values()) callback(); await h.ready();
  assert.equal(h.errors.message, 'codex.pathRecovery.timeout'); assert.ok(h.request); assert.equal(h.retried, 0);
  write.resolve(); await h.ready(); assert.equal(h.retried, 0);
  assert.equal(h.calls.some(([command]) => command === 'set_codex_launch_on_switch'), false);
  h.h.unmount();
});

test('repeated save clicks cannot duplicate writes or retry the launch twice', async () => {
  const write = deferred<void>(); const h = harness({ invoke: async (command) => command === 'set_app_path' ? write.promise : undefined });
  await h.ready(); h.input('C:/Apps/ChatGPT.exe'); const submit = h.button('codex.pathRecovery.saveAndRetry').props.onClick;
  submit(); submit(); await h.ready(); assert.equal(h.calls.filter(([command]) => command === 'set_app_path').length, 1);
  write.resolve(); await h.ready(); assert.equal(h.retried, 1);
  h.h.unmount();
});

test('saving the shared switch fails in this dialog and never launches with unsaved settings', async () => {
  const h = harness({ invoke: async (command) => { if (command === 'set_codex_launch_on_switch') throw Error('settings write failed'); } });
  await h.ready(); h.input('C:/Apps/ChatGPT.exe'); h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  assert.ok(h.request); assert.equal(h.retried, 0); assert.equal(h.errors.message, 'settings write failed');
  assert.equal(h.button('codex.pathRecovery.saveAndRetry').props.disabled, false);
  h.h.unmount();
});

test('retry failure keeps the same dialog and original operation available', async () => {
  let fail = true;
  const h = harness({ retry: async () => { if (fail) throw Error('launch rejected'); } });
  await h.ready(); h.input('C:/Apps/ChatGPT.exe'); h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  assert.equal(h.request.id, 1); assert.equal(h.retried, 1); assert.equal(h.errors.message, 'launch rejected');
  fail = false; h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  assert.equal(h.retried, 2); assert.equal(h.request, null); assert.equal(h.errors.message, null);
  h.h.unmount();
});

test('a late config read cannot overwrite a new request or the newly entered path', async () => {
  const first = deferred<any>(); let reads = 0;
  const h = harness({ config: async () => ++reads === 1 ? first.promise : { codex_app_path: '', codex_launch_on_switch: false } });
  h.button('common.cancel').props.onClick(); h.replace(2); await h.ready(); h.input('D:/Chosen/ChatGPT.exe');
  first.resolve({ codex_app_path: 'C:/Old/ChatGPT.exe', codex_launch_on_switch: true }); await h.ready();
  assert.equal(h.all().find((node) => node.type === 'input').props.value, 'D:/Chosen/ChatGPT.exe');
  assert.equal(h.all().find((node) => node.props?.role === 'switch').props['aria-checked'], false);
  h.h.unmount();
});

test('non-Windows automatic detection notifies shared configuration readers after its native write', async () => {
  const h = harness({ windows: false, invoke: async () => '/Applications/ChatGPT.app' }); await h.ready();
  h.all().find((node) => node.type === 'button' && Array.isArray(node.props.children) && node.props.children.includes('settings.general.codexPathReset')).props.onClick(); await h.ready();
  assert.equal(h.all().find((node) => node.type === 'input').props.value, '/Applications/ChatGPT.app');
  assert.deepEqual(h.calls[1], ['detect_app_path', { app: 'codex', force: true }]);
  assert.deepEqual(h.events, ['config-updated']);
  h.h.unmount();
});

test('a canceled native settings write still refreshes readers if it completes, but never resumes startup', async () => {
  const write = deferred<void>(); const h = harness({ invoke: async (command) => command === 'set_app_path' ? write.promise : undefined });
  await h.ready(); h.input('C:/Apps/ChatGPT.exe'); h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  h.button('common.cancel').props.onClick(); write.resolve(); await h.ready();
  assert.deepEqual(h.events, ['config-updated']); assert.equal(h.retried, 0); assert.equal(h.request, null);
  h.h.unmount();
});

test('settings-only saves use normal path storage semantics without desktop validation', async () => {
  const h = harness(); await h.ready(); h.toggle(); h.input('D:/Future installation/ChatGPT.exe');
  h.button('codex.pathRecovery.saveOnly').props.onClick(); await h.ready();
  assert.equal(h.calls.some(([command]) => command === 'validate_codex_app_path'), false);
  assert.deepEqual(h.events, ['config-updated', 'config-updated']); assert.equal(h.canceled, 1); assert.equal(h.retried, 0);
  h.h.unmount();
});

test('a busy native path validator gives a localized retry hint and does not write configuration', async () => {
  const h = harness({ invoke: async () => { throw Error('CODEX_APP_PATH_VALIDATION_BUSY'); } });
  await h.ready(); h.input('C:/Apps/ChatGPT.exe'); h.button('codex.pathRecovery.saveAndRetry').props.onClick(); await h.ready();
  assert.equal(h.errors.message, 'codex.pathRecovery.validationBusy');
  assert.ok(h.request); assert.equal(h.retried, 0);
  assert.equal(h.calls.some(([command]) => command === 'set_app_path' || command === 'set_codex_launch_on_switch'), false);
  assert.equal(h.button('codex.pathRecovery.saveAndRetry').props.disabled, false);
  h.h.unmount();
});
