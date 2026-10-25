import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
function harness(mode: string, options: { actions?: any[]; loaded?: boolean; account?: any; save?: () => Promise<any>; refresh?: () => Promise<any>; latestAccount?: any; accounts?: any[]; imageLoaded?: boolean; imageModel?: string; execute?: (launchAfterSwitch: boolean, options?: any) => Promise<boolean> } = {}) {
  let traps: any[] = [];
  let escapes: any[] = [];
  let closed = 0;
  const remembered: any[] = [];
  const remember = (event?: any) => { if (event?.currentTarget) remembered.push(event.currentTarget); };
  const t = (key: string) => key;
  const modalError = { message: null, scrollKey: 0, set() {} };
  const config = { experimental_model_catalog_available: true, experimental_model_catalog_enabled: false, experimental_model_catalog_models: [],
    experimental_model_catalog_default_model_id: null };
  const emptyAccounts: any[] = options.accounts ?? [];
  const emptyInstances: any[] = [];
  const instanceStore: any = (selector: any) => selector({ instances: emptyInstances });
  instanceStore.getState = () => ({ instances: emptyInstances });
  instanceStore.setState = () => {};
  const accountStore: any = (selector: any) => selector({ accounts: emptyAccounts, currentAccount: null, fetchAccounts: async () => {} });
  accountStore.getState = () => ({ accounts: [options.latestAccount ?? options.account].filter(Boolean), applyAccountSnapshot() {} });
  const h = loadHookModule(new URL('./CodexLaunchPreviewModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t, i18n: { language: 'en' } }) },
    '@tauri-apps/plugin-dialog': { confirm: async () => true },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean, restore = true, active = true) => { traps.push({ ref, open, restore, active }); } },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void, ref: any) => escapes.push({ open, close, ref }) },
    '../../services/codexInstanceService': { saveCodexInstanceQuickConfig: options.save ?? (async () => config),
      saveCodexInstanceConfiguration: async () => ({ quickConfig: config, instance: { id: '__default__' } }) },
    '../../services/codexService': { forceRefreshCodexTokens: options.refresh ?? (async () => options.account) },
    '../../services/codexLaunchPreviewConfigService': {
      getCachedCodexLaunchPreviewConfig: () => options.loaded ? config : null,
      loadCodexLaunchPreviewConfig: () => options.loaded ? Promise.resolve(config) : new Promise(() => {}),
      rememberCodexLaunchPreviewConfig() {},
      CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT: 'timeout',
    },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: accountStore },
    '../../stores/useCodexInstanceStore': { useCodexInstanceStore: instanceStore },
    './CodexModelRoutingFields': {
      CodexModelRoutingFields: 'routing', buildCodexModelRoutingValue: () => ({ enabled: false, routes: [] }),
      createCodexModelSourceResolver: () => () => null, normalizeCodexModelRoutingRoutes: () => [],
      eligibleCodexModelRoutingAccounts: () => [], syncExperimentalModelsWithRouting: (_routes: any, models: any[]) => models,
    },
    './useLaunchPreviewChildReturnFocus': { useLaunchPreviewChildReturnFocus: () => remember },
    './useCodexImageModelConfig': { useCodexImageModelConfig: () => ({ model: options.imageLoaded === false ? null : options.imageModel ?? 'gpt-image-2.5', control: null, loaded: options.imageLoaded !== false }) },
    './CodexImageAccountPickerModal': { CodexImageAccountPickerModal: 'image-picker' },
    './CodexQuotaMiniRows': { CodexQuotaMiniRows: 'quotas' },
    './CodexExperimentalModelEditor': { CodexExperimentalModelEditor: 'model-editor' },
    './CodexSessionVisibilityRepairModal': { CodexSessionVisibilityRepairModal: 'repair' },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'dropdown' },
    './CodexContextOverrideEditor': { CodexContextOverrideEditor: 'context-editor', resolveCodexContextOverridePreset: () => 'default' },
    './CodexSpeedControl': { CodexSpeedControl: 'speed' },
    '../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => modalError },
  }, { document: { querySelectorAll: () => [] }, performance: { now: () => 0 } });
  h.render(() => {
    escapes = []; traps = [];
    return h.exports.CodexLaunchPreviewModal({ mode, account: options.account, accountLabel: 'Synthetic account', summary: {}, actions: options.actions ?? [],
      onClose: () => { closed++; }, onExecute: options.execute ?? (async () => false) });
  });
  return { h, remembered, get tree() { return h.flush(); }, get trap() { const active = traps.filter((entry) => entry.open && entry.active); assert.equal(active.length, 1); return active[0]; }, get traps() { return traps; }, async ready() { await settlePromises(); h.flush(); }, get escapes() { for (const entry of escapes.filter((item) => item.open)) assert.equal(entry.ref, traps.find((trap) => trap.open && trap.active)?.ref); return escapes; }, get closed() { return closed; } };
}

for (const mode of ['account', 'instance', 'apiService']) {
  test(`${mode} launch previews register their actual root before configuration loads and preserve Escape`, () => {
    const h = harness(mode);
    const dialog = nodes(h.tree).find((node) => node.props?.className === 'modal codex-launch-preview-modal');
    assert.equal(dialog.props.ref, h.trap.ref);
    assert.equal(dialog.props.tabIndex, -1);
    assert.equal(dialog.props.role, 'dialog');
    assert.equal(dialog.props['aria-modal'], 'true');
    assert.equal(h.trap.open, true);
    const overlay = nodes(h.tree).find((node) => node.props?.className === 'modal-overlay codex-launch-preview-overlay');
    assert.equal(overlay.props.onClick, undefined);
    const active = h.escapes.filter((entry) => entry.open);
    assert.equal(active.length, 1);
    active[0].close();
    assert.equal(h.closed, 1);
    h.h.unmount();
  });
}


test('a busy launch preview keeps its focus trap while Escape and the header close remain disabled', () => {
  const h = harness('apiService');
  nodes(h.tree).find((node) => node.type === 'speed').props.onSavingChange(true);
  const tree = h.tree;
  assert.equal(h.trap.open, true);
  assert.equal(h.escapes.filter((entry) => entry.open).length, 0);
  const headerClose = nodes(tree).find((node) => node.props?.className === 'modal-close');
  assert.equal(headerClose.props.disabled, true);
  assert.equal(h.closed, 0);
  h.h.unmount();
});


for (const close of ['escape', 'button']) {
  test(`context child closes through ${close} without closing its parent or leaving the parent trap active`, async () => {
    const h = harness('apiService', { loaded: true }); await h.ready();
    const trigger = nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.contextOverride.adjust');
    assert.equal(trigger.props.disabled, false); const opener = { id: 'context-trigger' }; trigger.props.onClick({ currentTarget: opener });
    assert.equal(h.remembered[0], opener);
    const tree = h.tree;
    const child = nodes(tree).find((node) => node.props?.className?.includes('codex-launch-preview-context-config-modal'));
    assert.equal(child.props.ref, h.trap.ref);
    assert.equal(child.props.tabIndex, -1);
    const parent = nodes(tree).find((node) => node.props?.className === 'modal-overlay codex-launch-preview-overlay');
    assert.equal(parent.props.inert, true); assert.equal(parent.props['aria-hidden'], true);
    assert.equal(h.traps.find((entry) => entry.ref === nodes(parent).find((node) => node.props?.role === 'dialog').props.ref).active, false);
    if (close === 'escape') {
      const active = h.escapes.filter((entry) => entry.open); assert.equal(active.length, 1); active[0].close();
    } else nodes(child).find((node) => node.props?.className === 'modal-close').props.onClick();
    const next = h.tree;
    assert.equal(nodes(next).some((node) => node.props?.className?.includes('codex-launch-preview-context-config-modal')), false);
    assert.equal(h.closed, 0);
    const main = nodes(next).find((node) => node.props?.className === 'modal codex-launch-preview-modal');
    assert.equal(main.props.ref, h.trap.ref);
    assert.equal(nodes(next).find((node) => node.props?.className === parent.props.className).props.inert, false);
    h.h.unmount();
  });
}

test('model-management Escape cancels the child draft and preserves the preview', async () => {
  const h = harness('apiService', { loaded: true }); await h.ready();
  nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.modelManagement.enable').props.onClick({ currentTarget: 'model-trigger' });
  assert.equal(h.remembered[0], 'model-trigger');
  await h.ready();
  const child = nodes(h.tree).find((node) => node.props?.className === 'modal codex-launch-preview-model-config-modal');
  assert.equal(child.props.ref, h.trap.ref);
  const active = h.escapes.filter((entry) => entry.open); assert.equal(active.length, 1); active[0].close();
  assert.equal(nodes(h.tree).some((node) => node.props?.className === child.props.className), false);
  assert.equal(h.closed, 0);
  assert.ok(nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.modelManagement.enable'));
  h.h.unmount();
});

test('applying context keeps the child open and disables keyboard and explicit close while saving', async () => {
  const save = deferred<any>();
  const h = harness('apiService', { loaded: true, save: () => save.promise }); await h.ready();
  nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.contextOverride.adjust').props.onClick();
  const child = nodes(h.tree).find((node) => node.props?.className?.includes('codex-launch-preview-context-config-modal'));
  nodes(child).find((node) => node.type === 'context-editor').props.onChange({ enabled: true, contextWindow: '200000', compactLimit: '180000' });
  nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.contextOverride.apply').props.onClick();
  await h.ready();
  const savingChild = nodes(h.tree).find((node) => node.props?.className?.includes('codex-launch-preview-context-config-modal'));
  assert.equal(savingChild.props.ref, h.trap.ref);
  assert.equal(nodes(savingChild).find((node) => node.props?.className === 'modal-close').props.disabled, true);
  assert.equal(h.escapes.filter((entry) => entry.open).length, 0);
  assert.equal(h.closed, 0);
  h.h.unmount();
});

test('image-account selection suspends the preview and cancel restores its active root', async () => {
  const account = { id: 'qa', email: 'qa@example.invalid', auth_mode: 'apikey', api_provider_id: 'deepseek',
    api_protocol: 'responses', openai_api_key: 'QA_DUMMY' };
  const h = harness('account', { loaded: true, account }); await h.ready();
  const row = nodes(h.tree).find((node) => node.props?.className === 'codex-launch-preview-tool-controls'
    && nodes(node).some((child) => child.type === 'button' && child.props.children === 'codex.deepSeek.start.imageGenPick'));
  nodes(row).find((node) => node.type === 'input' && node.props.type === 'checkbox').props.onChange({ target: { checked: true }, currentTarget: 'image-checkbox-trigger' });
  assert.equal(h.remembered[0], 'image-checkbox-trigger');
  const tree = h.tree;
  const picker = nodes(tree).find((node) => node.type === 'image-picker');
  assert.ok(picker);
  const parent = nodes(tree).find((node) => node.props?.className === 'modal-overlay codex-launch-preview-overlay');
  assert.equal(parent.props.inert, true);
  assert.equal(h.traps.filter((entry) => entry.open && entry.active).length, 0); // Picker's own module owns the new trap.
  picker.props.onCancel();
  assert.equal(nodes(h.tree).some((node) => node.type === 'image-picker'), false);
  assert.equal(h.trap.ref, nodes(h.tree).find((node) => node.props?.className === 'modal codex-launch-preview-modal').props.ref);
  assert.equal(h.closed, 0);
  h.h.unmount();
});

test('opening session repair suspends the preview and its close leaves the parent intact', async () => {
  const h = harness('apiService', { loaded: true }); await h.ready();
  nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.sessionManager.actions.repairVisibility').props.onClick({ currentTarget: 'repair-trigger' });
  assert.equal(h.remembered[0], 'repair-trigger');
  const repair = nodes(h.tree).find((node) => node.type === 'repair');
  assert.equal(repair.props.open, true);
  assert.equal(repair.props.repairKind, 'visibility');
  assert.equal(h.traps.filter((entry) => entry.open && entry.active).length, 0);
  repair.props.onClose();
  assert.equal(nodes(h.tree).find((node) => node.type === 'repair').props.open, false);
  assert.equal(h.trap.open, true);
  assert.equal(h.closed, 0);
  h.h.unmount();
});

for (const mode of ['account', 'instance', 'apiService']) {
  test(`${mode} preview exposes a separate index-repair entry and reuses one dialog`, async () => {
    const h = harness(mode, { loaded: true }); await h.ready();
    const indexTrigger = nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.sessionManager.actions.repairIndex');
    assert.ok(indexTrigger);
    assert.equal(indexTrigger.props.disabled, false);
    indexTrigger.props.onClick({ currentTarget: 'index-repair-trigger' });
    assert.equal(h.remembered[0], 'index-repair-trigger');
    let repairs = nodes(h.tree).filter((node) => node.type === 'repair');
    assert.equal(repairs.length, 1);
    assert.equal(repairs[0].props.open, true);
    assert.equal(repairs[0].props.repairKind, 'history');
    assert.equal(h.traps.filter((entry) => entry.open && entry.active).length, 0);
    repairs[0].props.onRepaired({ message: 'Index reset; reopen the chat to rebuild.' });
    assert.ok(nodes(h.tree).some((node) => node.type === 'span' && node.props.children === 'Index reset; reopen the chat to rebuild.'));
    repairs[0].props.onClose();
    h.h.flush();
    assert.equal(h.trap.open, true);
    nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.sessionManager.actions.repairVisibility').props.onClick({ currentTarget: 'visibility-repair-trigger' });
    repairs = nodes(h.tree).filter((node) => node.type === 'repair');
    assert.equal(repairs.length, 1);
    assert.equal(repairs[0].props.repairKind, 'visibility');
    repairs[0].props.onClose();
    assert.equal(h.closed, 0);
    h.h.unmount();
  });
}


for (const childKind of ['access-mode', 'image-mode']) {
  test(`${childKind} child owns focus and Escape without closing the launch preview`, async () => {
    const account = { id: 'qa', email: 'qa@example.invalid', auth_mode: 'apikey', api_provider_id: 'deepseek',
      api_instance_access_mode: 'direct', openai_api_key: 'QA_DUMMY' };
    const h = harness('account', { loaded: true, account }); await h.ready();
    if (childKind === 'access-mode') {
      nodes(h.tree).find((node) => node.props?.className?.includes('codex-launch-preview-dropdown-trigger')).props.onClick({ currentTarget: 'access-mode-trigger' });
      assert.equal(h.remembered[0], 'access-mode-trigger');
    } else {
      const row = nodes(h.tree).find((node) => node.props?.className === 'codex-launch-preview-tool-controls'
        && nodes(node).some((child) => child.type === 'button' && child.props.children === 'codex.deepSeek.start.imageGenPick'));
      nodes(row).find((node) => node.type === 'input' && node.props.type === 'checkbox').props.onChange({ target: { checked: true }, currentTarget: 'image-mode-trigger' });
      assert.equal(h.remembered[0], 'image-mode-trigger');
    }
    const child = nodes(h.tree).find((node) => node.props?.role === 'dialog' && node.props.ref === h.trap.ref);
    assert.equal(child.props.tabIndex, -1);
    assert.notEqual(child.props.className, 'modal codex-launch-preview-modal');
    assert.equal(nodes(h.tree).find((node) => node.props?.className === 'modal-overlay codex-launch-preview-overlay').props.inert, true);
    const active = h.escapes.filter((entry) => entry.open); assert.equal(active.length, 1); active[0].close();
    assert.equal(nodes(h.tree).find((node) => node.props?.className === 'modal codex-launch-preview-modal').props.ref, h.trap.ref);
    assert.equal(h.closed, 0);
    h.h.unmount();
  });
}

test('token-refresh feedback owns the trap while running and Escape closes only finished feedback', async () => {
  const refresh = deferred<any>();
  const account = { id: 'qa', email: 'qa@example.invalid', auth_mode: 'oauth', plan_type: 'pro', tokens: { access_token: 'QA_ACCESS_NOT_VALID', refresh_token: 'QA_REFRESH_NOT_VALID' } };
  const h = harness('account', { loaded: true, account, refresh: () => refresh.promise }); await h.ready();
  const trigger = nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.launchPreview.forceRefreshAction');
  assert.ok(trigger); trigger.props.onClick({ currentTarget: 'refresh-trigger' });
  assert.equal(h.remembered[0], 'refresh-trigger');
  const child = nodes(h.tree).find((node) => node.props?.['aria-labelledby'] === 'codex-launch-preview-refresh-title');
  assert.equal(child.props.ref, h.trap.ref);
  assert.equal(h.escapes.filter((entry) => entry.open).length, 0);
  refresh.reject(Error('synthetic refresh failure')); await h.ready();
  const active = h.escapes.filter((entry) => entry.open); assert.equal(active.length, 1); active[0].close();
  assert.equal(nodes(h.tree).some((node) => node.props?.['aria-labelledby'] === 'codex-launch-preview-refresh-title'), false);
  assert.equal(h.closed, 0);
  h.h.unmount();
});


function healthyRefreshAccount() {
  const encode = (value: object) => Buffer.from(JSON.stringify(value)).toString('base64url');
  const token = `${encode({ alg: 'none' })}.${encode({ exp: Math.floor(Date.now() / 1000) + 6 * 86400 })}.QA_SIGNATURE`;
  return { id: 'qa', email: 'qa@example.invalid', auth_mode: 'oauth', plan_type: 'pro', requires_reauth: false,
    tokens: { access_token: token, refresh_token: 'QA_REFRESH_NOT_VALID' } };
}
for (const [reason, reauth] of [
  ['Token 刷新失败：Token 刷新请求失败: error sending request for url (https://auth.openai.com/oauth/token)', false],
  ['TLS certificate expired during refresh', false],
  ['PROXY_AUTH_FAILED: refresh status=401 Unauthorized', false],
  ['refresh status=403 unsupported_country_region_territory', false],
  ['refresh status=503 upstream unavailable', false],
  ['refresh response JSON parse failed', false],
  ['Token refresh failed: error_code=refresh_token_reused', false],
  ['Token-Aktualisierung fehlgeschlagen: error_code=refresh_token_expired', true],
  ['Codex 登录授权已被服务端撤销，原始错误: error_code=refresh_token_invalidated', true],
  ['OAuth refresh: error_code=token_revoked', true],
  ['OAuth refresh: error_code=token_invalidated', true],
  ['Codex 登录授权无效，原始错误: status=401 Unauthorized', true],
] as const) {
  test(`refresh result requires reauthorization=${reauth} for ${reason}`, async () => {
    const account = healthyRefreshAccount();
    const h = harness('account', { loaded: true, account, refresh: async () => { throw Error(reason); } }); await h.ready();
    nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.launchPreview.forceRefreshAction').props.onClick();
    await h.ready();
    const child = nodes(h.tree).find((node) => node.props?.['aria-labelledby'] === 'codex-launch-preview-refresh-title');
    const buttons = nodes(child).filter((node) => node.type === 'button');
    assert.equal(buttons.some((node) => node.props.children === 'common.reauthorize'), reauth);
    assert.ok(buttons.some((node) => node.props.children === 'codex.launchPreview.forceRefreshRetry' && node.props.disabled === false));
    assert.equal(account.requires_reauth, false);
    assert.equal(h.closed, 0);
    h.h.unmount();
  });
}

test('refresh result respects a confirmed latest account authorization state without inferring from token dates', async () => {
  const account = healthyRefreshAccount();
  const latestAccount = { ...account, requires_reauth: true, reauth_reason: 'Confirmed backend authorization failure' };
  const h = harness('account', { loaded: true, account, latestAccount, refresh: async () => { throw Error('neutral read failure'); } }); await h.ready();
  nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.launchPreview.forceRefreshAction').props.onClick();
  await h.ready();
  const child = nodes(h.tree).find((node) => node.props?.['aria-labelledby'] === 'codex-launch-preview-refresh-title');
  assert.ok(nodes(child).find((node) => node.type === 'button' && node.props.children === 'common.reauthorize'));
  h.h.unmount();
});


test('an expired identity token alone does not turn a transport failure into a reauthorization result', async () => {
  const account = healthyRefreshAccount();
  (account.tokens as any).id_token = `e30.${Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) - 3600 })).toString('base64url')}.QA_SIGNATURE`;
  const h = harness('account', { loaded: true, account, refresh: async () => { throw Error('TLS certificate expired during refresh'); } }); await h.ready();
  nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.launchPreview.forceRefreshAction').props.onClick();
  await h.ready();
  const child = nodes(h.tree).find((node) => node.props?.['aria-labelledby'] === 'codex-launch-preview-refresh-title');
  assert.equal(nodes(child).some((node) => node.type === 'button' && node.props.children === 'common.reauthorize'), false);
  h.h.unmount();
});


test('the explicit image-account picker button records its own return target before opening the child', async () => {
  const account = { id: 'qa', email: 'qa@example.invalid', auth_mode: 'apikey', api_provider_id: 'deepseek',
    api_image_generation_account_ids: ['qa-image'], openai_api_key: 'QA_DUMMY' };
  const h = harness('account', { loaded: true, account }); await h.ready();
  const trigger = nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'codex.deepSeek.start.imageGenPick');
  assert.equal(trigger.props.disabled, false);
  trigger.props.onClick({ currentTarget: 'picker-button-trigger' });
  assert.equal(h.remembered[0], 'picker-button-trigger');
  assert.ok(nodes(h.tree).find((node) => node.type === 'image-picker'));
  h.h.unmount();
});


test('launch does not silently discard a selected API image source while image settings are unavailable', async () => {
  let executed = 0;
  const apiSource = { id: 'image-api', auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5'] };
  const account = { id: 'deepseek', email: 'DeepSeek', auth_mode: 'apikey', api_provider_id: 'deepseek',
    api_instance_access_mode: 'gateway', api_image_generation_account_ids: ['image-api'], api_image_generation_account_models: { 'image-api': 'gpt-image-2.5-high' } };
  const { h, ready } = harness('account', { account, loaded: true, accounts: [apiSource], imageLoaded: false,
    execute: async () => { executed++; return true; } });
  await ready();
  const action = nodes(h.flush()).find((node) => node.type === 'button' && [node.props?.children].flat(Infinity).includes('codex.launchPreview.switchAndStart'));
  assert.ok(action);
  await action.props.onClick(); await settlePromises();
  assert.equal(executed, 0);
  const picker = nodes(h.flush()).find((node) => node.type === 'image-picker');
  assert.ok(picker);
  assert.deepEqual(picker.props.selectedIds, ['image-api']);
  assert.deepEqual(picker.props.selectedModels, { 'image-api': 'gpt-image-2.5-high' });
  assert.equal(picker.props.imageGenerationModel, null);
  h.unmount();
});


test('launch forwards the saved full image variant together with source IDs', async () => {
  let launchOptions: any;
  const source = { id: 'source-api', email: 'Image source', auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5-high'] };
  const account = { id: 'deepseek-chat', email: 'DeepSeek', auth_mode: 'apikey', api_provider_id: 'deepseek', api_instance_access_mode: 'gateway',
    api_image_generation_account_ids: ['source-api'], api_image_generation_account_models: { 'source-api': 'gpt-image-2.5-high' } };
  const { h, ready } = harness('account', { account, loaded: true, accounts: [source], imageModel: 'gpt-image-2.5-high',
    execute: async (_start, options) => { launchOptions = options; return true; } });
  await ready();
  const action = nodes(h.flush()).find((node) => node.type === 'button' && [node.props?.children].flat(Infinity).includes('codex.launchPreview.switchAndStart'));
  assert.ok(action); action.props.onClick(); await settlePromises(); h.flush();
  assert.deepEqual(launchOptions.imageGenerationAccountIds, ['source-api']);
  assert.deepEqual(launchOptions.imageGenerationAccountModels, { 'source-api': 'gpt-image-2.5-high' });
  h.unmount();
});


for (const mode of ['account', 'instance', 'apiService']) {
  test(`${mode} rendered CLI action opens CLI once and closes the preview`, async () => {
    let opened = 0;
    let desktopStarts = 0;
    const h = harness(mode, { loaded: true,
      actions: [{ id: 'cli', label: 'CLI', onAction: () => { opened++; } }],
      execute: async () => { desktopStarts++; return true; },
    });
    await h.ready();
    const button = nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'CLI');
    assert.equal(button.props.disabled, false);
    button.props.onClick();
    await h.ready();
    assert.equal(opened, 1);
    assert.equal(h.closed, 1);
    assert.equal(desktopStarts, 0);
    h.h.unmount();
  });
}

test('rendered CLI action is disabled while preview configuration is loading', () => {
  const h = harness('account', { actions: [{ id: 'cli', label: 'CLI', onAction() {} }] });
  const button = nodes(h.tree).find((node) => node.type === 'button' && node.props.children === 'CLI');
  assert.equal(button.props.disabled, true);
  h.h.unmount();
});
