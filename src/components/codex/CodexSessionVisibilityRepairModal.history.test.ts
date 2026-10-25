import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

type Element = { type: unknown; props: Record<string, any> };
const prefix = 'codex.sessionManager.repairModal.';
function nodes(value: any): Element[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}
function contains(value: any, text: string): boolean {
  if (typeof value === 'string') return value === text;
  if (Array.isArray(value)) return value.some((child) => contains(child, text));
  return Boolean(value && contains(value.props?.children, text));
}

function harness(options: { repairKind?: 'visibility' | 'history'; running?: boolean; secondaryRunning?: boolean; selected?: string[] } = {}) {
  const providers = deferred<any>();
  const calls: Array<{ runId: string; options: any }> = [];
  const repaired: unknown[] = [];
  const running: boolean[] = [];
  let closed = 0;
  let formatted = 0;
  let providerLoads = 0;
  const error = { message: null as string | null, scrollKey: 0, set(value: string | null) {
    error.message = value;
    if (value) error.scrollKey += 1;
  } };
  let execute: (request: any) => Promise<any> = async (request) => ({
    message: request.dryRun ? 'history preview from host' : 'history reset from host',
    historyProjectionCount: 3,
    skippedSqliteFileCount: 1,
    mutatedInstanceCount: 0,
  });
  const store = {
    listSessionVisibilityRepairInstances: async () => ({ defaultInstanceId: 'default', instances: [
      { id: 'default', name: 'Default', isDefault: true, running: Boolean(options.running) },
      { id: 'secondary', name: 'Secondary', isDefault: false, running: Boolean(options.secondaryRunning) },
    ] }),
    listSessionVisibilityRepairProviders: () => { providerLoads += 1; return providers.promise; },
    repairSessionVisibilityAcrossInstances: async (runId: string, request: any) => {
      calls.push({ runId, options: structuredClone(request) });
      return execute(request);
    },
  };
  const t = (key: string, values?: any) => key + (values?.error ? `: ${values.error}` : '');
  const h = loadHookModule(new URL('./CodexSessionVisibilityRepairModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../stores/useCodexInstanceStore': { useCodexInstanceStore: (selector: any) => selector(store) },
    '../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => error },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    '../../utils/codexSessionVisibility': { formatCodexSessionVisibilityRepairMessage: () => {
      formatted += 1;
      return 'provider repair formatter';
    } },
  });
  const props = { open: true, repairKind: options.repairKind ?? 'history', description: 'provider migration description',
    selectedSessionIds: options.selected ?? ['chat-a', 'chat-a', 'chat-b'], totalSessionCount: 4,
    onClose: () => { closed += 1; }, onRepaired: (value: unknown) => { repaired.push(value); },
    onRunningChange: (value: boolean) => { running.push(value); } };
  h.render(() => h.exports.CodexSessionVisibilityRepairModal(props));
  const rendered = () => nodes(h.flush());
  const button = (key: string) => {
    const result = rendered().find((node) => node.type === 'button' && contains(node.props.children, prefix + key));
    assert.ok(result, `missing button ${key}`);
    return result;
  };
  return { h, calls, repaired, running, error, providers, rendered, button,
    closed: () => closed, formatted: () => formatted, providerLoads: () => providerLoads,
    kind: (kind: 'visibility' | 'history') => { props.repairKind = kind; h.flush(); },
    open: (value: boolean) => { props.open = value; h.flush(); },
    execute: (fn: typeof execute) => { execute = fn; },
    mode: (key: string) => { button(key).props.onClick(); h.flush(); },
    instance: (id: string) => {
      rendered().find((node) => node.type === 'dropdown' && node.props.ariaLabel === prefix + 'targetInstance')!.props.onChange(id);
      h.flush();
    },
  };
}

test('history repair ignores pending providers and passes only the selected instance and chat scope', async () => {
  const app = harness();
  assert.equal(app.button('preview').props.disabled, true);
  await settlePromises();
  assert.equal(app.providerLoads(), 0, 'the dedicated history entry must never load providers');
  for (const key of ['modeTitle', 'modeQuick', 'modeDeep', 'modeHistory']) {
    assert.equal(app.rendered().some((node) => contains(node, prefix + key)), false);
  }
  assert.equal(app.rendered().find((node) => node.props?.role === 'dialog')!.props['aria-label'], 'codex.sessionManager.actions.repairIndex');
  assert.equal(app.rendered().some((node) => node.type === 'dropdown' && node.props.ariaLabel === prefix + 'targetProvider'), false);
  assert.ok(app.rendered().some((node) => contains(node, prefix + 'historyDescription')));
  assert.equal(app.rendered().some((node) => contains(node, 'provider migration description')), false);
  assert.equal(app.button('preview').props.disabled, false);
  app.button('preview').props.onClick();
  await settlePromises();
  assert.deepEqual(app.calls[0].options, { mode: 'history', dryRun: true,
    targetInstanceId: null, targetProvider: null, repairInstanceIds: ['default'], sessionIds: ['chat-a', 'chat-b'] });
  assert.ok(app.rendered().some((node) => contains(node, 'history preview from host')));
  app.button('confirmRepair').props.onClick();
  await settlePromises();
  assert.deepEqual(app.calls[1].options, { ...app.calls[0].options, dryRun: false });
  assert.ok(app.rendered().some((node) => contains(node, 'history reset from host')));
  assert.equal(app.formatted(), 0, 'history must retain the host message instead of provider-migration summaries');
  assert.equal(app.closed(), 0);
  assert.equal(app.repaired.length, 1);
  app.h.unmount();
});

for (const rejectBeforeSwitch of [false, true]) {
  test(`history mode clears or ignores provider failures (${rejectBeforeSwitch ? 'existing' : 'late'})`, async () => {
    const app = harness({ repairKind: 'visibility' });
    await settlePromises();
    if (!rejectBeforeSwitch) app.kind('history');
    app.providers.reject(new Error('provider list unavailable'));
    await settlePromises();
    if (rejectBeforeSwitch) {
      assert.ok(app.error.message?.includes('providerLoadFailed'));
      app.kind('history');
    }
    await settlePromises();
    assert.equal(app.error.message, null);
    assert.equal(app.button('start').props.disabled, false);
    app.h.unmount();
  });
}

test('running history targets allow previews but block writes, including all-instance scope', async () => {
  const app = harness({ running: true });
  await settlePromises();
  assert.equal(app.button('preview').props.disabled, false);
  assert.equal(app.button('start').props.disabled, true);
  assert.ok(app.rendered().some((node) => contains(node, prefix + 'historyStopHint')));
  app.button('start').props.onClick();
  await settlePromises();
  assert.equal(app.calls.length, 0, 'write handler must enforce the running-state guard too');
  assert.equal(app.error.message, prefix + 'historyRunning');
  app.button('preview').props.onClick();
  await settlePromises();
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].options.dryRun, true);
  app.instance('secondary');
  assert.equal(app.button('start').props.disabled, false);
  app.button('instanceScopeAll').props.onClick();
  assert.equal(app.button('start').props.disabled, true);
  assert.equal(app.button('preview').props.disabled, false);
  app.h.unmount();
});

test('history failures keep the dialog and selection, preserve technical errors, and clear errors on retry', async () => {
  const app = harness();
  await settlePromises();
  app.instance('secondary');
  app.execute(async () => { throw new Error('HISTORY_SCHEMA_UNSUPPORTED'); });
  app.button('start').props.onClick();
  await settlePromises();
  assert.equal(app.error.message, 'HISTORY_SCHEMA_UNSUPPORTED');
  assert.ok(app.rendered().some((node) => node.props?.role === 'dialog'));
  assert.equal(app.closed(), 0);
  assert.equal(app.repaired.length, 0);
  assert.equal(app.button('start').props.disabled, false);
  const pending = deferred<any>();
  app.execute(() => pending.promise);
  app.button('start').props.onClick();
  app.h.flush();
  assert.equal(app.error.message, null);
  assert.equal(app.calls[1].options.targetProvider, null);
  assert.deepEqual(app.calls[1].options.repairInstanceIds, ['secondary']);
  assert.deepEqual(app.calls[1].options.sessionIds, ['chat-a', 'chat-b']);
  pending.resolve({ message: 'index reset only', historyProjectionCount: 1 });
  await settlePromises();
  assert.ok(app.rendered().some((node) => contains(node, 'index reset only')));
  assert.deepEqual(app.running.slice(-4), [true, false, true, false]);
  app.h.unmount();
});

for (const action of ['preview', 'start']) {
  test(`history ${action} preserves the backend-localized error without repeating its prefix`, async () => {
    const app = harness();
    await settlePromises();
      const message = '历史索引修复未完成：HISTORY_SCHEMA_UNSUPPORTED';
    app.execute(async () => { throw new Error(message); });
    app.button(action).props.onClick();
    await settlePromises();
    assert.equal(app.error.message, message);
    assert.equal(app.closed(), 0);
    assert.equal(app.repaired.length, 0);
    app.h.unmount();
  });
}

test('a history preview refreshes returned running states after exit and retains other instances', async () => {
  const app = harness({ running: true, secondaryRunning: true });
  await settlePromises();
  assert.equal(app.button('start').props.disabled, true);
  app.execute(async () => ({ message: 'history preview after client exit', historyProjectionCount: 1,
    items: [{ instanceId: 'default', running: false }] }));
  app.button('preview').props.onClick();
  await settlePromises();
  assert.equal(app.button('confirmRepair').props.disabled, false, 'stopped target can now be repaired');
  assert.equal(app.rendered().some((node) => contains(node, prefix + 'historyStopHint')), false);
  app.button('instanceScopeAll').props.onClick();
  assert.equal(app.button('start').props.disabled, true, 'an omitted running instance stays running');
  app.button('instanceScopeTarget').props.onClick();
  assert.equal(app.button('start').props.disabled, false);
  app.button('start').props.onClick();
  await settlePromises();
  assert.equal(app.calls.length, 2);
  assert.equal(app.calls[1].options.dryRun, false);
  assert.deepEqual(app.calls[1].options.repairInstanceIds, ['default']);
  app.h.unmount();
});

test('all-chat/all-instance history requests retain null scopes without inferring a provider', async () => {
  const app = harness({ selected: [] });
  await settlePromises();
  app.button('instanceScopeAll').props.onClick();
  app.button('preview').props.onClick();
  await settlePromises();
  assert.deepEqual(app.calls[0].options, { mode: 'history', dryRun: true,
    targetInstanceId: null, targetProvider: null, repairInstanceIds: null, sessionIds: null });
  app.h.unmount();
});

test('quick and deep modes retain provider selection and the stopped-client requirement', async () => {
  const app = harness({ running: true, repairKind: 'visibility' });
  app.providers.resolve({ defaultProvider: 'fixture', providers: [{ id: 'fixture', sources: [], isDefault: true }] });
  await settlePromises();
  assert.equal(app.button('start').props.disabled, false);
  app.button('preview').props.onClick();
  await settlePromises();
  assert.equal(app.calls[0].options.targetProvider, 'fixture');
  assert.equal(app.calls[0].options.targetInstanceId, 'default');
  app.mode('modeDeep');
  assert.equal(app.button('start').props.disabled, true);
  assert.ok(app.rendered().some((node) => contains(node, prefix + 'stopInstanceHint')));
  assert.equal(app.rendered().some((node) => contains(node, prefix + 'modeHistory')), false);
  app.h.unmount();
});

test('changing repair kind and reopening initialize the correct mode and selected-chat scope', async () => {
  const app = harness({ repairKind: 'visibility' });
  app.providers.resolve({ defaultProvider: 'fixture', providers: [{ id: 'fixture', sources: [], isDefault: true }] });
  await settlePromises();
  app.mode('modeDeep');
  app.button('scopeAll').props.onClick();
  app.button('preview').props.onClick();
  await settlePromises();
  assert.equal(app.calls[0].options.mode, 'deep');
  assert.equal(app.calls[0].options.sessionIds, null);
  const loads = app.providerLoads();
  app.kind('history');
  await settlePromises();
  assert.equal(app.providerLoads(), loads);
  assert.equal(app.rendered().some((node) => contains(node, 'provider repair formatter')), false);
  assert.equal(app.rendered().some((node) => contains(node, prefix + 'modeQuick')), false);
  app.button('preview').props.onClick();
  await settlePromises();
  assert.equal(app.calls[1].options.mode, 'history');
  assert.equal(app.calls[1].options.targetProvider, null);
  assert.deepEqual(app.calls[1].options.sessionIds, ['chat-a', 'chat-b']);
  app.kind('visibility');
  await settlePromises();
  assert.equal(app.button('modeQuick').props['aria-pressed'], true);
  assert.equal(app.rendered().some((node) => contains(node, prefix + 'modeHistory')), false);
  app.open(false);
  app.kind('history');
  app.open(true);
  await settlePromises();
  app.button('preview').props.onClick();
  await settlePromises();
  assert.equal(app.calls[2].options.mode, 'history');
  assert.deepEqual(app.calls[2].options.sessionIds, ['chat-a', 'chat-b']);
  app.h.unmount();
});

test('a late result from another repair kind cannot replace the new dialog state', async () => {
  const app = harness();
  await settlePromises();
  const pending = deferred<any>();
  app.execute(() => pending.promise);
  app.button('preview').props.onClick();
  app.h.flush();
  app.kind('visibility');
  app.providers.resolve({ defaultProvider: 'fixture', providers: [{ id: 'fixture', sources: [], isDefault: true }] });
  await settlePromises();
  pending.resolve({ message: 'late history result', items: [] });
  await settlePromises();
  assert.equal(app.rendered().some((node) => contains(node, 'late history result')), false);
  assert.equal(app.button('modeQuick').props['aria-pressed'], true);
  assert.equal(app.button('start').props.disabled, false);
  app.h.unmount();
});

test('history progress stages use dedicated localized messages', () => {
  const translations: Array<{ key: string; values: any }> = [];
  const t = (key: string, values?: any) => { translations.push({ key, values }); return key; };
  const h = loadHookModule(new URL('./CodexSessionVisibilityRepairModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '@tauri-apps/api/event': {},
    '../../hooks/useModalFocusTrap': {},
    '../../hooks/useEscClose': {},
    '../../stores/useCodexInstanceStore': {},
    '../ModalErrorMessage': {},
    '../SingleSelectDropdown': {},
    '../../utils/codexSessionVisibility': {},
  });
  for (const [stage, key] of [['scan_history_index', 'historyScan'], ['reset_history_index', 'historyReset']]) {
    translations.length = 0;
    h.render(() => h.exports.CodexSessionVisibilityRepairProgressView({ progress: {
      runId: 'history-run', mode: 'history', stage, current: 2, total: 7, percent: 20,
    } }));
    assert.ok(translations.some((entry) => entry.key === prefix + key));
    if (stage === 'scan_history_index') {
      const values = translations.find((entry) => entry.key === prefix + key)!.values;
      assert.equal(values.current, 2);
      assert.equal(values.total, 7);
    }
  }
  h.unmount();
});
