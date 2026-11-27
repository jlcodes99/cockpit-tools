import assert from 'node:assert/strict';
import test from 'node:test';
import { Children, isValidElement, type ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import * as providerQuota from '../../utils/codexProviderPoolQuota';
import type {
  T3CodeProviderInstanceView,
  T3CodeProviderRuntimeView,
} from '../../types/t3code';

const mockT = (key: string, options?: Record<string, unknown>) =>
  key === 'settings.general.codexAppUiInjectionWeeklyLabel'
    ? '周'
    : options && 'count' in options ? `${key}:${options.count}` : key;

const registryMod = loadHookModule(
  new URL('./t3codePlatformRegistry.tsx', import.meta.url),
  {
    '../icons/T3CodeIcon': { T3CodeIcon: () => null },
    '../../utils/durableUiSettings': {
      readDurableUiSetting: () => null,
      saveDurableUiSetting: async () => {},
    },
    '../../utils/platformMeta': { renderPlatformIcon: () => null },
  },
);

const presentationMod = loadHookModule(
  new URL('./t3codePresentation.tsx', import.meta.url),
  {
    'react-i18next': { useTranslation: () => ({ t: mockT }) },
    '../codex/CodexAccountPlanBadge': { CodexAccountPlanBadge: () => null },
    '../../utils/codexProviderPoolQuota': providerQuota,
    './t3codePlatformRegistry': registryMod.exports,
  },
);

const orderKey = 'agtools.t3code.platform_order.v1';

function loadOverview(readOrder = () => null as string | null) {
  const empty = () => null;
  return loadHookModule(new URL('./T3codeOverviewSection.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: mockT }) },
    '../QuickSettingsPopover': { QuickSettingsPopover: empty },
    '../SingleSelectDropdown': { SingleSelectDropdown: empty },
    './T3codeCommonSettings': { T3codeCommonSettings: empty },
    '../../hooks/useDurableUiSetting': { useDurableUiSetting: readOrder },
    '../../utils/codexProviderPoolQuota': providerQuota,
    '../../utils/durableUiSettings': { T3CODE_PLATFORM_ORDER_STORAGE_KEY: orderKey },
    '../../utils/platformMeta': { renderPlatformIcon: empty },
    './t3codePlatformRegistry': registryMod.exports,
    './t3codePresentation': presentationMod.exports,
  });
}

test('adding an account offers only accounts that are not already linked to the platform', () => {
  const { getDefaultProvider, getAvailableProviderAccounts } = registryMod.exports;
  const accounts = ['default', 'parallel', 'new'].map((id) => ({ id }));
  const workspace = {
    providerInstances: [
      { driver: 'workbuddy', instanceId: 'workbuddy-2', boundAccountId: 'parallel' },
      { driver: 'workbuddy', instanceId: 'workbuddy', boundAccountId: 'default' },
    ],
    extraAccounts: { workbuddy: accounts },
  };
  assert.equal(getDefaultProvider(workspace, 'workbuddy'), workspace.providerInstances[1]);
  assert.deepEqual(Array.from(getAvailableProviderAccounts(workspace, 'workbuddy'),
    (account: { id: string }) => account.id), ['new']);
  assert.equal(getDefaultProvider({ ...workspace, providerInstances: workspace.providerInstances.slice(0, 1) },
    'workbuddy'), undefined);
});

function loadAccountPicker() {
  return loadHookModule(new URL('./T3codeModals.tsx', import.meta.url), {
    'react-dom': { createPortal: (view: ReactNode) => view },
    'react-i18next': { useTranslation: () => ({ t: mockT }) },
    '../ModalErrorMessage': { ModalErrorMessage: () => null, useModalErrorState: () => ({ clear() {} }) },
    '../PlatformLaunchPathField': { PlatformLaunchPathField: () => null },
    './T3codeRuntimeSetup': { T3codeRuntimeSetup: () => null },
    '../../hooks/useEscClose': { useEscCloseTopmost: () => {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: () => {} },
    './t3codePlatformRegistry': registryMod.exports,
    './t3codePresentation': presentationMod.exports,
  }, { document: { body: {} } });
}

test('Antigravity account picker shows reported quota groups and omits individual model quotas', () => {
  const modal = loadAccountPicker();
  const models = [
    { id: 'gemini-flash-lite', label: 'Gemini Flash Lite', remainingPercent: 100 },
    { id: 'gemini-hydrogen-xhigh', label: 'Gemini Hydrogen (XHigh)', remainingPercent: 100 },
  ];
  const accounts = [
    {
      id: 'grouped', email: 'grouped@example.test', windows: models,
      quotaSummaryGroups: [
        { id: 'gemini', name: 'Gemini', windows: [
          { id: 'gemini-5h', period: '5h', percentage: 72 },
          { id: 'gemini-weekly', period: 'weekly', percentage: 49 },
        ] },
        { id: 'third-party', name: 'Third-party models', windows: [
          { id: '3p-5h', period: '5h', percentage: 0 },
        ] },
      ],
    },
    { id: 'models-only', email: 'models-only@example.test', windows: models },
  ];
  const view = modal.render(() => modal.exports.T3codeModals({
    modal: { kind: 'add', driver: 'antigravity' },
    workspace: { providerInstances: [], antigravityAccounts: accounts },
    gatewayName: '',
    maskAccountText: (value: string) => value,
    renderPlanBadge: () => null,
    onClose: () => {},
    onAdd: async () => {},
    onRemove: async () => {},
  }));
  const markup = renderToStaticMarkup(view);
  assert.match(markup, /Gemini 5h<.*72%/);
  assert.match(markup, /周<.*49%/);
  assert.match(markup, /Third-party models 5h<.*>0%/);
  assert.match(markup, /models-only@example\.test<.*t3code\.quotaQuery\.empty/);
  assert.doesNotMatch(markup, /Gemini Flash Lite|Gemini Hydrogen|100%/);
});

test('Codex API Service is offered once and shows its cumulative quota in both account views', () => {
  const windows = [
    { id: '5h', label: '5h', remainingPercent: 199 },
    { id: 'weekly', label: '7d', remainingPercent: 178 },
  ];
  const gatewayName = 'Codex API Service';
  const account = { id: '__api_service__', isApiService: true, windows };
  const linked: T3CodeProviderInstanceView = {
    instanceId: 'codex', driver: 'codex', displayName: 'Codex', enabled: true,
    isBuiltinDefault: true, boundAccountId: account.id, boundIsApiService: true, windows,
  };
  const shared = { gatewayName, maskAccountText: (value: string) => value, renderPlanBadge: () => null };
  const picker = loadAccountPicker();
  const renderPicker = (providerInstances: T3CodeProviderInstanceView[]) => renderToStaticMarkup(
    picker.render(() => picker.exports.T3codeModals({
      ...shared,
      modal: { kind: 'add', driver: 'codex' },
      workspace: { providerInstances, codexAccounts: [account] },
      onClose: () => {},
      onAdd: async () => {},
      onRemove: async () => {},
    })),
  );
  const overview = loadOverview();
  const overviewMarkup = renderToStaticMarkup(overview.render(() => overview.exports.T3codeOverviewSection({
    ...shared,
    workspace: { providerInstances: [linked], codexAccounts: [account] },
    providers: [{ instance: linked, status: { tone: 'ready' } }],
  })));
  for (const markup of [renderPicker([]), overviewMarkup]) {
    assert.match(markup, /Codex API Service/);
    assert.match(markup, /5h<.*199%/);
    assert.match(markup, /周<.*178%/);
  }
  assert.doesNotMatch(renderPicker([linked]), /Codex API Service|199%|178%/);
});

type Control = {
  'aria-label'?: string;
  'aria-pressed'?: boolean;
  type?: string;
  disabled?: boolean;
  children?: ReactNode;
  onClick?: () => void;
  onChange?: (event: { target: { value: string } }) => void;
  onSubmit?: (event: { preventDefault: () => void }) => void;
  items?: string[];
  onReorder?: (active: string, target: string) => void;
  message?: string | null;
};

function findControl(node: ReactNode, predicate: (props: Control) => boolean): Control | undefined {
  for (const child of Children.toArray(node)) {
    if (!isValidElement<Control>(child)) continue;
    if (predicate(child.props)) return child.props;
    const match = findControl(child.props.children, predicate);
    if (match) return match;
  }
  return undefined;
}

test('overview keeps launch available when T3 Code is running or stopped', () => {
  const overview = loadOverview();
  let launches = 0;
  const config = {
    workspace: { providerInstances: [] },
    providers: [],
    onStartOrRestart: () => { launches += 1; },
  };
  const getAction = (running: boolean, restarting = false) => {
    const view = overview.render(() => overview.exports.T3codeOverviewSection({
      ...config, t3Running: running, restarting,
    }));
    const label = `instances.actions.${running ? 'restart' : 'start'}`;
    return findControl(view, (control) => control.type === 'button' &&
      Children.toArray(control.children).includes(label));
  };

  const restart = getAction(true);
  assert.ok(restart?.onClick);
  assert.equal(restart.disabled, false);
  restart.onClick();
  assert.equal(getAction(true, true)?.disabled, true);

  const start = getAction(false);
  assert.ok(start?.onClick);
  assert.equal(start.disabled, false);
  start.onClick();
  assert.equal(launches, 2);
});

test('unified overview shows every linked account, moves the default to the top and exposes account actions', () => {
  let savedOrder: string | null = null;
  const overview = loadOverview(() => savedOrder);
  const accounts = ['Primary', 'Alpha', 'Beta', 'Gamma', 'Delta', 'Unlinked'].map((displayName, index) => ({
    id: String(index),
    displayName,
    email: `${displayName.toLowerCase()}@example.test`,
  }));
  const instances = accounts.slice(0, -1).map((account, index): T3CodeProviderInstanceView => ({
    instanceId: index === 1 ? 'workbuddy' : `workbuddy-${index}`,
    displayName: account.displayName,
    driver: 'workbuddy',
    enabled: true,
    isBuiltinDefault: false,
    boundAccountId: account.id,
    boundAccountPlan: 'PRO',
    boundAccountEmail: account.email,
    boundIsApiService: false,
    windows: [{ id: 'credits', label: 'Credits', remainingAmount: 75 }],
  }));
  const emptyDefault: T3CodeProviderInstanceView = {
    instanceId: 'codex',
    driver: 'codex',
    displayName: 'Codex',
    enabled: false,
    isBuiltinDefault: true,
    boundIsApiService: false,
    windows: [],
  };
  const added: string[] = [];
  const promoted: string[] = [];
  const removed: string[] = [];
  const config = {
    workspace: { providerInstances: [...instances, emptyDefault], extraAccounts: { workbuddy: accounts } },
    providers: [...instances, emptyDefault].map((instance) => ({ instance, status: { tone: 'ready' } })),
    maskAccountText: (value: string) => value.replace('@example.test', '@masked.test'),
    renderPlanBadge: (_driver: string, _id: string, plan: string) => plan,
    onAdd: (driver: string) => added.push(driver),
    onSetDefault: (instance: T3CodeProviderInstanceView) => promoted.push(instance.instanceId),
    onRemove: (instance: T3CodeProviderInstanceView) => removed.push(instance.instanceId),
  };
  const view = overview.render(() => overview.exports.T3codeOverviewSection(config));
  const markup = renderToStaticMarkup(view);
  assert.match(markup, /aria-label="Codex".*aria-label="WorkBuddy"/);
  assert.match(markup, />Alpha<.*>Primary<.*>Beta<.*>Gamma<.*>Delta</);
  assert.doesNotMatch(markup, />Unlinked<|@example\.test|100%/);
  assert.match(markup, /primary@masked\.test.*>75</);

  const defaultActions = findControl(view, (control) => Children.toArray(control.children).some((child) =>
    isValidElement<Control>(child) && child.props['aria-label'] === 't3code.overview.removeAccount · Alpha'));
  const [defaultBadge] = Children.toArray(defaultActions?.children);
  assert.ok(isValidElement<Control>(defaultBadge));
  assert.equal(defaultBadge.props.children, 't3code.providerWorkspace.builtinTag');

  const openAdd = findControl(view, (control) => control.children != null &&
    Children.toArray(control.children).includes('t3code.providerWorkspace.addAccount'))?.onClick;
  const setDefault = findControl(view, (control) => control['aria-label'] === 't3code.overview.setDefaultAccount · Primary')?.onClick;
  const removeDefault = findControl(view, (control) => control['aria-label'] === 't3code.overview.removeAccount · Alpha')?.onClick;
  assert.ok(openAdd && setDefault && removeDefault);
  openAdd();
  setDefault();
  removeDefault();
  assert.deepEqual(added, ['codex']);
  assert.deepEqual(promoted, ['workbuddy-0']);
  assert.deepEqual(removed, ['workbuddy']);
  assert.equal(findControl(view, (control) => control['aria-label'] === 't3code.overview.setDefaultAccount · Alpha'), undefined);

  savedOrder = JSON.stringify(['grok', 'workbuddy', 'retired', 'codex', 'workbuddy']);
  const reordered = renderToStaticMarkup(overview.flush());
  assert.match(reordered, /aria-label="WorkBuddy".*>Alpha<.*>Primary<.*aria-label="Codex"/);
  assert.doesNotMatch(reordered, /aria-label="Grok"|aria-label="retired"/);

  const search = findControl(view, (control) => control.type === 'search')?.onChange;
  assert.ok(search);
  search({ target: { value: 'delta@example.test' } });
  const filtered = renderToStaticMarkup(overview.flush());
  assert.match(filtered, />Delta</);
  assert.doesNotMatch(filtered, />Primary<|>Alpha<|>Beta<|>Gamma</);
});

test('platform order dialog keeps changes in draft until saved and restores the written order on reopen', async () => {
  let savedOrder: string | null = JSON.stringify(['codebuddy', 'retired', 'codebuddy']);
  let failSave = true;
  let closes = 0;
  function openDialog() {
    const dialog = loadHookModule(new URL('./T3codePlatformOrder.tsx', import.meta.url), {
      'react-dom': { createPortal: (view: ReactNode) => view },
      'react-i18next': { useTranslation: () => ({ t: mockT }) },
      '../../hooks/useDurableUiSetting': { useDurableUiSetting: () => savedOrder },
      '../../hooks/useEscClose': { useEscCloseTopmost: () => {} },
      '../../hooks/useModalFocusTrap': { useModalFocusTrap: () => {} },
      '../../hooks/useModalScrollLock': { useModalScrollLock: () => {} },
      '../../utils/durableUiSettings': {
        T3CODE_PLATFORM_ORDER_STORAGE_KEY: orderKey,
        saveDurableUiSetting: async (key: string, value: string) => {
          assert.equal(key, orderKey);
          if (failSave) throw new Error('disk write failed');
          savedOrder = value;
        },
      },
      '../../utils/platformMeta': { renderPlatformIcon: () => null },
      '../ModalErrorMessage': {
        ModalErrorMessage: () => null,
        useModalErrorState() {
          const [message, setMessage] = dialog.hooks.useState(null);
          return { message, clear: () => setMessage(null), report: setMessage, scrollKey: 0 };
        },
      },
      '../SortableLayoutList': { SortableLayoutItem: () => null, SortableLayoutList: () => null },
      './t3codePlatformRegistry': registryMod.exports,
    }, { document: { body: {} } });
    dialog.render(() => dialog.exports.T3codePlatformOrderDialog({ onClose: () => { closes += 1; } }));
    const list = () => findControl(dialog.flush(), (control) => Boolean(control.onReorder))!;
    const button = (label: string) => findControl(dialog.flush(), (control) => control.children === label)!;
    return { dialog, list, button };
  }

  const canceled = openDialog();
  const expected = ['codebuddy', 'antigravity', 'codex', 'claudeAgent', 'workbuddy',
    'workbuddy_ai', 'codebuddy_cn', 'grok', 'minimaxcode', 'qoder', 'qoder_cn'];
  assert.deepEqual(Array.from(canceled.list().items!), expected);
  canceled.list().onReorder!('minimaxcode', 'codebuddy');
  assert.equal(canceled.list().items?.[0], 'minimaxcode');
  canceled.button('common.cancel').onClick!();
  canceled.dialog.unmount();
  assert.equal(savedOrder, JSON.stringify(['codebuddy', 'retired', 'codebuddy']));
  assert.equal(closes, 1);

  const editor = openDialog();
  assert.deepEqual(Array.from(editor.list().items!), expected);
  editor.list().onReorder!('minimaxcode', 'codebuddy');
  await editor.button('common.save').onClick!();
  assert.ok(findControl(editor.dialog.flush(), (control) => control.message === 't3code.platformOrder.saveFailed'));
  assert.equal(editor.list().items?.[0], 'minimaxcode');
  assert.equal(closes, 1);
  failSave = false;
  await editor.button('common.save').onClick!();
  assert.equal(closes, 2);
  assert.equal(savedOrder, JSON.stringify(['minimaxcode', ...expected.filter((driver) => driver !== 'minimaxcode')]));
  assert.equal(openDialog().list().items?.[0], 'minimaxcode');
});

test('account overview registry includes both Qoder account platforms', () => {
  const { T3CODE_PLATFORM_SPECS } = registryMod.exports;
  assert.deepEqual(Array.from(T3CODE_PLATFORM_SPECS, (spec: { driver: string }) => spec.driver), [
    'antigravity',
    'codex',
    'claudeAgent',
    'workbuddy',
    'workbuddy_ai',
    'codebuddy',
    'codebuddy_cn',
    'grok',
    'minimaxcode',
    'qoder',
    'qoder_cn',
  ]);
});

test('Qoder CN switches only the selected provider to an unbound CN account', () => {
  const picker = loadAccountPicker();
  const selected = { id: 'target-cn', email: 'new-cn@example.test', windows: [
    { id: 'credits', label: 'Credits', remainingAmount: 42 },
  ] };
  const instance = { driver: 'qoder_cn', instanceId: 'qoder-cn-work', boundAccountId: 'current-cn' };
  const calls: unknown[] = [];
  const view = picker.render(() => picker.exports.T3codeModals({
    modal: { kind: 'switch', instance },
    workspace: {
      providerInstances: [instance],
      extraAccounts: {
        qoder: [{ id: 'global', email: 'global@example.test' }],
        qoder_cn: [{ id: 'current-cn', email: 'current-cn@example.test' }, selected],
      },
    },
    gatewayName: '',
    maskAccountText: (value: string) => value,
    renderPlanBadge: () => null,
    onClose: () => {},
    onSwitch: async (target: unknown, accountId: string) => calls.push([target, accountId]),
  }));
  const html = renderToStaticMarkup(view);
  assert.match(html, /new-cn@example\.test.*>42</);
  assert.doesNotMatch(html, /current-cn@example\.test|global@example\.test/);
  const option = findControl(view, (control) => control['aria-pressed'] === false);
  assert.ok(option?.onClick);
  option.onClick();
  const form = findControl(picker.flush(), (control) => Boolean(control.onSubmit));
  assert.ok(form?.onSubmit);
  form.onSubmit({ preventDefault() {} });
  assert.deepEqual(calls, [[instance, selected.id]]);
});

test('provider runtime resolution preserves staged state until live runtime catches up', () => {
  const { resolveEffectiveRuntime, resolveProviderRowStatus } = presentationMod.exports;

  const instance: T3CodeProviderInstanceView = {
    instanceId: 'codex',
    driver: 'codex',
    displayName: 'Codex',
    enabled: true,
    isBuiltinDefault: true,
    boundAccountId: 'acc-new',
    boundAccountEmail: 'new@example.test',
    boundIsApiService: false,
    boundUpdatedAt: 2000,
    runtime: {
      instanceId: 'codex',
      source: 'staged',
      modelCount: 4,
      matchesGateway: false,
    },
    windows: [],
  };

  const staleLive: T3CodeProviderRuntimeView = {
    instanceId: 'codex',
    source: 'live',
    authStatus: 'authenticated',
    availability: 'available',
    accountEmail: 'new@example.test',
    modelCount: 4,
    matchesGateway: false,
    checkedAt: new Date(1000 * 1000).toISOString(),
  };

  const effectiveStale = resolveEffectiveRuntime(instance, staleLive);
  assert.equal(effectiveStale?.source, 'staged');
  const stagedStatus = resolveProviderRowStatus(instance, effectiveStale, true, mockT);
  assert.equal(stagedStatus.tone, 'pending');
  assert.equal(stagedStatus.needsRestart, true);
  const afterRestart = resolveProviderRowStatus(instance, effectiveStale, true, mockT, new Date(2100 * 1000).toISOString());
  assert.equal(afterRestart.needsRestart, false);
  assert.equal(afterRestart.text, 't3code.providerWorkspace.runtimePending');
  const loadingModels = resolveProviderRowStatus(
    { ...instance, modelRefreshPending: true }, effectiveStale, true, mockT,
  );
  assert.equal(loadingModels.tone, 'pending');
  assert.equal(loadingModels.needsRestart, false);

  const applied = { ...instance, bindingAppliedLive: true };
  for (const pendingRuntime of [undefined, effectiveStale, staleLive]) {
    const status = resolveProviderRowStatus(applied, pendingRuntime, true, mockT);
    assert.equal(status.tone, 'pending');
    assert.equal(status.needsRestart, false);
  }

  const freshLive: T3CodeProviderRuntimeView = {
    ...staleLive,
    checkedAt: new Date(2500 * 1000).toISOString(),
  };
  const effectiveFresh = resolveEffectiveRuntime(instance, freshLive);
  assert.equal(effectiveFresh?.source, 'live');
  const readyStatus = resolveProviderRowStatus(instance, effectiveFresh, true, mockT);
  assert.equal(readyStatus.tone, 'ready');
  assert.equal(readyStatus.needsRestart, false);
  const signedOutStatus = resolveProviderRowStatus(
    applied,
    { ...freshLive, authStatus: 'unauthenticated' },
    true,
    mockT,
  );
  assert.equal(signedOutStatus.tone, 'warning');
  const workbuddyStatus = resolveProviderRowStatus(
    { ...instance, driver: 'workbuddy' },
    { ...freshLive, accountEmail: undefined, authStatus: 'unknown', status: 'ready' },
    true,
    mockT,
  );
  assert.equal(workbuddyStatus.tone, 'ready');
  assert.equal(workbuddyStatus.needsRestart, false);
});

test('Grok becomes ready without a reported workspace ID and keeps confirmed conflicts visible', () => {
  const { resolveProviderRowStatus } = presentationMod.exports;
  const instance: T3CodeProviderInstanceView = {
    instanceId: 'grok', driver: 'grok', displayName: 'Grok', enabled: true, isBuiltinDefault: true,
    boundAccountId: 'grok-account', boundAccountEmail: 'grok@example.test', boundAccountWorkspaceId: 'team',
    boundIsApiService: false, windows: [],
  };
  const runtime: T3CodeProviderRuntimeView = {
    instanceId: 'grok', source: 'live', authStatus: 'authenticated', availability: 'available',
    accountEmail: 'grok@example.test', modelCount: 4, matchesGateway: false,
  };
  assert.equal(resolveProviderRowStatus(instance, runtime, true, mockT).text, 't3code.providerWorkspace.modelsReady:4');
  assert.equal(resolveProviderRowStatus(instance, { ...runtime, workspaceId: 'other' }, true, mockT).text,
    't3code.overview.accountMismatch');
  assert.equal(resolveProviderRowStatus(instance, { ...runtime, accountEmail: 'other@example.test' }, true, mockT).text,
    't3code.overview.accountMismatch');
});

test('a saved default selection reports waiting, restart, and offline failure distinctly', () => {
  const { resolveProviderRowStatus } = presentationMod.exports;
  const instance: T3CodeProviderInstanceView = {
    instanceId: 'antigravity-other', driver: 'antigravity', displayName: 'Antigravity',
    enabled: true, isBuiltinDefault: false, boundAccountId: 'selected',
    boundIsApiService: false, windows: [],
  };
  const status = (state: NonNullable<T3CodeProviderInstanceView['pendingDefault']>['state'], running: boolean) =>
    resolveProviderRowStatus({ ...instance, pendingDefault: { state } }, undefined, running, mockT);

  assert.equal(status('waiting', true).text, 't3code.providerWorkspace.defaultWaiting');
  assert.equal(status('waiting', true).needsRestart, false);
  assert.equal(status('restartRequired', true).needsRestart, true);
  assert.equal(status('restartRequired', false).text, 't3code.providerWorkspace.readyOnLaunch');
  assert.equal(status('failed', false).text, 't3code.providerWorkspace.defaultApplyFailed');
});

test('a missing Antigravity runtime offers path setup even when an older model list is staged', () => {
  const { resolveEffectiveRuntime, resolveProviderRowStatus } = presentationMod.exports;
  const instance: T3CodeProviderInstanceView = {
    instanceId: 'antigravity', driver: 'antigravity', displayName: 'Antigravity',
    enabled: true, isBuiltinDefault: true, boundAccountId: 'account',
    boundIsApiService: false, boundUpdatedAt: 2000, windows: [],
    runtime: { instanceId: 'antigravity', source: 'staged', modelCount: 11, matchesGateway: false },
  };
  const missing: T3CodeProviderRuntimeView = {
    instanceId: 'antigravity', installed: false, status: 'error', modelCount: 0,
    matchesGateway: false, source: 'cache', checkedAt: new Date(2100 * 1000).toISOString(),
    message: 'Antigravity ACP runtime is not installed',
  };
  const runtime = resolveEffectiveRuntime(instance, missing);
  const status = resolveProviderRowStatus(instance, runtime, true, mockT);
  assert.equal(status.text, 't3code.providerWorkspace.runtimeNotFound');
  assert.equal(status.pathTarget, 'antigravity_cli');
  assert.equal(status.needsRestart, false);
  assert.equal(status.title, missing.message);
});

test('account label masking and quota progress rendering follow shared semantics', () => {
  const { T3codeQuotaSection, formatAccountLabel } = presentationMod.exports;

  const masked = formatAccountLabel(
    { email: 'alice@example.com', displayName: 'Alice', isApiService: false },
    'Local Gateway',
    (text: string) => `***${text.slice(-4)}`,
  );
  assert.equal(masked, '***lice · ***.com');

  const gateway = formatAccountLabel(
    { email: 'ignored@example.com', displayName: 'Ignored', isApiService: true },
    'Cockpit Gateway',
    () => '***',
  );
  assert.equal(gateway, 'Cockpit Gateway');

  const tree = presentationMod.render(() =>
    T3codeQuotaSection({
      driver: 'antigravity',
      windows: [
        { id: 'gemini-5h', label: '5h', remainingPercent: 82 },
        { id: '3p-5h', label: '5h', remainingPercent: 8 },
      ],
    }),
  );
  const markup = renderToStaticMarkup(tree);
  assert.match(markup, /Gemini 5h/);
  assert.match(markup, /82%/);
  assert.match(markup, /is-high/);
  assert.match(markup, /Claude 5h/);
  assert.match(markup, /8%/);
  assert.match(markup, /is-critical/);
});
